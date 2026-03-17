use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

pub const PAGE_SIZE: usize = 4096;
const MAX_PAGES: usize = 1024;

pub struct Pager {
    file: File,
    file_length: u64,
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

        Ok(Pager {
            file,
            file_length,
            pages: vec![None; MAX_PAGES],
            dirty: vec![false; MAX_PAGES],
        })
    }

    pub fn num_pages(&self) -> u32 {
        (self.file_length / PAGE_SIZE as u64) as u32
    }

    pub fn get_page(&mut self, page_num: u32) -> Result<&[u8; PAGE_SIZE], String> {
        let idx = page_num as usize;
        if idx >= MAX_PAGES {
            return Err(format!("Page {} exceeds max {}", page_num, MAX_PAGES));
        }
        if self.pages[idx].is_none() {
            let mut buf = [0u8; PAGE_SIZE];
            if page_num < self.num_pages() {
                self.file
                    .seek(SeekFrom::Start(page_num as u64 * PAGE_SIZE as u64))
                    .map_err(|e| e.to_string())?;
                self.file.read_exact(&mut buf).map_err(|e| e.to_string())?;
            }
            self.pages[idx] = Some(buf);
        }
        Ok(self.pages[idx].as_ref().unwrap())
    }

    pub fn get_page_mut(&mut self, page_num: u32) -> Result<&mut [u8; PAGE_SIZE], String> {
        self.get_page(page_num)?;
        let idx = page_num as usize;
        self.dirty[idx] = true;
        let new_end = (page_num as u64 + 1) * PAGE_SIZE as u64;
        if new_end > self.file_length {
            self.file_length = new_end;
        }
        Ok(self.pages[idx].as_mut().unwrap())
    }

    pub fn allocate_page(&mut self) -> Result<u32, String> {
        let page_num = self.num_pages();
        self.get_page_mut(page_num)?;
        Ok(page_num)
    }

    pub fn flush(&mut self) -> Result<(), String> {
        for i in 0..MAX_PAGES {
            if self.dirty[i] {
                if let Some(ref buf) = self.pages[i] {
                    self.file
                        .seek(SeekFrom::Start(i as u64 * PAGE_SIZE as u64))
                        .map_err(|e| e.to_string())?;
                    self.file.write_all(buf).map_err(|e| e.to_string())?;
                    self.dirty[i] = false;
                }
            }
        }
        self.file.flush().map_err(|e| e.to_string())
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
}
