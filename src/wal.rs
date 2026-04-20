/// Write-Ahead Log (WAL) for crash-safe page writes.
///
/// Record layout (fixed 4109 bytes):
///   [0]        record_type  (0x01 = page_write, 0x02 = commit)
///   [1..9]     txn_id       (u64 LE)
///   [9..13]    page_num     (u32 LE)
///   [13..4109] page_data    (4096 bytes)
///
/// For commit records, page_num and page_data are zeroed.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use crate::pager::PAGE_SIZE;

const RECORD_TYPE_PAGE: u8 = 0x01;
const RECORD_TYPE_COMMIT: u8 = 0x02;
const RECORD_SIZE: usize = 1 + 8 + 4 + PAGE_SIZE; // 4109

pub struct Wal {
    file: File,
    path: String,
    pub txn_id: u64,
}

/// A single recovered page write from the WAL.
pub struct WalRecord {
    pub page_num: u32,
    pub data: [u8; PAGE_SIZE],
}

impl Wal {
    pub fn open(db_path: &str) -> Result<Self, String> {
        let path = format!("{}.wal", db_path);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .map_err(|e| format!("WAL open '{}': {}", path, e))?;
        Ok(Wal { file, path, txn_id: 0 })
    }

    /// Append a page-write record to the WAL.
    pub fn append_page(&mut self, page_num: u32, data: &[u8; PAGE_SIZE]) -> Result<(), String> {
        let mut rec = [0u8; RECORD_SIZE];
        rec[0] = RECORD_TYPE_PAGE;
        rec[1..9].copy_from_slice(&self.txn_id.to_le_bytes());
        rec[9..13].copy_from_slice(&page_num.to_le_bytes());
        rec[13..].copy_from_slice(data);
        self.file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        self.file.write_all(&rec).map_err(|e| e.to_string())
    }

    /// Append a commit marker and fsync the WAL.
    pub fn append_commit(&mut self) -> Result<(), String> {
        let mut rec = [0u8; RECORD_SIZE];
        rec[0] = RECORD_TYPE_COMMIT;
        rec[1..9].copy_from_slice(&self.txn_id.to_le_bytes());
        self.file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        self.file.write_all(&rec).map_err(|e| e.to_string())?;
        self.file.sync_all().map_err(|e| e.to_string())?;
        self.txn_id += 1;
        Ok(())
    }

    /// Truncate the WAL to zero bytes.
    pub fn truncate(&mut self) -> Result<(), String> {
        self.file.set_len(0).map_err(|e| e.to_string())?;
        self.file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Recover committed page writes from the WAL.
    /// Returns only records belonging to transactions that have a commit marker.
    pub fn recover(&mut self) -> Result<Vec<WalRecord>, String> {
        let len = self.file.metadata().map_err(|e| e.to_string())?.len();
        if len == 0 {
            return Ok(Vec::new());
        }
        self.file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;

        // First pass: find committed txn_ids and collect page records.
        let mut committed = std::collections::HashSet::new();
        let mut pages: Vec<(u64, WalRecord)> = Vec::new();
        let mut buf = [0u8; RECORD_SIZE];

        loop {
            match self.file.read_exact(&mut buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.to_string()),
            }
            let rtype = buf[0];
            let txn = u64::from_le_bytes(buf[1..9].try_into().unwrap());
            match rtype {
                RECORD_TYPE_COMMIT => { committed.insert(txn); }
                RECORD_TYPE_PAGE => {
                    let page_num = u32::from_le_bytes(buf[9..13].try_into().unwrap());
                    let mut data = [0u8; PAGE_SIZE];
                    data.copy_from_slice(&buf[13..]);
                    pages.push((txn, WalRecord { page_num, data }));
                }
                _ => {} // skip unknown
            }
        }

        // Keep only records from committed transactions.
        Ok(pages.into_iter().filter(|(txn, _)| committed.contains(txn)).map(|(_, r)| r).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_wal(name: &str) -> (String, Wal) {
        let db_path = format!("/tmp/mukhidb_wal_{}.db", name);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(format!("{}.wal", db_path));
        // Create the .db file so the WAL path is valid context
        File::create(&db_path).unwrap();
        let wal = Wal::open(&db_path).unwrap();
        (db_path, wal)
    }

    #[test]
    fn append_and_recover_committed() {
        let (db_path, mut wal) = test_wal("recover");
        let mut page = [0u8; PAGE_SIZE];
        page[0] = 0xAB;
        wal.append_page(5, &page).unwrap();
        wal.append_commit().unwrap();

        // Recover should return the committed record.
        let mut wal2 = Wal::open(&db_path).unwrap();
        let records = wal2.recover().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].page_num, 5);
        assert_eq!(records[0].data[0], 0xAB);

        fs::remove_file(&db_path).unwrap();
        let _ = fs::remove_file(format!("{}.wal", db_path));
    }

    #[test]
    fn uncommitted_records_discarded() {
        let (db_path, mut wal) = test_wal("uncommitted");
        let page = [0u8; PAGE_SIZE];
        wal.append_page(1, &page).unwrap();
        // No commit marker written.

        let mut wal2 = Wal::open(&db_path).unwrap();
        let records = wal2.recover().unwrap();
        assert!(records.is_empty());

        fs::remove_file(&db_path).unwrap();
        let _ = fs::remove_file(format!("{}.wal", db_path));
    }

    #[test]
    fn truncate_clears_wal() {
        let (db_path, mut wal) = test_wal("truncate");
        let page = [0u8; PAGE_SIZE];
        wal.append_page(0, &page).unwrap();
        wal.append_commit().unwrap();
        wal.truncate().unwrap();

        let mut wal2 = Wal::open(&db_path).unwrap();
        let records = wal2.recover().unwrap();
        assert!(records.is_empty());

        fs::remove_file(&db_path).unwrap();
        let _ = fs::remove_file(format!("{}.wal", db_path));
    }

    #[test]
    fn multiple_txns_only_committed_recovered() {
        let (db_path, mut wal) = test_wal("multi_txn");
        // Txn 0: committed
        let mut p1 = [0u8; PAGE_SIZE];
        p1[0] = 0x01;
        wal.append_page(0, &p1).unwrap();
        wal.append_commit().unwrap(); // txn_id becomes 1

        // Txn 1: NOT committed
        let mut p2 = [0u8; PAGE_SIZE];
        p2[0] = 0x02;
        wal.append_page(1, &p2).unwrap();
        // no commit

        let mut wal2 = Wal::open(&db_path).unwrap();
        let records = wal2.recover().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].data[0], 0x01);

        fs::remove_file(&db_path).unwrap();
        let _ = fs::remove_file(format!("{}.wal", db_path));
    }
}

impl Drop for Wal {
    fn drop(&mut self) {
        // Clean up empty WAL files.
        if let Ok(meta) = std::fs::metadata(&self.path) {
            if meta.len() == 0 {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}
