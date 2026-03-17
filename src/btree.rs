/// B+Tree storage engine operating on fixed-size pages via the Pager.
///
/// Page layout
/// -----------
/// Leaf node:
///   [0]       node_type   = 0x01
///   [1..5]    num_cells   (u32 LE) — number of key/row pairs
///   [5..9]    next_leaf   (u32 LE) — sibling pointer (0 = none)
///   [9..]     cells: each cell is [key: i64 LE (8 bytes)] [row_data: row_size bytes]
///
/// Internal node:
///   [0]       node_type   = 0x02
///   [1..5]    num_keys    (u32 LE)
///   [5..9]    right_child (u32 LE) — rightmost child page
///   [9..]     entries: each is [child_page: u32 LE (4 bytes)] [key: i64 LE (8 bytes)]

use crate::pager::{Pager, PAGE_SIZE};

const NODE_TYPE_LEAF: u8 = 0x01;
const NODE_TYPE_INTERNAL: u8 = 0x02;

// Header offsets shared by both node types.
const NUM_CELLS_OFFSET: usize = 1;
// Leaf-specific
const NEXT_LEAF_OFFSET: usize = 5;
const LEAF_HEADER_SIZE: usize = 9;
// Internal-specific
const RIGHT_CHILD_OFFSET: usize = 5;
const INTERNAL_HEADER_SIZE: usize = 9;
const INTERNAL_ENTRY_SIZE: usize = 4 + 8; // child_page(4) + key(8)

// ---------------------------------------------------------------------------
// Helpers to read/write little-endian integers from a page slice
// ---------------------------------------------------------------------------

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
// Leaf helpers
// ---------------------------------------------------------------------------

fn leaf_cell_size(row_size: usize) -> usize {
    8 + row_size // key(8) + row
}

fn leaf_max_cells(row_size: usize) -> usize {
    (PAGE_SIZE - LEAF_HEADER_SIZE) / leaf_cell_size(row_size)
}

fn leaf_cell_offset(cell_idx: usize, row_size: usize) -> usize {
    LEAF_HEADER_SIZE + cell_idx * leaf_cell_size(row_size)
}

fn leaf_num_cells(page: &[u8]) -> u32 {
    read_u32(page, NUM_CELLS_OFFSET)
}

fn leaf_set_num_cells(page: &mut [u8], n: u32) {
    write_u32(page, NUM_CELLS_OFFSET, n);
}

fn leaf_next_leaf(page: &[u8]) -> u32 {
    read_u32(page, NEXT_LEAF_OFFSET)
}

fn leaf_set_next_leaf(page: &mut [u8], v: u32) {
    write_u32(page, NEXT_LEAF_OFFSET, v);
}

fn leaf_cell_key(page: &[u8], cell_idx: usize, row_size: usize) -> i64 {
    read_i64(page, leaf_cell_offset(cell_idx, row_size))
}

fn leaf_cell_value(page: &[u8], cell_idx: usize, row_size: usize) -> &[u8] {
    let off = leaf_cell_offset(cell_idx, row_size) + 8;
    &page[off..off + row_size]
}

/// Initialise a page as an empty leaf node.
fn init_leaf(page: &mut [u8]) {
    page[0] = NODE_TYPE_LEAF;
    leaf_set_num_cells(page, 0);
    leaf_set_next_leaf(page, 0);
}

// ---------------------------------------------------------------------------
// Internal node helpers
// ---------------------------------------------------------------------------

fn internal_num_keys(page: &[u8]) -> u32 {
    read_u32(page, NUM_CELLS_OFFSET)
}

fn internal_set_num_keys(page: &mut [u8], n: u32) {
    write_u32(page, NUM_CELLS_OFFSET, n);
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
    let row_size = row_data.len();

    // Find the leaf that should contain this key, tracking the path of
    // (page_num, child_index_chosen) so we can split upward.
    let mut path: Vec<(u32, usize)> = Vec::new();
    let mut cur = root_page;

    loop {
        let node_type = pager.get_page(cur)?[0];
        if node_type == NODE_TYPE_LEAF {
            break;
        }
        // Internal node — binary search for the child to descend into.
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

    // cur is now the target leaf page.
    let max = leaf_max_cells(row_size);

    // Read current cell count.
    let num = leaf_num_cells(pager.get_page(cur)?) as usize;

    if num < max {
        // Room in this leaf — insert in sorted position.
        insert_into_leaf(pager, cur, key, row_data, row_size)?;
        return Ok(root_page);
    }

    // Leaf is full — split.
    let (median_key, new_leaf) = split_leaf(pager, cur, key, row_data, row_size)?;

    // Propagate the median key upward through internal nodes.
    let mut promote_key = median_key;
    let mut promote_child = new_leaf;

    for &(parent_page, _child_idx) in path.iter().rev() {
        let page = pager.get_page(parent_page)?;
        let nk = internal_num_keys(page) as usize;
        if nk < internal_max_keys() {
            insert_into_internal(pager, parent_page, promote_key, promote_child)?;
            return Ok(root_page);
        }
        // Internal node is also full — split it.
        let (new_median, new_internal) =
            split_internal(pager, parent_page, promote_key, promote_child)?;
        promote_key = new_median;
        promote_child = new_internal;
    }

    // If we get here, the root itself was split (or the root was the leaf).
    // Create a new root.
    let new_root = pager.allocate_page()?;
    let page = pager.get_page_mut(new_root)?;
    init_internal(page);
    internal_set_num_keys(page, 1);
    internal_set_entry(page, 0, root_page, promote_key);
    internal_set_right_child(page, promote_child);
    Ok(new_root)
}

/// Scan all rows in key order by walking the leaf chain.
/// Returns Vec of raw row byte vectors.
pub fn scan_all(pager: &mut Pager, root_page: u32, row_size: usize) -> Result<Vec<Vec<u8>>, String> {
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

    // Now walk the leaf chain via next_leaf pointers.
    let mut rows = Vec::new();
    loop {
        let page = pager.get_page(cur)?;
        let n = leaf_num_cells(page) as usize;
        for i in 0..n {
            rows.push(leaf_cell_value(page, i, row_size).to_vec());
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
// Internal insertion / splitting helpers
// ---------------------------------------------------------------------------

/// Insert a cell into a leaf that has room, maintaining sorted key order.
fn insert_into_leaf(
    pager: &mut Pager,
    page_num: u32,
    key: i64,
    row_data: &[u8],
    row_size: usize,
) -> Result<(), String> {
    let page = pager.get_page_mut(page_num)?;
    let num = leaf_num_cells(page) as usize;
    let cell_sz = leaf_cell_size(row_size);

    // Find insertion point (first cell with key >= new key).
    let mut pos = 0;
    while pos < num && leaf_cell_key(page, pos, row_size) < key {
        pos += 1;
    }

    // Shift cells right to make room.
    if pos < num {
        let src = leaf_cell_offset(pos, row_size);
        let count = (num - pos) * cell_sz;
        // copy within the page — use a temp buffer since we can't borrow twice.
        let mut tmp = vec![0u8; count];
        tmp.copy_from_slice(&page[src..src + count]);
        let dst = src + cell_sz;
        page[dst..dst + count].copy_from_slice(&tmp);
    }

    // Write the new cell.
    let off = leaf_cell_offset(pos, row_size);
    write_i64(page, off, key);
    page[off + 8..off + 8 + row_size].copy_from_slice(row_data);
    leaf_set_num_cells(page, (num + 1) as u32);
    Ok(())
}

/// Split a full leaf. The new key/row is included in the split.
/// Returns (median_key, new_page_num).
fn split_leaf(
    pager: &mut Pager,
    page_num: u32,
    key: i64,
    row_data: &[u8],
    row_size: usize,
) -> Result<(i64, u32), String> {
    let max = leaf_max_cells(row_size);
    let total = max + 1; // existing full leaf + 1 new cell

    // Collect all cells (existing + new) sorted by key into a temp buffer.
    let mut all_cells: Vec<(i64, Vec<u8>)> = Vec::with_capacity(total);
    {
        let page = pager.get_page(page_num)?;
        for i in 0..max {
            let k = leaf_cell_key(page, i, row_size);
            let v = leaf_cell_value(page, i, row_size).to_vec();
            all_cells.push((k, v));
        }
    }
    // Insert the new cell in sorted position.
    let mut ins = 0;
    while ins < all_cells.len() && all_cells[ins].0 < key {
        ins += 1;
    }
    all_cells.insert(ins, (key, row_data.to_vec()));

    let left_count = total / 2;
    let right_count = total - left_count;

    // Allocate the new right leaf.
    let new_page_num = pager.allocate_page()?;

    // Preserve the old leaf's next_leaf so the new leaf inherits it.
    let old_next = leaf_next_leaf(pager.get_page(page_num)?);

    // Write left half back into the original leaf.
    {
        let page = pager.get_page_mut(page_num)?;
        init_leaf(page);
        leaf_set_num_cells(page, left_count as u32);
        leaf_set_next_leaf(page, new_page_num);
        for i in 0..left_count {
            let off = leaf_cell_offset(i, row_size);
            write_i64(page, off, all_cells[i].0);
            page[off + 8..off + 8 + row_size].copy_from_slice(&all_cells[i].1);
        }
    }

    // Write right half into the new leaf.
    {
        let page = pager.get_page_mut(new_page_num)?;
        init_leaf(page);
        leaf_set_num_cells(page, right_count as u32);
        leaf_set_next_leaf(page, old_next);
        for i in 0..right_count {
            let off = leaf_cell_offset(i, row_size);
            let src = &all_cells[left_count + i];
            write_i64(page, off, src.0);
            page[off + 8..off + 8 + row_size].copy_from_slice(&src.1);
        }
    }

    // The median key is the first key of the right (new) leaf.
    let median_key = all_cells[left_count].0;
    Ok((median_key, new_page_num))
}

/// Insert a key + right-child pointer into an internal node that has room.
fn insert_into_internal(
    pager: &mut Pager,
    page_num: u32,
    key: i64,
    right_child: u32,
) -> Result<(), String> {
    let page = pager.get_page_mut(page_num)?;
    let nk = internal_num_keys(page) as usize;

    // Find insertion point.
    let mut pos = 0;
    while pos < nk && internal_key(page, pos) < key {
        pos += 1;
    }

    // The child pointer to the right of the new key becomes right_child.
    // The old child at pos..nk-1 and right_child need to shift.
    // Before insert: entries[0..nk], right_child = R
    // After insert at pos: the new entry's child = old entries[pos].child (left of key),
    //   new entry's right side = right_child, old entries shift right.

    // Shift entries [pos..nk-1] right by one.
    for i in (pos..nk).rev() {
        let k = internal_key(page, i);
        let c = internal_child(page, i);
        internal_set_entry(page, i + 1, c, k);
    }

    // The new entry at pos keeps the old child pointer that was at pos
    // (the left child of the key being promoted). The right_child becomes
    // the child pointer of entry pos+1 (or right_child of the node if pos == nk).
    let old_child_at_pos = if pos < nk {
        // We already shifted, so the old child at pos is now at pos+1.
        // But we need the original value. Since we shifted, entry[pos+1] has it.
        internal_child(page, pos + 1)
    } else {
        internal_right_child(page)
    };

    // Set the new entry: left child = old_child_at_pos, key = promoted key.
    internal_set_entry(page, pos, old_child_at_pos, key);

    // The right side of the new key is right_child.
    if pos + 1 <= nk {
        // entry[pos+1].child = right_child (this is the pointer between key[pos] and key[pos+1])
        // But we need to be careful: after shifting, entry[pos+1] already has the shifted data.
        // Actually, the child pointer at entry[pos+1] should be right_child.
        // And the old child that was there was already moved to be entry[pos].child above.
        write_u32(page, internal_entry_offset(pos + 1), right_child);
    }
    if pos == nk {
        // New key is the largest — right_child becomes the new right_child of the node.
        // The old right_child becomes the left child of the new entry (already set above).
        internal_set_right_child(page, right_child);
    }

    internal_set_num_keys(page, (nk + 1) as u32);
    Ok(())
}

/// Split a full internal node. Returns (median_key, new_page_num).
fn split_internal(
    pager: &mut Pager,
    page_num: u32,
    key: i64,
    right_child: u32,
) -> Result<(i64, u32), String> {
    let max = internal_max_keys();
    let total = max + 1;

    // Collect all entries + the new one, sorted.
    let mut entries: Vec<(u32, i64)> = Vec::with_capacity(total);
    let old_right;
    {
        let page = pager.get_page(page_num)?;
        old_right = internal_right_child(page);
        for i in 0..max {
            entries.push((internal_child(page, i), internal_key(page, i)));
        }
    }

    // Insert new entry in sorted position.
    let mut ins = 0;
    while ins < entries.len() && entries[ins].1 < key {
        ins += 1;
    }

    // Determine the child pointer for the new entry.
    // The new entry's left child is the child that was at position `ins` before
    // (or old_right if ins == entries.len()), and right_child goes to the right.
    let new_entry_child = if ins < entries.len() {
        entries[ins].0
    } else {
        old_right
    };
    entries.insert(ins, (new_entry_child, key));

    // Now fix up child pointers: after the inserted key, the next entry's child
    // should be right_child.
    if ins + 1 < entries.len() {
        entries[ins + 1].0 = right_child;
    }
    // The overall right_child of the combined set:
    let combined_right = if ins + 1 == entries.len() {
        right_child
    } else {
        old_right
    };

    // Split: left gets [0..mid), median is entries[mid], right gets [mid+1..total).
    let mid = total / 2;
    let median_key = entries[mid].1;

    let left_count = mid;
    let right_count = total - mid - 1;

    // The right child of the left node = entries[mid].child (left child of median).
    let left_right_child = entries[mid].0;

    // The right child of the right node = combined_right.
    let right_right_child = combined_right;

    // Rewrite the original page as the left node.
    {
        let page = pager.get_page_mut(page_num)?;
        init_internal(page);
        internal_set_num_keys(page, left_count as u32);
        for i in 0..left_count {
            internal_set_entry(page, i, entries[i].0, entries[i].1);
        }
        internal_set_right_child(page, left_right_child);
    }

    // Allocate and write the right node.
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
    row_size: usize,
    depth: usize,
) -> Result<String, String> {
    let indent = "  ".repeat(depth);
    let page = pager.get_page(page_num)?;
    let node_type = page[0];

    if node_type == NODE_TYPE_LEAF {
        let n = leaf_num_cells(page) as usize;
        let next = leaf_next_leaf(page);
        let mut out = format!(
            "{}Leaf (page={}, cells={}, next={})",
            indent, page_num, n, next
        );
        for i in 0..n {
            let k = leaf_cell_key(page, i, row_size);
            out.push_str(&format!("\n{}  - key {}", indent, k));
        }
        Ok(out)
    } else if node_type == NODE_TYPE_INTERNAL {
        let nk = internal_num_keys(page) as usize;
        let rc = internal_right_child(page);
        // Collect children and keys from the page before recursive calls.
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
            out.push_str(&format!("\n{}", dump_tree(pager, children[i], row_size, depth + 1)?));
            out.push_str(&format!("\n{}  key {}", indent, keys[i]));
        }
        out.push_str(&format!("\n{}", dump_tree(pager, children[nk], row_size, depth + 1)?));
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
        Pager::open(path).unwrap()
    }

    fn insert_and_collect(pager: &mut Pager, keys: &[i64], row_size: usize) -> (u32, Vec<i64>) {
        let mut root = create_tree(pager).unwrap();
        let data = vec![0u8; row_size];
        for &k in keys {
            let mut buf = data.clone();
            buf[..8].copy_from_slice(&k.to_le_bytes());
            root = insert(pager, root, k, &buf).unwrap();
        }
        let rows = scan_all(pager, root, row_size).unwrap();
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
        let (_, keys) = insert_and_collect(&mut pager, &[3, 1, 4, 1, 5, 9, 2, 6], 8);
        assert_eq!(keys, vec![1, 1, 2, 3, 4, 5, 6, 9]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn insert_triggers_splits() {
        let path = "/tmp/mukhidb_btree_split.db";
        let mut pager = test_pager(path);
        let row_size = 256;
        let max_per_leaf = leaf_max_cells(row_size);
        let count = max_per_leaf * 4;
        let keys: Vec<i64> = (0..count as i64).collect();
        let (_, out) = insert_and_collect(&mut pager, &keys, row_size);
        assert_eq!(out.len(), count);
        assert_eq!(out, keys); // should come back in sorted order
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reverse_order_insert() {
        let path = "/tmp/mukhidb_btree_reverse.db";
        let mut pager = test_pager(path);
        let keys: Vec<i64> = (0..100).rev().collect();
        let (_, out) = insert_and_collect(&mut pager, &keys, 8);
        let expected: Vec<i64> = (0..100).collect();
        assert_eq!(out, expected);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn duplicate_keys() {
        let path = "/tmp/mukhidb_btree_dups.db";
        let mut pager = test_pager(path);
        let keys: Vec<i64> = vec![5; 50];
        let (_, out) = insert_and_collect(&mut pager, &keys, 8);
        assert_eq!(out.len(), 50);
        assert!(out.iter().all(|&k| k == 5));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn single_row() {
        let path = "/tmp/mukhidb_btree_single.db";
        let mut pager = test_pager(path);
        let (_, out) = insert_and_collect(&mut pager, &[42], 8);
        assert_eq!(out, vec![42]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn empty_tree_scan() {
        let path = "/tmp/mukhidb_btree_empty.db";
        let mut pager = test_pager(path);
        let root = create_tree(&mut pager).unwrap();
        let rows = scan_all(&mut pager, root, 8).unwrap();
        assert!(rows.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn negative_keys() {
        let path = "/tmp/mukhidb_btree_neg.db";
        let mut pager = test_pager(path);
        let keys = vec![-10, 0, -5, 3, -1, 7];
        let (_, out) = insert_and_collect(&mut pager, &keys, 8);
        assert_eq!(out, vec![-10, -5, -1, 0, 3, 7]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn persistence_round_trip() {
        let path = "/tmp/mukhidb_btree_persist.db";
        let _ = fs::remove_file(path);
        let row_size = 8;
        let keys: Vec<i64> = (0..60).collect();

        // Write
        let root = {
            let mut pager = Pager::open(path).unwrap();
            let (root, _) = insert_and_collect(&mut pager, &keys, row_size);
            pager.flush().unwrap();
            root
        };

        // Reopen and read
        {
            let mut pager = Pager::open(path).unwrap();
            let rows = scan_all(&mut pager, root, row_size).unwrap();
            let out: Vec<i64> = rows
                .iter()
                .map(|r| i64::from_le_bytes(r[..8].try_into().unwrap()))
                .collect();
            assert_eq!(out, keys);
        }

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn large_dataset_500_rows() {
        let path = "/tmp/mukhidb_btree_500.db";
        let mut pager = test_pager(path);
        let row_size = 264; // matches (id INTEGER, name TEXT) schema
        let keys: Vec<i64> = (0..500).collect();
        let (_, out) = insert_and_collect(&mut pager, &keys, row_size);
        assert_eq!(out.len(), 500);
        assert_eq!(out, keys);
        fs::remove_file(path).unwrap();
    }
}
