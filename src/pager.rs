use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::wal::Wal;

pub const PAGE_SIZE: usize = 4096;
const MAX_PAGES: usize = 1024;

/// Page cache + on-disk file, accessed by both the read path (via interior
/// mutability) and the write path (via `&mut self`).
///
/// # Concurrency model
///
/// The Pager supports two access modes:
///
/// - **Read (`&self`)**: `read_page` acquires a short lock on `cache` to
///   look up / populate the page cache, and on `file` to read from disk on
///   cache miss. Returns an owned `[u8; PAGE_SIZE]` copy so the caller does
///   not hold the lock.
///
/// - **Write (`&mut self`)**: `get_page` and `get_page_mut` bypass the mutex
///   via `Mutex::get_mut()` since exclusive access is guaranteed by the
///   caller. They return borrowed references as before.
///
/// The higher layers (Session → RwLock<Storage>) ensure read/write paths
/// are never used simultaneously on the same Pager.
pub struct Pager {
    file: Mutex<File>,
    file_length: AtomicU64,
    cache: Mutex<Cache>,
    wal: Wal,
    in_transaction: bool,
}

struct Cache {
    pages: Vec<Option<[u8; PAGE_SIZE]>>,
    dirty: Vec<bool>,
}

impl Pager {
    pub fn open(path: &str) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .map_err(|e| format!("Failed to open '{}': {}", path, e))?;

        let file_length = file.metadata().map_err(|e| e.to_string())?.len();

        if file_length % PAGE_SIZE as u64 != 0 {
            return Err(format!(
                "Corrupt file '{}': length {} not a multiple of {}",
                path, file_length, PAGE_SIZE
            ));
        }

        let mut wal = Wal::open(path)?;

        // Crash recovery: replay any committed WAL records.
        let records = wal.recover()?;
        if !records.is_empty() {
            let mut f = OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            for rec in &records {
                f.seek(SeekFrom::Start(rec.page_num as u64 * PAGE_SIZE as u64))
                    .map_err(|e| e.to_string())?;
                f.write_all(&rec.data).map_err(|e| e.to_string())?;
            }
            f.sync_all().map_err(|e| e.to_string())?;
            wal.truncate()?;
        }

        // Re-read file length after potential recovery.
        let file_length = file.metadata().map_err(|e| e.to_string())?.len();

        Ok(Pager {
            file: Mutex::new(file),
            file_length: AtomicU64::new(file_length),
            cache: Mutex::new(Cache {
                pages: vec![None; MAX_PAGES],
                dirty: vec![false; MAX_PAGES],
            }),
            wal,
            in_transaction: false,
        })
    }

    pub fn num_pages(&self) -> u32 {
        (self.file_length.load(Ordering::Acquire) / PAGE_SIZE as u64) as u32
    }

    pub fn in_transaction(&self) -> bool {
        self.in_transaction
    }

    // -------------------------------------------------------------------
    // Read path — &self, interior-mutable.
    // -------------------------------------------------------------------

    /// Read a page by number. Returns an owned 4KB copy.
    ///
    /// Safe to call concurrently from multiple threads holding `&Pager`.
    pub fn read_page(&self, page_num: u32) -> Result<[u8; PAGE_SIZE], String> {
        let idx = page_num as usize;
        if idx >= MAX_PAGES {
            return Err(format!("Page {} exceeds max {}", page_num, MAX_PAGES));
        }

        // Fast path: cache hit.
        {
            let cache = self.cache.lock().expect("cache mutex poisoned");
            if let Some(buf) = cache.pages[idx] {
                return Ok(buf);
            }
        }

        // Cache miss: read from disk (holding only the file lock briefly),
        // then re-lock cache to insert. A concurrent reader may do the same
        // work; both will produce the same bytes.
        let mut buf = [0u8; PAGE_SIZE];
        let file_len = self.file_length.load(Ordering::Acquire);
        let page_end = (page_num as u64 + 1) * PAGE_SIZE as u64;
        if page_end <= file_len {
            let mut file = self.file.lock().expect("file mutex poisoned");
            file.seek(SeekFrom::Start(page_num as u64 * PAGE_SIZE as u64))
                .map_err(|e| e.to_string())?;
            file.read_exact(&mut buf).map_err(|e| e.to_string())?;
        }

        let mut cache = self.cache.lock().expect("cache mutex poisoned");
        cache.pages[idx] = Some(buf);
        Ok(buf)
    }

    // -------------------------------------------------------------------
    // Write path — &mut self, exclusive access.
    // -------------------------------------------------------------------

    pub fn get_page(&mut self, page_num: u32) -> Result<&[u8; PAGE_SIZE], String> {
        let idx = page_num as usize;
        if idx >= MAX_PAGES {
            return Err(format!("Page {} exceeds max {}", page_num, MAX_PAGES));
        }

        // Populate the cache if this page isn't loaded.
        let cache = self.cache.get_mut().expect("cache mutex poisoned");
        if cache.pages[idx].is_none() {
            let mut buf = [0u8; PAGE_SIZE];
            let file_len = self.file_length.load(Ordering::Acquire);
            let page_end = (page_num as u64 + 1) * PAGE_SIZE as u64;
            if page_end <= file_len {
                let file = self.file.get_mut().expect("file mutex poisoned");
                file.seek(SeekFrom::Start(page_num as u64 * PAGE_SIZE as u64))
                    .map_err(|e| e.to_string())?;
                file.read_exact(&mut buf).map_err(|e| e.to_string())?;
            }
            cache.pages[idx] = Some(buf);
        }
        Ok(cache.pages[idx].as_ref().unwrap())
    }

    pub fn get_page_mut(&mut self, page_num: u32) -> Result<&mut [u8; PAGE_SIZE], String> {
        self.get_page(page_num)?;
        let idx = page_num as usize;
        let new_end = (page_num as u64 + 1) * PAGE_SIZE as u64;
        let cur_len = self.file_length.load(Ordering::Acquire);
        if new_end > cur_len {
            self.file_length.store(new_end, Ordering::Release);
        }
        let cache = self.cache.get_mut().expect("cache mutex poisoned");
        cache.dirty[idx] = true;
        Ok(cache.pages[idx].as_mut().unwrap())
    }

    pub fn allocate_page(&mut self) -> Result<u32, String> {
        let page_num = self.num_pages();
        self.get_page_mut(page_num)?;
        Ok(page_num)
    }

    /// Begin an explicit transaction.
    pub fn begin(&mut self) -> Result<(), String> {
        if self.in_transaction {
            return Err("Already in a transaction.".to_string());
        }
        self.in_transaction = true;
        Ok(())
    }

    /// Commit: WAL-write dirty pages → fsync WAL → apply to .db → fsync .db → truncate WAL.
    pub fn commit(&mut self) -> Result<(), String> {
        let cache = self.cache.get_mut().expect("cache mutex poisoned");
        // Write dirty pages to WAL.
        for i in 0..MAX_PAGES {
            if cache.dirty[i] {
                if let Some(ref buf) = cache.pages[i] {
                    self.wal.append_page(i as u32, buf)?;
                }
            }
        }
        self.wal.append_commit()?;

        // Apply dirty pages to the .db file.
        let file = self.file.get_mut().expect("file mutex poisoned");
        for i in 0..MAX_PAGES {
            if cache.dirty[i] {
                if let Some(ref buf) = cache.pages[i] {
                    file.seek(SeekFrom::Start(i as u64 * PAGE_SIZE as u64))
                        .map_err(|e| e.to_string())?;
                    file.write_all(buf).map_err(|e| e.to_string())?;
                    cache.dirty[i] = false;
                }
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;

        self.wal.truncate()?;
        self.in_transaction = false;
        Ok(())
    }

    /// Rollback: discard dirty pages (reload from disk on next access).
    pub fn rollback(&mut self) -> Result<(), String> {
        let cache = self.cache.get_mut().expect("cache mutex poisoned");
        for i in 0..MAX_PAGES {
            if cache.dirty[i] {
                cache.pages[i] = None;
                cache.dirty[i] = false;
            }
        }
        // Recalculate file_length from actual file.
        let file = self.file.get_mut().expect("file mutex poisoned");
        let fl = file.metadata().map_err(|e| e.to_string())?.len();
        self.file_length.store(fl, Ordering::Release);
        self.wal.truncate()?;
        self.in_transaction = false;
        Ok(())
    }

    /// Flush — for callers that aren't using explicit transactions (auto-commit).
    pub fn flush(&mut self) -> Result<(), String> {
        self.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn round_trip_page() {
        let path = "/tmp/mukhidb_pager_test.db";
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(format!("{}.wal", path));

        {
            let mut p = Pager::open(path).unwrap();
            assert_eq!(p.num_pages(), 0);
            let page = p.get_page_mut(0).unwrap();
            page[0] = 0xAB;
            page[4095] = 0xCD;
            p.flush().unwrap();
        }
        {
            let mut p = Pager::open(path).unwrap();
            assert_eq!(p.num_pages(), 1);
            let page = p.get_page(0).unwrap();
            assert_eq!(page[0], 0xAB);
            assert_eq!(page[4095], 0xCD);
        }

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rollback_discards_changes() {
        let path = "/tmp/mukhidb_pager_rollback.db";
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(format!("{}.wal", path));

        let mut p = Pager::open(path).unwrap();
        let page = p.get_page_mut(0).unwrap();
        page[0] = 0xFF;
        p.flush().unwrap();

        p.begin().unwrap();
        let page = p.get_page_mut(0).unwrap();
        page[0] = 0x00;
        p.rollback().unwrap();

        let page = p.get_page(0).unwrap();
        assert_eq!(page[0], 0xFF);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_page_works_via_shared_ref() {
        let path = "/tmp/mukhidb_pager_read_page.db";
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(format!("{}.wal", path));

        let mut p = Pager::open(path).unwrap();
        let page = p.get_page_mut(0).unwrap();
        page[0] = 0x11;
        page[1] = 0x22;
        p.flush().unwrap();

        let pref: &Pager = &p;
        let copy = pref.read_page(0).unwrap();
        assert_eq!(copy[0], 0x11);
        assert_eq!(copy[1], 0x22);

        fs::remove_file(path).unwrap();
    }
}
