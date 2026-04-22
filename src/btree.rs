/// B+Tree storage engine with slotted-page leaves for variable-size rows.
///
/// Leaf page layout (slotted)
/// --------------------------
///   [0]        node_type    = 0x01
///   [1..5]     num_cells    (u32 LE)
///   [5..9]     next_leaf    (u32 LE) — sibling pointer (0 = none)
///   [9..11]    data_start   (u16 LE) — byte offset where data region begins
///   [11..]     slot directory: num_cells × 4 bytes, each [offset: u16][length: u16]
///   ...        free space
///   [data_start..PAGE_SIZE]  cell data packed from end of page toward header
///
/// Each cell in the data area: [key: i64 LE (8 bytes)][row_data: variable bytes]
///
/// Internal node layout (unchanged)
/// ---------------------------------
///   [0]        node_type    = 0x02
///   [1..5]     num_keys     (u32 LE)
///   [5..9]     right_child  (u32 LE) — rightmost child page
///   [9..]      entries: each is [child_page: u32 LE (4 bytes)][key: i64 LE (8 bytes)]

use crate::pager::{Pager, PAGE_SIZE};

const NODE_TYPE_LEAF: u8 = 0x01;
const NODE_TYPE_INTERNAL: u8 = 0x02;

// ---------------------------------------------------------------------------
// Leaf slotted-page constants
// ---------------------------------------------------------------------------
const LEAF_NUM_CELLS_OFFSET: usize = 1;
const LEAF_NEXT_LEAF_OFFSET: usize = 5;
const LEAF_DATA_START_OFFSET: usize = 9;
const LEAF_HEADER_SIZE: usize = 11;
const SLOT_SIZE: usize = 4; // offset(u16) + length(u16)
const KEY_SIZE: usize = 8;

/// Minimum free space needed to insert a cell: slot entry + key + at least 1 byte of row.
/// Used as a sanity bound. The real check uses actual cell size.
const MAX_CELL_DATA: usize = PAGE_SIZE - LEAF_HEADER_SIZE - SLOT_SIZE;

// ---------------------------------------------------------------------------
// Internal node constants (unchanged)
// ---------------------------------------------------------------------------
const INTERNAL_NUM_KEYS_OFFSET: usize = 1;
const RIGHT_CHILD_OFFSET: usize = 5;
const INTERNAL_HEADER_SIZE: usize = 9;
const INTERNAL_ENTRY_SIZE: usize = 4 + 8; // child_page(4) + key(8)

// ---------------------------------------------------------------------------
// Little-endian helpers
// ---------------------------------------------------------------------------

fn read_u16(page: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(page[off..off + 2].try_into().unwrap())
}
fn write_u16(page: &mut [u8], off: usize, v: u16) {
    page[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn read_u32(page: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(page[off..off + 4].try_into().unwrap())
}
fn write_u32(page: &mut [u8], off: usize, v: u32) {
    page[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn read_i64(page: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(page[off..off + 8].try_into().unwrap())
}
fn write_i64(page: &mut [u8], off: usize, v: i64) {
    page[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Leaf helpers — slotted page
// ---------------------------------------------------------------------------

fn leaf_num_cells(page: &[u8]) -> u32 {
    read_u32(page, LEAF_NUM_CELLS_OFFSET)
}
fn leaf_set_num_cells(page: &mut [u8], n: u32) {
    write_u32(page, LEAF_NUM_CELLS_OFFSET, n);
}
fn leaf_next_leaf(page: &[u8]) -> u32 {
    read_u32(page, LEAF_NEXT_LEAF_OFFSET)
}
fn leaf_set_next_leaf(page: &mut [u8], v: u32) {
    write_u32(page, LEAF_NEXT_LEAF_OFFSET, v);
}
fn leaf_data_start(page: &[u8]) -> u16 {
    read_u16(page, LEAF_DATA_START_OFFSET)
}
fn leaf_set_data_start(page: &mut [u8], v: u16) {
    write_u16(page, LEAF_DATA_START_OFFSET, v);
}

/// Byte offset of slot `i` in the slot directory.
fn slot_offset(i: usize) -> usize {
    LEAF_HEADER_SIZE + i * SLOT_SIZE
}

/// Read slot i: returns (data_offset, data_length).
fn read_slot(page: &[u8], i: usize) -> (u16, u16) {
    let off = slot_offset(i);
    (read_u16(page, off), read_u16(page, off + 2))
}

/// Write slot i.
fn write_slot(page: &mut [u8], i: usize, data_off: u16, data_len: u16) {
    let off = slot_offset(i);
    write_u16(page, off, data_off);
    write_u16(page, off + 2, data_len);
}

/// Read the key from cell i (key is the first 8 bytes of cell data).
fn leaf_cell_key(page: &[u8], i: usize) -> i64 {
    let (off, _) = read_slot(page, i);
    read_i64(page, off as usize)
}

/// Read the row data (excluding key) from cell i.
fn leaf_cell_row(page: &[u8], i: usize) -> &[u8] {
    let (off, len) = read_slot(page, i);
    let start = off as usize + KEY_SIZE;
    let end = off as usize + len as usize;
    &page[start..end]
}

/// Free space available in a leaf page.
fn leaf_free_space(page: &[u8]) -> usize {
    let num = leaf_num_cells(page) as usize;
    let slots_end = LEAF_HEADER_SIZE + num * SLOT_SIZE;
    let data_start = leaf_data_start(page) as usize;
    if data_start > slots_end {
        data_start - slots_end
    } else {
        0
    }
}

/// Initialise a page as an empty leaf node.
fn init_leaf(page: &mut [u8]) {
    page[0] = NODE_TYPE_LEAF;
    leaf_set_num_cells(page, 0);
    leaf_set_next_leaf(page, 0);
    leaf_set_data_start(page, PAGE_SIZE as u16);
}

// ---------------------------------------------------------------------------
// Internal node helpers (unchanged from previous milestone)
// ---------------------------------------------------------------------------

fn internal_num_keys(page: &[u8]) -> u32 {
    read_u32(page, INTERNAL_NUM_KEYS_OFFSET)
}
fn internal_set_num_keys(page: &mut [u8], n: u32) {
    write_u32(page, INTERNAL_NUM_KEYS_OFFSET, n);
}
fn internal_right_child(page: &[u8]) -> u32 {
    read_u32(page, RIGHT_CHILD_OFFSET)
}
fn internal_set_right_child(page: &mut [u8], v: u32) {
    write_u32(page, RIGHT_CHILD_OFFSET, v);
}
fn internal_entry_offset(idx: usize) -> usize {
    INTERNAL_HEADER_SIZE + idx * INTERNAL_ENTRY_SIZE
}
fn internal_child(page: &[u8], idx: usize) -> u32 {
    read_u32(page, internal_entry_offset(idx))
}
fn internal_key(page: &[u8], idx: usize) -> i64 {
    read_i64(page, internal_entry_offset(idx) + 4)
}
fn internal_set_entry(page: &mut [u8], idx: usize, child: u32, key: i64) {
    let off = internal_entry_offset(idx);
    write_u32(page, off, child);
    write_i64(page, off + 4, key);
}
fn internal_max_keys() -> usize {
    (PAGE_SIZE - INTERNAL_HEADER_SIZE) / INTERNAL_ENTRY_SIZE
}
fn init_internal(page: &mut [u8]) {
    page[0] = NODE_TYPE_INTERNAL;
    internal_set_num_keys(page, 0);
    internal_set_right_child(page, 0);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Maximum row data size that can be stored (must fit in a leaf with at least 2 cells).
pub fn max_row_data_size() -> usize {
    // Each cell needs: SLOT_SIZE + KEY_SIZE + row_data
    // Two cells must fit: 2 * (SLOT_SIZE + KEY_SIZE + row_data) <= PAGE_SIZE - LEAF_HEADER_SIZE
    (PAGE_SIZE - LEAF_HEADER_SIZE) / 2 - SLOT_SIZE - KEY_SIZE
}

/// Create a new B+Tree: allocates a single leaf page and returns its page number.
pub fn create_tree(pager: &mut Pager) -> Result<u32, String> {
    let page_num = pager.allocate_page()?;
    let page = pager.get_page_mut(page_num)?;
    init_leaf(page);
    Ok(page_num)
}

/// Insert a key + serialized row into the tree rooted at root_page.
/// Returns the (possibly new) root page number (root changes on root split).
pub fn insert(
    pager: &mut Pager,
    root_page: u32,
    key: i64,
    row_data: &[u8],
) -> Result<u32, String> {
    let cell_size = KEY_SIZE + row_data.len();
    let space_needed = SLOT_SIZE + cell_size;

    if cell_size > MAX_CELL_DATA {
        return Err(format!(
            "Row too large ({} bytes). Maximum is {} bytes.",
            row_data.len(),
            max_row_data_size()
        ));
    }

    // Find the leaf that should contain this key, tracking the path.
    let mut path: Vec<(u32, usize)> = Vec::new();
    let mut cur = root_page;

    loop {
        let node_type = pager.get_page(cur)?[0];
        if node_type == NODE_TYPE_LEAF {
            break;
        }
        let page = pager.get_page(cur)?;
        let nk = internal_num_keys(page) as usize;
        let mut idx = 0;
        while idx < nk && key > internal_key(page, idx) {
            idx += 1;
        }
        let child = if idx < nk {
            internal_child(page, idx)
        } else {
            internal_right_child(page)
        };
        path.push((cur, idx));
        cur = child;
    }

    // Check if leaf has room.
    let free = leaf_free_space(pager.get_page(cur)?);
    if free >= space_needed {
        insert_into_leaf(pager, cur, key, row_data)?;
        return Ok(root_page);
    }

    // Leaf is full — split.
    let (median_key, new_leaf) = split_leaf(pager, cur, key, row_data)?;

    // Propagate the median key upward.
    let mut promote_key = median_key;
    let mut promote_child = new_leaf;

    for &(parent_page, _child_idx) in path.iter().rev() {
        let page = pager.get_page(parent_page)?;
        let nk = internal_num_keys(page) as usize;
        if nk < internal_max_keys() {
            insert_into_internal(pager, parent_page, promote_key, promote_child)?;
            return Ok(root_page);
        }
        let (new_median, new_internal) =
            split_internal(pager, parent_page, promote_key, promote_child)?;
        promote_key = new_median;
        promote_child = new_internal;
    }

    // Root was split — create a new root.
    let new_root = pager.allocate_page()?;
    let page = pager.get_page_mut(new_root)?;
    init_internal(page);
    internal_set_num_keys(page, 1);
    internal_set_entry(page, 0, root_page, promote_key);
    internal_set_right_child(page, promote_child);
    Ok(new_root)
}

/// Scan all rows in key order by walking the leaf chain.
/// Returns Vec of raw row byte vectors (variable length).
pub fn scan_all(pager: &mut Pager, root_page: u32) -> Result<Vec<Vec<u8>>, String> {
    // Walk to the leftmost leaf.
    let mut cur = root_page;
    loop {
        let node_type = pager.get_page(cur)?[0];
        if node_type == NODE_TYPE_LEAF {
            break;
        }
        let page = pager.get_page(cur)?;
        let nk = internal_num_keys(page) as usize;
        cur = if nk > 0 {
            internal_child(page, 0)
        } else {
            internal_right_child(page)
        };
    }

    // Walk the leaf chain via next_leaf pointers.
    let mut rows = Vec::new();
    loop {
        let page = pager.get_page(cur)?;
        let n = leaf_num_cells(page) as usize;
        for i in 0..n {
            rows.push(leaf_cell_row(page, i).to_vec());
        }
        let next = leaf_next_leaf(page);
        if next == 0 {
            break;
        }
        cur = next;
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Leaf insertion / splitting
// ---------------------------------------------------------------------------

/// Insert a cell into a leaf that has room, maintaining sorted key order.
fn insert_into_leaf(
    pager: &mut Pager,
    page_num: u32,
    key: i64,
    row_data: &[u8],
) -> Result<(), String> {
    let cell_size = KEY_SIZE + row_data.len();
    let page = pager.get_page_mut(page_num)?;
    let num = leaf_num_cells(page) as usize;

    // Find sorted insertion point.
    let mut pos = 0;
    while pos < num && leaf_cell_key(page, pos) < key {
        pos += 1;
    }

    // Shift slot entries [pos..num) right by one to make room.
    for i in (pos..num).rev() {
        let (off, len) = read_slot(page, i);
        write_slot(page, i + 1, off, len);
    }

    // Allocate space in the data region (grows downward).
    let new_data_start = leaf_data_start(page) as usize - cell_size;
    leaf_set_data_start(page, new_data_start as u16);

    // Write cell data: key + row_data.
    write_i64(page, new_data_start, key);
    page[new_data_start + KEY_SIZE..new_data_start + cell_size].copy_from_slice(row_data);

    // Write the new slot.
    write_slot(page, pos, new_data_start as u16, cell_size as u16);

    leaf_set_num_cells(page, (num + 1) as u32);
    Ok(())
}

/// Collect all cells from a leaf page as (key, row_data) pairs, sorted by key.
fn collect_leaf_cells(page: &[u8]) -> Vec<(i64, Vec<u8>)> {
    let n = leaf_num_cells(page) as usize;
    let mut cells = Vec::with_capacity(n);
    for i in 0..n {
        let key = leaf_cell_key(page, i);
        let row = leaf_cell_row(page, i).to_vec();
        cells.push((key, row));
    }
    cells
}

/// Write a set of cells into an empty leaf page.
fn write_leaf_cells(page: &mut [u8], cells: &[(i64, Vec<u8>)]) {
    init_leaf(page);
    let mut data_start = PAGE_SIZE;
    for (i, (key, row)) in cells.iter().enumerate() {
        let cell_size = KEY_SIZE + row.len();
        data_start -= cell_size;
        write_i64(page, data_start, *key);
        page[data_start + KEY_SIZE..data_start + cell_size].copy_from_slice(row);
        write_slot(page, i, data_start as u16, cell_size as u16);
    }
    leaf_set_num_cells(page, cells.len() as u32);
    leaf_set_data_start(page, data_start as u16);
}

/// Split a full leaf. The new key/row is included in the split.
/// Returns (median_key, new_page_num).
fn split_leaf(
    pager: &mut Pager,
    page_num: u32,
    key: i64,
    row_data: &[u8],
) -> Result<(i64, u32), String> {
    // Collect all existing cells + the new one, sorted by key.
    let mut all_cells = collect_leaf_cells(pager.get_page(page_num)?);
    let mut ins = 0;
    while ins < all_cells.len() && all_cells[ins].0 < key {
        ins += 1;
    }
    all_cells.insert(ins, (key, row_data.to_vec()));

    // Find split point: aim for ~50% of total data bytes in the left half.
    let total_data: usize = all_cells
        .iter()
        .map(|(_, r)| SLOT_SIZE + KEY_SIZE + r.len())
        .sum();
    let half = total_data / 2;
    let mut left_count = 0;
    let mut left_bytes = 0;
    for (_, r) in &all_cells {
        let cell_cost = SLOT_SIZE + KEY_SIZE + r.len();
        if left_bytes + cell_cost > half && left_count > 0 {
            break;
        }
        left_bytes += cell_cost;
        left_count += 1;
    }
    // Ensure at least 1 cell on each side.
    if left_count == 0 {
        left_count = 1;
    }
    if left_count >= all_cells.len() {
        left_count = all_cells.len() - 1;
    }

    let right_cells = all_cells.split_off(left_count);
    let left_cells = all_cells;

    // Preserve the old leaf'"'"'s next_leaf so the new leaf inherits it.
    let old_next = leaf_next_leaf(pager.get_page(page_num)?);

    // Allocate the new right leaf.
    let new_page_num = pager.allocate_page()?;

    // Write left half back into the original leaf.
    {
        let page = pager.get_page_mut(page_num)?;
        write_leaf_cells(page, &left_cells);
        leaf_set_next_leaf(page, new_page_num);
    }

    // Write right half into the new leaf.
    {
        let page = pager.get_page_mut(new_page_num)?;
        write_leaf_cells(page, &right_cells);
        leaf_set_next_leaf(page, old_next);
    }

    let median_key = right_cells[0].0;
    Ok((median_key, new_page_num))
}

// ---------------------------------------------------------------------------
// Internal insertion / splitting (unchanged logic)
// ---------------------------------------------------------------------------

fn insert_into_internal(
    pager: &mut Pager,
    page_num: u32,
    key: i64,
    right_child: u32,
) -> Result<(), String> {
    let page = pager.get_page_mut(page_num)?;
    let nk = internal_num_keys(page) as usize;

    let mut pos = 0;
    while pos < nk && internal_key(page, pos) < key {
        pos += 1;
    }

    for i in (pos..nk).rev() {
        let k = internal_key(page, i);
        let c = internal_child(page, i);
        internal_set_entry(page, i + 1, c, k);
    }

    let old_child_at_pos = if pos < nk {
        internal_child(page, pos + 1)
    } else {
        internal_right_child(page)
    };

    internal_set_entry(page, pos, old_child_at_pos, key);

    if pos + 1 <= nk {
        write_u32(page, internal_entry_offset(pos + 1), right_child);
    }
    if pos == nk {
        internal_set_right_child(page, right_child);
    }

    internal_set_num_keys(page, (nk + 1) as u32);
    Ok(())
}

fn split_internal(
    pager: &mut Pager,
    page_num: u32,
    key: i64,
    right_child: u32,
) -> Result<(i64, u32), String> {
    let max = internal_max_keys();
    let total = max + 1;

    let mut entries: Vec<(u32, i64)> = Vec::with_capacity(total);
    let old_right;
    {
        let page = pager.get_page(page_num)?;
        old_right = internal_right_child(page);
        for i in 0..max {
            entries.push((internal_child(page, i), internal_key(page, i)));
        }
    }

    let mut ins = 0;
    while ins < entries.len() && entries[ins].1 < key {
        ins += 1;
    }

    let new_entry_child = if ins < entries.len() {
        entries[ins].0
    } else {
        old_right
    };
    entries.insert(ins, (new_entry_child, key));

    if ins + 1 < entries.len() {
        entries[ins + 1].0 = right_child;
    }
    let combined_right = if ins + 1 == entries.len() {
        right_child
    } else {
        old_right
    };

    let mid = total / 2;
    let median_key = entries[mid].1;

    let left_count = mid;
    let right_count = total - mid - 1;

    let left_right_child = entries[mid].0;
    let right_right_child = combined_right;

    {
        let page = pager.get_page_mut(page_num)?;
        init_internal(page);
        internal_set_num_keys(page, left_count as u32);
        for i in 0..left_count {
            internal_set_entry(page, i, entries[i].0, entries[i].1);
        }
        internal_set_right_child(page, left_right_child);
    }

    let new_page = pager.allocate_page()?;
    {
        let page = pager.get_page_mut(new_page)?;
        init_internal(page);
        internal_set_num_keys(page, right_count as u32);
        for i in 0..right_count {
            let src = &entries[mid + 1 + i];
            internal_set_entry(page, i, src.0, src.1);
        }
        internal_set_right_child(page, right_right_child);
    }

    Ok((median_key, new_page))
}

/// Dump the tree structure as a human-readable string for debugging.
pub fn dump_tree(
    pager: &mut Pager,
    page_num: u32,
    depth: usize,
) -> Result<String, String> {
    let indent = "  ".repeat(depth);
    let page = pager.get_page(page_num)?;
    let node_type = page[0];

    if node_type == NODE_TYPE_LEAF {
        let n = leaf_num_cells(page) as usize;
        let next = leaf_next_leaf(page);
        let free = leaf_free_space(page);
        let mut out = format!(
            "{}Leaf (page={}, cells={}, next={}, free={})",
            indent, page_num, n, next, free
        );
        for i in 0..n {
            let k = leaf_cell_key(page, i);
            out.push_str(&format!("\n{}  - key {}", indent, k));
        }
        Ok(out)
    } else if node_type == NODE_TYPE_INTERNAL {
        let nk = internal_num_keys(page) as usize;
        let rc = internal_right_child(page);
        let mut children: Vec<u32> = Vec::with_capacity(nk + 1);
        let mut keys: Vec<i64> = Vec::with_capacity(nk);
        for i in 0..nk {
            children.push(internal_child(page, i));
            keys.push(internal_key(page, i));
        }
        children.push(rc);

        let mut out = format!(
            "{}Internal (page={}, keys={})",
            indent, page_num, nk
        );
        for i in 0..nk {
            out.push_str(&format!("\n{}", dump_tree(pager, children[i], depth + 1)?));
            out.push_str(&format!("\n{}  key {}", indent, keys[i]));
        }
        out.push_str(&format!("\n{}", dump_tree(pager, children[nk], depth + 1)?));
        Ok(out)
    } else {
        Err(format!("Unknown node type {} at page {}", node_type, page_num))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_pager(path: &str) -> Pager {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(format!("{}.wal", path));
        Pager::open(path).unwrap()
    }

    /// Helper: insert keys with a minimal row (just the key as 8 bytes).
    fn insert_and_collect(pager: &mut Pager, keys: &[i64]) -> (u32, Vec<i64>) {
        let mut root = create_tree(pager).unwrap();
        for &k in keys {
            let row = k.to_le_bytes().to_vec();
            root = insert(pager, root, k, &row).unwrap();
        }
        let rows = scan_all(pager, root).unwrap();
        let out: Vec<i64> = rows
            .iter()
            .map(|r| i64::from_le_bytes(r[..8].try_into().unwrap()))
            .collect();
        (root, out)
    }

    #[test]
    fn insert_and_scan_small() {
        let path = "/tmp/mukhidb_btree_small.db";
        let mut pager = test_pager(path);
        let (_, keys) = insert_and_collect(&mut pager, &[3, 1, 4, 1, 5, 9, 2, 6]);
        assert_eq!(keys, vec![1, 1, 2, 3, 4, 5, 6, 9]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn insert_triggers_splits() {
        let path = "/tmp/mukhidb_btree_split.db";
        let mut pager = test_pager(path);
        // Insert enough rows to trigger multiple splits.
        let count = 200;
        let keys: Vec<i64> = (0..count).collect();
        let (_, out) = insert_and_collect(&mut pager, &keys);
        assert_eq!(out.len(), count as usize);
        assert_eq!(out, keys);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reverse_order_insert() {
        let path = "/tmp/mukhidb_btree_reverse.db";
        let mut pager = test_pager(path);
        let keys: Vec<i64> = (0..100).rev().collect();
        let (_, out) = insert_and_collect(&mut pager, &keys);
        let expected: Vec<i64> = (0..100).collect();
        assert_eq!(out, expected);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn duplicate_keys() {
        let path = "/tmp/mukhidb_btree_dups.db";
        let mut pager = test_pager(path);
        let keys: Vec<i64> = vec![5; 50];
        let (_, out) = insert_and_collect(&mut pager, &keys);
        assert_eq!(out.len(), 50);
        assert!(out.iter().all(|&k| k == 5));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn single_row() {
        let path = "/tmp/mukhidb_btree_single.db";
        let mut pager = test_pager(path);
        let (_, out) = insert_and_collect(&mut pager, &[42]);
        assert_eq!(out, vec![42]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn empty_tree_scan() {
        let path = "/tmp/mukhidb_btree_empty.db";
        let mut pager = test_pager(path);
        let root = create_tree(&mut pager).unwrap();
        let rows = scan_all(&mut pager, root).unwrap();
        assert!(rows.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn negative_keys() {
        let path = "/tmp/mukhidb_btree_neg.db";
        let mut pager = test_pager(path);
        let keys = vec![-10, 0, -5, 3, -1, 7];
        let (_, out) = insert_and_collect(&mut pager, &keys);
        assert_eq!(out, vec![-10, -5, -1, 0, 3, 7]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn persistence_round_trip() {
        let path = "/tmp/mukhidb_btree_persist.db";
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(format!("{}.wal", path));
        let keys: Vec<i64> = (0..60).collect();

        let root = {
            let mut pager = Pager::open(path).unwrap();
            let (root, _) = insert_and_collect(&mut pager, &keys);
            pager.flush().unwrap();
            root
        };

        {
            let mut pager = Pager::open(path).unwrap();
            let rows = scan_all(&mut pager, root).unwrap();
            let out: Vec<i64> = rows
                .iter()
                .map(|r| i64::from_le_bytes(r[..8].try_into().unwrap()))
                .collect();
            assert_eq!(out, keys);
        }

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn variable_size_rows() {
        let path = "/tmp/mukhidb_btree_varsize.db";
        let mut pager = test_pager(path);
        let mut root = create_tree(&mut pager).unwrap();

        // Insert rows with varying text lengths.
        let texts = vec!["Hi", "Hello, world!", "", "A medium length string for testing", "x"];
        for (i, text) in texts.iter().enumerate() {
            let key = i as i64;
            // Row format: i64 key + u32 text_len + text bytes (matches row.rs encoding)
            let mut row_data = Vec::new();
            row_data.extend_from_slice(&key.to_le_bytes());
            row_data.extend_from_slice(&(text.len() as u32).to_le_bytes());
            row_data.extend_from_slice(text.as_bytes());
            root = insert(&mut pager, root, key, &row_data).unwrap();
        }

        let rows = scan_all(&mut pager, root).unwrap();
        assert_eq!(rows.len(), 5);

        // Verify rows come back in key order and text is intact.
        for (i, row) in rows.iter().enumerate() {
            let k = i64::from_le_bytes(row[..8].try_into().unwrap());
            assert_eq!(k, i as i64);
            let text_len = u32::from_le_bytes(row[8..12].try_into().unwrap()) as usize;
            let text = std::str::from_utf8(&row[12..12 + text_len]).unwrap();
            assert_eq!(text, texts[i]);
        }

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn large_dataset_500_rows() {
        let path = "/tmp/mukhidb_btree_500.db";
        let mut pager = test_pager(path);
        let keys: Vec<i64> = (0..500).collect();
        let (_, out) = insert_and_collect(&mut pager, &keys);
        assert_eq!(out.len(), 500);
        assert_eq!(out, keys);
        fs::remove_file(path).unwrap();
    }
}