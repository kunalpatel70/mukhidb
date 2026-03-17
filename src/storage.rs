use std::collections::HashMap;
use crate::btree;
use crate::pager::Pager;
use crate::row;
use crate::types::{Column, DataType, Row, Value};

/// Metadata page (page 0) layout:
///   [0..4]    root_page  (u32 LE)
///   [4..8]    num_columns (u32 LE)
///   [8..]     columns: each is [name_len: u16 LE][name bytes][type: u8 (0=int,1=text)]
const META_ROOT_OFFSET: usize = 0;
const META_NCOLS_OFFSET: usize = 4;
const META_COLS_OFFSET: usize = 8;

fn write_metadata(pager: &mut Pager, root_page: u32, columns: &[Column]) -> Result<(), String> {
    let page = pager.get_page_mut(0)?;
    page[META_ROOT_OFFSET..META_ROOT_OFFSET + 4].copy_from_slice(&root_page.to_le_bytes());
    page[META_NCOLS_OFFSET..META_NCOLS_OFFSET + 4]
        .copy_from_slice(&(columns.len() as u32).to_le_bytes());
    let mut off = META_COLS_OFFSET;
    for col in columns {
        let name = col.name.as_bytes();
        let len = name.len().min(255) as u16;
        page[off..off + 2].copy_from_slice(&len.to_le_bytes());
        off += 2;
        page[off..off + len as usize].copy_from_slice(&name[..len as usize]);
        off += len as usize;
        page[off] = match col.data_type {
            DataType::Integer => 0,
            DataType::Text => 1,
        };
        off += 1;
    }
    Ok(())
}

fn read_metadata(pager: &mut Pager) -> Result<(u32, Vec<Column>), String> {
    let page = pager.get_page(0)?;
    let root = u32::from_le_bytes(page[META_ROOT_OFFSET..META_ROOT_OFFSET + 4].try_into().unwrap());
    let ncols =
        u32::from_le_bytes(page[META_NCOLS_OFFSET..META_NCOLS_OFFSET + 4].try_into().unwrap())
            as usize;
    let mut columns = Vec::with_capacity(ncols);
    let mut off = META_COLS_OFFSET;
    for _ in 0..ncols {
        let len = u16::from_le_bytes(page[off..off + 2].try_into().unwrap()) as usize;
        off += 2;
        let name = String::from_utf8_lossy(&page[off..off + len]).to_string();
        off += len;
        let data_type = match page[off] {
            0 => DataType::Integer,
            1 => DataType::Text,
            t => return Err(format!("Unknown type tag {} in metadata", t)),
        };
        off += 1;
        columns.push(Column { name, data_type });
    }
    Ok((root, columns))
}

/// Per-table handle: pager + schema + root page.
pub struct TableStore {
    pub pager: Pager,
    pub columns: Vec<Column>,
    pub root_page: u32,
}

pub struct Storage {
    tables: HashMap<String, TableStore>,
}

impl Storage {
    pub fn new() -> Self {
        Storage { tables: HashMap::new() }
    }

    /// Open an existing table from a .db file.
    pub fn open_table(&mut self, name: &str, path: &str) -> Result<(), String> {
        let mut pager = Pager::open(path)?;
        let (root_page, columns) = read_metadata(&mut pager)?;
        self.tables.insert(
            name.to_string(),
            TableStore { pager, columns, root_page },
        );
        Ok(())
    }

    pub fn create_table(&mut self, name: String, columns: Vec<Column>) -> Result<(), String> {
        if self.tables.contains_key(&name) {
            return Err(format!("Table '{}' already exists.", name));
        }
        let path = format!("{}.db", name);
        let mut pager = Pager::open(&path)?;

        // Page 0 = metadata.
        pager.allocate_page()?;

        // Page 1+ = B+Tree root (a single empty leaf).
        let root_page = btree::create_tree(&mut pager)?;

        write_metadata(&mut pager, root_page, &columns)?;
        pager.flush()?;

        self.tables.insert(
            name.clone(),
            TableStore { pager, columns, root_page },
        );
        Ok(())
    }

    pub fn insert(&mut self, table_name: &str, values: Vec<Value>) -> Result<(), String> {
        let store = self
            .tables
            .get_mut(table_name)
            .ok_or_else(|| format!("Table '{}' not found.", table_name))?;

        if values.len() != store.columns.len() {
            return Err(format!(
                "Expected {} values, got {}.",
                store.columns.len(),
                values.len()
            ));
        }

        // Type-check.
        for (i, (col, val)) in store.columns.iter().zip(values.iter()).enumerate() {
            match (&col.data_type, val) {
                (DataType::Integer, Value::Integer(_)) => {}
                (DataType::Text, Value::Text(_)) => {}
                _ => {
                    return Err(format!(
                        "Column '{}' (position {}) expects {:?}, got {:?}.",
                        col.name, i, col.data_type, val
                    ))
                }
            }
        }

        // Extract the key (first INTEGER column value).
        let key = extract_key(&store.columns, &values)?;

        // Serialize the row.
        let rsize = row::row_size(&store.columns);
        let mut buf = vec![0u8; rsize];
        let r = Row { values };
        row::serialize(&r, &store.columns, &mut buf);

        // B+Tree insert.
        let new_root = btree::insert(&mut store.pager, store.root_page, key, &buf)?;
        if new_root != store.root_page {
            store.root_page = new_root;
            write_metadata(&mut store.pager, new_root, &store.columns)?;
        }
        store.pager.flush()?;
        Ok(())
    }

    pub fn select_all(&mut self, table_name: &str) -> Result<(Vec<String>, Vec<Row>), String> {
        let store = self
            .tables
            .get_mut(table_name)
            .ok_or_else(|| format!("Table '{}' not found.", table_name))?;

        let rsize = row::row_size(&store.columns);
        let raw_rows = btree::scan_all(&mut store.pager, store.root_page, rsize)?;

        let headers: Vec<String> = store.columns.iter().map(|c| c.name.clone()).collect();
        let rows: Vec<Row> = raw_rows
            .iter()
            .map(|buf| row::deserialize(buf, &store.columns))
            .collect();
        Ok((headers, rows))
    }

    /// Dump the B+Tree structure for a table as a human-readable string.
    pub fn dump_btree(&mut self, table_name: &str) -> Result<String, String> {
        let store = self
            .tables
            .get_mut(table_name)
            .ok_or_else(|| format!("Table '{}' not found.", table_name))?;
        let rsize = row::row_size(&store.columns);
        btree::dump_tree(&mut store.pager, store.root_page, rsize, 0)
    }
}

/// Extract the B+Tree key from the row values.
/// Uses the first INTEGER column; falls back to a 0 key if none exists.
fn extract_key(columns: &[Column], values: &[Value]) -> Result<i64, String> {
    for (col, val) in columns.iter().zip(values.iter()) {
        if col.data_type == DataType::Integer {
            if let Value::Integer(n) = val {
                return Ok(*n);
            }
        }
    }
    Ok(0)
}
