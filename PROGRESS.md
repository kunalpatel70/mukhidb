# Build Log

## Milestone 1 — REPL + In-Memory Storage

**Goal:** A working interactive shell that can create tables, insert rows, and query them.

### What was built

- `types.rs` — Core data model: `DataType`, `Value`, `Column`, `Row`, `Table`
- `parser.rs` — Parses raw SQL strings into a `Statement` enum (CREATE TABLE, INSERT, SELECT)
- `storage.rs` — In-memory `Storage` struct backed by a `HashMap<String, Table>`
- `executor.rs` — Matches a `Statement` to a storage operation, returns a result
- `repl.rs` — Read-Eval-Print loop: prompts, reads input, dispatches, pretty-prints output

### SQL supported

```sql
CREATE TABLE users (id INTEGER, name TEXT)
INSERT INTO users VALUES (1, 'Alice')
SELECT * FROM users
```

### Key decisions

- Using an enum (`Value`) rather than dynamic typing to represent cell values — keeps things explicit and Rust-idiomatic.
- Parser is hand-rolled (no crate) to understand the mechanics before reaching for a library.
- Storage is purely in-memory for now — everything is lost on exit. Milestone 2 fixes this.

### What's missing / next

- No persistence — data is gone when you quit
- No WHERE filtering
- Parser is fragile — doesn't handle edge cases or errors gracefully
- No data type validation on INSERT

---

## Milestone 2 — Persist Rows to Disk

**Goal:** Survive restarts — tables and their data are saved to disk and reloaded automatically.

### What was built

- `disk.rs` — New module with two public functions:
  - `save_row(table_name, row)` — appends one encoded row to `<table_name>.db`
  - `load_table(path)` — reads a `.db` file and reconstructs a `Table` (schema + rows)
  - `create_file(table_name, columns)` — creates a new `.db` file with a schema header
  - `find_db_files()` — scans the current directory for `.db` files
- `storage.rs` — Wired `create_file()` into `create_table()` and `save_row()` into `insert()`
- `repl.rs` — On startup, scans for `.db` files and restores tables into memory

### File format

One file per table, named `<table_name>.db`. Human-readable with `cat`.

- **Line 1 (header):** schema encoded as `col:<name>:<type>` separated by `|`
- **Lines 2+:** data rows encoded as `<type_tag>:<data>` separated by `|`

Type tags: `int`, `text`, `null`

Example (`users.db`):
```
col:id:integer|col:name:text
int:1|text:Alice
int:2|text:Bob
```

### Known limitation

- TEXT values containing `|` are rejected on INSERT because `|` is the column delimiter. This is a deliberate trade-off for format simplicity.
- `.db` files are read from the current working directory (`fs::read_dir("."`)), so data location depends on where you run the binary. A fixed data directory (e.g. `~/.mukhidb/`) would be more robust.

### What's next

- Milestone 3: B+Tree storage engine — replace the flat file with a page-based tree structure for efficient lookups

---

## Milestone 3 — B+Tree Storage Engine

**Goal:** Replace the delimiter-based flat file with a B+Tree backed by fixed-size pages, giving O(log n) key lookups and sorted iteration.

### What was built

- `pager.rs` — Fixed-size 4096-byte page I/O layer with lazy-loaded cache and dirty-page tracking. Handles read, write, allocate, and flush.
- `row.rs` — Binary row serialization. `INTEGER` = 8 bytes little-endian, `TEXT` = 256 bytes fixed (4-byte length prefix + 252 bytes content, zero-padded).
- `btree.rs` — B+Tree with leaf and internal node types. Supports:
  - Insert with sorted key placement
  - Leaf splitting when a leaf is full
  - Internal node splitting when propagation fills a parent
  - Root splitting (tree grows a new level)
  - Full sequential scan via leaf sibling chain (`next_leaf` pointers)
  - `dump_tree()` for human-readable tree visualisation
- `storage.rs` — Replaced `HashMap<String, Table>` (in-memory `Vec<Row>`) with `HashMap<String, TableStore>` where each `TableStore` holds a `Pager`, column schema, and root page number. No full table copy in memory.
- `disk.rs` — Slimmed to just `find_db_files()`. All I/O now goes through the pager.
- `repl.rs` — Startup opens `.db` files via `storage.open_table()`. Added `.btree <table>` meta-command.

### On-disk format

One binary file per table (`<name>.db`), composed of 4096-byte pages:

- **Page 0 — Metadata:** root page number (u32), column count (u32), column definitions (name length + name bytes + type tag)
- **Page 1+ — B+Tree nodes:**
  - Leaf: `[type=0x01][num_cells: u32][next_leaf: u32][cells: (key: i64, row_data)...]`
  - Internal: `[type=0x02][num_keys: u32][right_child: u32][entries: (child: u32, key: i64)...]`

### Row layout (fixed-size)

| Type    | Size on disk | Encoding                              |
|---------|-------------|---------------------------------------|
| INTEGER | 8 bytes     | i64 little-endian                     |
| TEXT    | 256 bytes   | u32 length prefix + up to 252 bytes UTF-8, zero-padded |

For a table `(id INTEGER, name TEXT)`, each row is 264 bytes.

### Key decisions

- **Fixed-size rows** — simplifies cell offset arithmetic and splitting. Variable-size rows are deferred to a future milestone.
- **First INTEGER column as B+Tree key** — rows are stored sorted by this key. Tables without an INTEGER column use key 0 (degenerate but functional).
- **Flush after every INSERT** — simple durability guarantee at the cost of write throughput. Batched/deferred flushing is a future optimisation.
- **Page cache with dirty tracking** — only modified pages are written back on flush. Pages are loaded lazily on first access.

### Meta-commands added

- `.btree <table>` — prints the tree structure showing internal nodes, separator keys, and leaf nodes with their cell keys

### Tests

- 11 unit tests: pager round-trip, row serialization, B+Tree insert (small, large, reverse, duplicates, negatives, single row, empty tree), persistence round-trip, 500-row stress test
- 6 integration tests: create/insert/select, persistence across restarts, sorted output, type mismatch error, duplicate table error, 200-row stress test

### Known limitations

- TEXT values are capped at 252 bytes (fixed-size slot)
- No DELETE or UPDATE
- No WHERE clause (Milestone 4)
- `.db` files from Milestone 2 are incompatible — must be deleted before upgrading

### What's next

- Milestone 4: WHERE clause filtering — leverage the B+Tree key for efficient lookups

---

## Milestone 4 — WHERE Clause Filtering

**Goal:** Support filtering rows with `SELECT * FROM <table> WHERE <col> <op> <val>`, with `=`, `>`, and `<` operators.

### What was built

- `types.rs` — Added `CompOp` enum (Eq, Lt, Gt) and `Expr` struct (column + op + value) to represent a single WHERE condition.
- `parser.rs` — Extended `Statement::Select` with an `Option<Expr>` field. After parsing `FROM <table>`, checks for a `WHERE` keyword and parses `<column> <op> <value>`.
- `executor.rs` — Added `evaluate()` function that resolves a column name to its index, then compares the row's value against the expression. Filtering is applied after `select_all` returns all rows (full scan + filter).
- `repl.rs` — Updated `.help` text to show WHERE syntax.

### SQL supported

```sql
SELECT * FROM users WHERE id = 2
SELECT * FROM users WHERE name = 'Alice'
SELECT * FROM users WHERE id > 5
SELECT * FROM users WHERE id < 10
```

### Key decisions

- **Filter in executor, not storage** — storage returns all rows, executor applies the predicate. Keeps the storage layer simple. Pushing the filter down (and using B+Tree key lookups for key-column equality) is a future optimisation.
- **Single condition only** — no AND/OR compound expressions yet. One operator, one column, one value.
- **Case-insensitive column matching** — `WHERE Name = 'Alice'` works the same as `WHERE name = 'Alice'`.

### Tests

- 7 new integration tests: equality on integer, equality on text, greater-than, less-than, no-match (0 rows), select-all regression, and WHERE across a B+Tree leaf split (20 rows)

### Known limitations

- **No B+Tree key optimisation** — WHERE on the key column still does a full leaf scan. A point lookup / range scan in `btree.rs` would avoid reading every leaf.
- **No compound conditions** — no `AND`, `OR`, or parenthesised expressions.
- **No `>=`, `<=`, `!=` operators** — only `=`, `>`, `<` for now.
- **Duplicate keys are not rejected** — inserting two rows with the same key (e.g. `INSERT INTO t VALUES (1, 'A')` twice) stores both. This differs from SQLite, which either errors on PRIMARY KEY conflict or uses an internal auto-increment rowid so user-column duplicates don't collide in the tree. A future milestone should either enforce unique keys, support upsert, or introduce an internal rowid.

### What's next

- Milestone 5: Multiple tables + JOIN

---

## Milestone 5 — Multiple Tables + JOIN

**Goal:** Support INNER JOIN between two tables with an ON equality condition, and WHERE filtering on joined results.

### What was built

- `types.rs` — Added `JoinClause` struct (right_table, left_col, right_col).
- `parser.rs` — Extended `Statement::Select` with `join: Option<JoinClause>`. Parses `JOIN <table> ON <col> = <col>` between FROM and WHERE. Supports both `table.col` and bare `col` syntax in the ON clause.
- `executor.rs` — Refactored SELECT execution into `execute_select` (single table) and `execute_join` (two tables). The join implementation:
  - Fetches both tables via `select_all`
  - Resolves join column indices (handles `table.col` dot notation)
  - Nested loop: for each left row × right row, emits combined row if join columns match
  - Prefixes all output headers with `table_name.` to avoid column name ambiguity
  - Applies WHERE filter on the joined result
- `repl.rs` — Updated `.help` text to show JOIN syntax.

### SQL supported

```sql
SELECT * FROM users JOIN orders ON users.id = orders.user_id
SELECT * FROM users JOIN orders ON id = user_id
SELECT * FROM users JOIN orders ON users.id = orders.user_id WHERE users.name = 'Alice'
```

### Key decisions

- **Nested loop join** — O(n × m), simplest correct implementation. Hash join or sort-merge join are future optimisations.
- **Always prefix headers** — joined output uses `table.column` for all columns (e.g. `users.id | users.name | orders.id | orders.user_id`). Avoids ambiguity without extra logic.
- **Join in executor, not storage** — the executor calls `select_all` on both tables and does the join in memory. Keeps the storage layer unchanged.
- **Multiple tables already worked** — each CREATE TABLE creates a separate `.db` file with its own TableStore. Storage holds them all in a HashMap. No changes needed for multi-table support itself.

### Tests

- 4 new integration tests: basic join (3 matched rows across 2 tables), join + WHERE filter, join with no matches (0 rows), bare column names in ON clause

### Known limitations

- **INNER JOIN only** — no LEFT, RIGHT, or FULL OUTER JOIN.
- **Two tables only** — no chained joins (`a JOIN b JOIN c`).
- **Equality join only** — ON clause only supports `=`, not `>`, `<`, or expressions.
- **No SELECT column list** — always `SELECT *`, no way to pick specific columns.
- **Nested loop performance** — O(n × m) with no index usage. A hash join or leveraging the B+Tree key for the join column would be significantly faster for large tables.

### What's next

- Milestone 6: Transactions + Write-Ahead Log
