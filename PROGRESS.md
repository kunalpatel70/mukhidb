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

---

## Milestone 6 — Transactions + Write-Ahead Log

**Goal:** Atomic multi-statement transactions with crash-safe durability via a Write-Ahead Log.

### What was built

- `wal.rs` — New module implementing a Write-Ahead Log:
  - Fixed-size records (4109 bytes): `[type: u8][txn_id: u64][page_num: u32][page_data: 4096 bytes]`
  - Two record types: `PAGE_WRITE` (0x01) and `COMMIT` (0x02)
  - `append_page()` — writes a dirty page to the WAL
  - `append_commit()` — writes a commit marker and fsyncs the WAL
  - `recover()` — reads the WAL and returns only records from committed transactions (uncommitted records are discarded)
  - `truncate()` — resets the WAL to zero bytes after successful flush to `.db`
  - Auto-cleanup of empty WAL files on drop
- `pager.rs` — Integrated WAL into the page I/O layer:
  - `begin()` — marks the start of an explicit transaction
  - `commit()` — WAL-write dirty pages → fsync WAL → apply to `.db` → fsync `.db` → truncate WAL
  - `rollback()` — discards dirty pages from cache (re-read from disk on next access), truncates WAL
  - `flush()` — now routes through `commit()` for auto-commit mode
  - Crash recovery on `open()` — if a WAL file exists with committed records, replays them to the `.db` before proceeding
- `storage.rs` — Added `begin()`, `commit()`, `rollback()` that fan out to all table pagers. `insert()` and `create_table()` skip auto-flush when inside an explicit transaction.  Rollback re-reads `root_page` from on-disk metadata since dirty pages are discarded.
- `parser.rs` — Parses `BEGIN`, `COMMIT`, `ROLLBACK` as new `Statement` variants.
- `executor.rs` — Handles the three new statement types, returning success/error messages.
- `repl.rs` — Updated `.help` text to list transaction commands.

### SQL supported

```sql
BEGIN
INSERT INTO users VALUES (1, 'Alice')
INSERT INTO users VALUES (2, 'Bob')
COMMIT

BEGIN
INSERT INTO users VALUES (3, 'Charlie')
ROLLBACK
```

Without an explicit `BEGIN`, every statement auto-commits (same behavior as before, now WAL-protected).

### Crash safety — the commit sequence

```
1. Write dirty pages to WAL file
2. fsync WAL                      ← WAL is durable
3. Write dirty pages to .db file
4. fsync .db                      ← .db is durable
5. Truncate WAL to 0 bytes        ← cleanup
```

Crash at any point is safe:
- Before step 2: WAL has no commit marker → recovery discards it, `.db` untouched
- Between steps 2–4: WAL has committed records → recovery replays them (idempotent)
- After step 4: everything is durable, WAL truncation is just cleanup

### Key decisions

- **Truncate-on-commit WAL** — the WAL is reset to zero after each commit. Simpler than checkpoint-based WAL (SQLite WAL mode), and sufficient for a single-client database. WAL size is bounded to one transaction's worth of dirty pages.
- **Per-table WAL files** — matches the existing per-table `.db` architecture. Each table gets a `<name>.db.wal` file. Multi-table atomicity is best-effort (each table commits independently).
- **Rollback = discard dirty pages** — since the pager caches pages in memory, rollback simply drops dirty pages and lets them be re-read from the unchanged `.db` file on next access. Also re-reads `root_page` from metadata since a B+Tree root split may have changed it.
- **Auto-commit preserved** — without `BEGIN`, each statement flushes through the WAL automatically, maintaining backward compatibility with all existing behavior.

### Tests

- 4 new unit tests in `wal.rs`: committed record recovery, uncommitted record discard, truncation clears WAL, multi-txn filtering (only committed txns recovered)
- 1 new unit test in `pager.rs`: rollback discards changes and restores original data
- 7 new integration tests: transaction commit, rollback, rollback preserves prior data across restart, commit persists across restart, double BEGIN error, COMMIT without BEGIN error, ROLLBACK without BEGIN error

### Known limitations

- **No multi-table atomicity** — `BEGIN`/`COMMIT` fans out to each table's pager independently. If the process crashes after committing table A but before committing table B, the two tables can be inconsistent. A single shared WAL would fix this.
- **No savepoints** — no nested transactions or `SAVEPOINT`/`RELEASE`.
- **No WAL size limit** — a very long transaction could produce a large WAL. A max-size check could be added.
- **CREATE TABLE inside a transaction** — creates the `.db` file immediately (can't be rolled back at the file level). The table's data inserts within the transaction are rollback-safe, but the file creation itself is not.

### What's next

- Milestone 7: Variable-size rows (overflow pages / slot-based layout)


---

## Milestone 7 — Variable-Size Rows (Slotted Pages)

**Goal:** Replace the fixed-size row layout (where every TEXT column wastes 256 bytes) with a variable-length encoding and slotted page storage, dramatically improving space efficiency and removing the 252-byte TEXT cap.

### The problem

Previously, every TEXT column was serialized as a 256-byte fixed slot (4-byte length prefix + 252 bytes content, zero-padded). A 5-character name like "Alice" occupied 256 bytes — 98% wasted. TEXT was hard-capped at 252 bytes. A table (id INTEGER, name TEXT) fit only 15 rows per 4KB leaf page.

### What was built

-  — Replaced fixed-size TEXT encoding with variable-length:
  - Old:  = 256 bytes always
  - New:  = 4 + actual length
  - Added  to compute per-row byte size
  - Removed the schema-level  function (no longer meaningful)

-  — Replaced flat fixed-cell leaf array with a **slotted page** layout:
  - **Header (11 bytes):** node_type + num_cells + next_leaf + data_start
  - **Slot directory:** grows forward from byte 11; each slot is 4 bytes (offset: u16, length: u16)
  - **Data area:** cells packed from the end of the page backward
  - Each cell: 
  - Free space = data_start - (header + num_slots x 4)
  - Insert checks free space rather than cell count
  - Split divides cells by total bytes (~50/50) rather than by count
  -  and  no longer need a  parameter — each slot carries its own length
  - Internal nodes are unchanged (they store only keys + child pointers)

-  — Updated to use the new APIs:
  - Removed  from  and  call paths
  - Added max-row-size guard on INSERT: rejects rows exceeding ~2KB (ensures at least 2 cells fit per leaf, required for splits)

### Space efficiency improvement

For a table (id INTEGER, name TEXT) with typical 5-character names:

| Metric | Before (fixed) | After (slotted) |
|--------|----------------|-----------------|
| Row size on disk | 264 bytes | 17 bytes |
| Rows per leaf | 15 | ~140 |
| TEXT capacity | 252 bytes | ~2,000 bytes |

### Key decisions

- **Slotted pages, not overflow pages** — handles the common case (short-to-medium strings) efficiently. Overflow pages deferred to a future milestone if truly unbounded text is needed.
- **Split by bytes, not count** — when a leaf is full, the split finds the point that divides total cell bytes roughly 50/50, giving balanced pages even with mixed-size rows.
- **u16 offsets in slots** — sufficient for 4KB pages (max offset 4095). Would need upgrading if page size ever exceeds 64KB.
- **Max cell size guard** — a single row cannot exceed half the usable page space (~2,030 bytes of row data). This guarantees every leaf can hold at least 2 cells, which is required for B+Tree splits to work.

### Tests

- 3 updated unit tests in : round-trip, variable sizes (short + 1000-char), empty text
- 10 unit tests in : small insert, splits, reverse order, duplicates, single row, empty tree, negative keys, persistence round-trip, variable-size rows, 500-row stress
- 4 new integration tests: long text (500 chars), mixed-length texts, long text persistence across restart, 100-row variable-text stress test
- All 43 pre-existing tests continue to pass

### Breaking change

The on-disk leaf page format changed from fixed-cell array to slotted pages. Existing .db files from Milestone 6 are incompatible and must be deleted before running the new code.

### Known limitations

- **No overflow pages** — single rows are capped at ~2KB. A blog post or long description won't fit. Overflow pages would remove this limit.
- **No compaction** — deleted rows (if DELETE existed) would leave holes in the data area. A page compaction / defragmentation step would reclaim space.
- **No DELETE or UPDATE** — still not implemented.

### What's next

- Milestone 8: TCP server + client


---

## Milestone 7 — Variable-Size Rows (Slotted Pages)

**Goal:** Replace the fixed-size row layout (where every TEXT column wastes 256 bytes) with a variable-length encoding and slotted page storage, dramatically improving space efficiency and removing the 252-byte TEXT cap.

### The problem

Previously, every TEXT column was serialized as a 256-byte fixed slot (4-byte length prefix + 252 bytes content, zero-padded). A 5-character name like "Alice" occupied 256 bytes — 98% wasted. TEXT was hard-capped at 252 bytes. A table (id INTEGER, name TEXT) fit only 15 rows per 4KB leaf page.

### What was built

- row.rs — Replaced fixed-size TEXT encoding with variable-length:
  - Old: [len: u32][content: 252 bytes, zero-padded] = 256 bytes always
  - New: [len: u32][content: len bytes] = 4 + actual length
  - Added serialized_size(row, columns) to compute per-row byte size
  - Removed the schema-level row_size(columns) function (no longer meaningful)

- btree.rs — Replaced flat fixed-cell leaf array with a **slotted page** layout:
  - **Header (11 bytes):** node_type + num_cells + next_leaf + data_start
  - **Slot directory:** grows forward from byte 11; each slot is 4 bytes (offset: u16, length: u16)
  - **Data area:** cells packed from the end of the page backward
  - Each cell: [key: i64][row_data: variable bytes]
  - Free space = data_start - (header + num_slots x 4)
  - Insert checks free space rather than cell count
  - Split divides cells by total bytes (~50/50) rather than by count
  - scan_all and dump_tree no longer need a row_size parameter — each slot carries its own length
  - Internal nodes are unchanged (they store only keys + child pointers)

- storage.rs — Updated to use the new APIs:
  - Removed row_size from select_all and dump_btree call paths
  - Added max-row-size guard on INSERT: rejects rows exceeding ~2KB (ensures at least 2 cells fit per leaf, required for splits)

### Space efficiency improvement

For a table (id INTEGER, name TEXT) with typical 5-character names:

| Metric | Before (fixed) | After (slotted) |
|--------|----------------|-----------------|
| Row size on disk | 264 bytes | 17 bytes |
| Rows per leaf | 15 | ~140 |
| TEXT capacity | 252 bytes | ~2,000 bytes |

### Key decisions

- **Slotted pages, not overflow pages** — handles the common case (short-to-medium strings) efficiently. Overflow pages deferred to a future milestone if truly unbounded text is needed.
- **Split by bytes, not count** — when a leaf is full, the split finds the point that divides total cell bytes roughly 50/50, giving balanced pages even with mixed-size rows.
- **u16 offsets in slots** — sufficient for 4KB pages (max offset 4095). Would need upgrading if page size ever exceeds 64KB.
- **Max cell size guard** — a single row cannot exceed half the usable page space (~2,030 bytes of row data). This guarantees every leaf can hold at least 2 cells, which is required for B+Tree splits to work.

### Tests

- 3 updated unit tests in row.rs: round-trip, variable sizes (short + 1000-char), empty text
- 10 unit tests in btree.rs: small insert, splits, reverse order, duplicates, single row, empty tree, negative keys, persistence round-trip, variable-size rows, 500-row stress
- 4 new integration tests: long text (500 chars), mixed-length texts, long text persistence across restart, 100-row variable-text stress test
- All 43 pre-existing tests continue to pass

### Breaking change

The on-disk leaf page format changed from fixed-cell array to slotted pages. Existing .db files from Milestone 6 are incompatible and must be deleted before running the new code.

### Known limitations

- **No overflow pages** — single rows are capped at ~2KB. A blog post or long description won't fit. Overflow pages would remove this limit.
- **No compaction** — deleted rows (if DELETE existed) would leave holes in the data area. A page compaction / defragmentation step would reclaim space.
- **No DELETE or UPDATE** — still not implemented.

### What's next

- Milestone 8: TCP server + client

---

## Milestone 8 — TCP Server + Client

**Goal:** Turn mukhidb from a single-process REPL into a client-server database. The server owns storage and listens on TCP; clients connect, send SQL, receive results.

### What was built

- `protocol.rs` — Length-prefixed typed wire protocol:
  - Frame format: `[length: u32 LE][type: u8][payload]`
  - Four message types: `Query` (client → server), `Ok` / `Error` / `Rows` (server → client)
  - `Rows` payload serializes headers and row cells as length-prefixed UTF-8 strings
  - `read_message` / `write_message` handle framing; `read_exact` under the hood prevents short-read bugs

- `server.rs` — Single-client TCP server:
  - Binds to a host/port, owns one `Storage` instance
  - Loads existing `.db` files on startup (same as REPL)
  - Accepts one client at a time (M9 will add concurrency)
  - Per session: loop `read_message` → dispatch → `write_message` until client disconnects
  - Special-cases `.btree <table>` as a server-side meta-command (needs storage access)

- `client.rs` — Interactive TCP client:
  - Connects to a server, presents a REPL identical in UX to the local REPL
  - Client-side meta-commands (`.exit`, `.help`) handled locally; everything else goes over the wire
  - Pretty-prints `Rows` responses, displays `Ok` / `Error` messages directly

- `main.rs` — Subcommand dispatch:
  - `mukhidb repl` (default) — local REPL, no network
  - `mukhidb server [--port N]` — TCP server (default port 4567)
  - `mukhidb connect [--host H] [--port N]` — TCP client

### Three running modes

1. **`mukhidb repl`** — original single-process mode preserved for offline work
2. **`mukhidb server`** — server process owning storage
3. **`mukhidb connect`** — REPL-like client that talks to the server

### Key decisions

- **Length-prefixed typed protocol** — mini-PostgreSQL shape. Clear error channel (`Ok` vs `Error` vs `Rows`) rather than string-sniffing. Extensible for future milestones.
- **Single binary, subcommands** — one build artifact, matches common CLI conventions (`git clone`, `cargo build`). Default to `repl` preserves the single-process UX.
- **One client at a time** — deliberate M8 scope. Keeps the storage layer unchanged (no locking, no Arc/Mutex). M9 adds concurrency.
- **`.btree` is server-side** — it needs access to the storage's pager. Sent over the wire as a normal `Query` message; server dispatches it specially.
- **Keep the local REPL** — zero regression risk. All 47 pre-milestone tests continue to work unchanged.

### Wire protocol specification

```
Frame layout:
  [length: u32 LE (4 bytes)] [type: u8 (1 byte)] [payload: length-1 bytes]

Message types:
  0x01  Query   — client → server, payload = UTF-8 SQL
  0x02  Ok      — server → client, payload = UTF-8 success message
  0x03  Error   — server → client, payload = UTF-8 error
  0x04  Rows    — server → client, payload:
                  [num_cols: u32][for each col: [len: u32][name bytes]]
                  [num_rows: u32][for each row: [for each cell: [len: u32][cell bytes]]]
```

### Tests

- 7 new unit tests in `protocol.rs`: query/ok/error/rows round-trip, empty rows, Unicode payload, multiple messages in one stream, unknown type tag errors
- 6 new integration tests in `tests/net.rs`: basic query, error channel, transactions over TCP, sequential client sessions, `.btree` over wire, long text over wire
- All 55 pre-existing tests continue to pass
- Total: 61 tests passing

### Known limitations

- **One client at a time** — M9 will lift this. Today a second client will block on `accept` until the first disconnects.
- **No authentication** — anyone who can reach the port can run queries.
- **No TLS** — plain TCP only. Fine for localhost; unsuitable for public networks.
- **No query cancellation** — once a query starts executing, the client can't interrupt it mid-flight.
- **No connection keep-alive / timeouts** — idle connections stay open forever.

### What's next

- Milestone 9: Concurrency — handle multiple clients simultaneously

---

## Milestone 9 — Concurrency (Multi-Client Server with Parallel Reads)

**Goal:** Lift the single-client restriction from M8. Multiple clients connect and run SQL simultaneously; `SELECT`s actually run in parallel; transactions remain atomic; a client disconnecting mid-transaction doesn't leak state.

### What was built

- `session.rs` — New module wrapping shared storage for concurrent access:
  - `Shared` — the shared state: `RwLock<Storage>`, `Mutex<Option<u64>> txn_owner`, `Condvar txn_cv`.
  - `Session` — per-client handle. Owns `Arc<Shared>`, its own `session_id` (from an `AtomicU64` counter), and an `in_transaction` flag.
  - Operations pass through a two-level gate: first `wait_for_txn_clear()` (block on `txn_cv` if another session holds a transaction), then acquire the `RwLock` in read or write mode as appropriate.
  - `Drop for Session` auto-rolls-back an open transaction so a disconnected client can't leave the txn gate locked.

- `pager.rs` — Refactored for interior-mutable reads:
  - Page cache moved behind `Mutex<Cache>`; file handle behind `Mutex<File>`; `file_length` promoted to `AtomicU64`.
  - New `read_page(&self, n) -> [u8; PAGE_SIZE]` — safe to call concurrently across threads, returns an owned 4KB copy.
  - Existing `get_page(&mut self)` / `get_page_mut(&mut self)` kept their signatures but use `Mutex::get_mut()` internally (lock-free when exclusive access is guaranteed).

- `btree.rs` — `scan_all` and `dump_tree` changed from `&mut Pager` to `&Pager` and now call `read_page()` for their reads. Write paths (`insert`, splits) unchanged.

- `storage.rs` — `Storage::select_all` and `Storage::dump_btree` now take `&self` (using `HashMap::get` instead of `get_mut`). Removed the `in_transaction` flag — transaction state lives in `Session` now; auto-flush is gated by `pager.in_transaction()` directly.

- `server.rs` — Each `accept()` spawns a thread (`std::thread::spawn`) that owns one `Session`. Server's main thread just accepts and dispatches.

- `repl.rs` — Still a single local session, but now uses the same `Session` / `Shared` plumbing as the server for uniformity.

### Concurrency model

All sessions share one `Arc<Shared>`. Operations coordinate through two primitives with clear roles:

| Primitive | Role |
|---|---|
| `RwLock<Storage>` | Serializes data access — many readers OR one writer at a time. Held briefly per statement. |
| `Mutex<Option<u64>> txn_owner` + `Condvar txn_cv` | Serializes transactions — at most one session can be inside `BEGIN..COMMIT` globally. |

Operation flow:

| Op | Gate check | Lock |
|---|---|---|
| `SELECT` (no txn) | wait while another session owns the txn | `storage.read()` briefly — **readers run in parallel** |
| `INSERT` / `CREATE` (no txn) | wait while another session owns the txn | `storage.write()` briefly |
| `BEGIN` | wait while `txn_owner` is Some, then claim it | `storage.write()` briefly for `pager.begin()` |
| `INSERT` / `SELECT` during my txn | pass (I own it) | `storage.write()` briefly |
| `COMMIT` / `ROLLBACK` | I own it | `storage.write()` for pager op, then clear `txn_owner`, `notify_all` |
| Client disconnect with open txn | `Drop for Session` issues auto-rollback | same as `ROLLBACK` |

### Key decisions

- **RwLock over Mutex** — the common database workload is read-heavy. Getting concurrent-read performance is the whole point. The cost is one more primitive to understand; the benefit is measurable parallelism.
- **Transactions globally serialized via a separate gate** — `RwLockWriteGuard` from `std` can't be stored across statement boundaries (lifetime bound to the RwLock). Instead, the txn_owner Mutex + Condvar holds the coarse "who has the txn" state, while the RwLock serves short per-statement access.
- **Interior-mutable page cache** — the pager kept `&mut self` for write APIs (to preserve borrowed-reference ergonomics in btree write paths) while adding a `&self` read path that locks its internal `Mutex<Cache>` briefly. This avoided a massive rewrite of `btree.rs`.
- **Zero new dependencies** — everything uses `std::sync` (`Mutex`, `RwLock`, `Condvar`, `Arc`) and `std::sync::atomic`. No tokio, no parking_lot, no crossbeam.
- **Drop-based cleanup** — instead of a manual "on disconnect" hook in the server, `Drop for Session` guarantees rollback regardless of how the session ends (normal exit, panic, TCP reset).
- **Thread-per-connection over async** — keeps the code linear and easy to read. `std::thread::spawn` is fine for the single-digit-clients-at-a-time workload we care about. Async would be a bigger refactor (executor choice, Pin futures) for no benefit at this scale.

### Architecture

```
           Client           Client           Client
             │                │                │
             ▼                ▼                ▼
           Session          Session          Session
             │                │                │
             └──────┐         │         ┌─────┘
                    ▼         ▼         ▼
                   ┌─────────────────────┐
                   │  Arc<Shared>        │
                   │  ├─ RwLock<Storage> │
                   │  ├─ Mutex<txn_owner>│
                   │  └─ Condvar<txn_cv> │
                   └──────────┬──────────┘
                              ▼
                         Storage / Pager
```

### Tests

- 6 new integration tests in `tests/concurrency.rs`:
  - `concurrent_inserts_all_persist` — 5 threads × 20 inserts each; verifies all 100 rows are present with correct data
  - `transaction_blocks_other_write` — A BEGINs and holds, B's INSERT must wait ≥ 200ms then proceed after A commits
  - `disconnect_during_txn_releases_gate` — A BEGINs and disconnects; B can BEGIN immediately (A's rows rolled back)
  - `second_begin_waits_for_first` — two concurrent BEGINs serialize correctly
  - `concurrent_reads_return_consistent_data` — 8 concurrent readers all get consistent output
  - `concurrent_reads_run_in_parallel` — timing-based proof: 8 readers take ~37ms vs ~98ms serialized (~2.6× speedup)
- 1 new unit test in `pager.rs`: `read_page_works_via_shared_ref` — verifies the new `read_page(&self)` path
- All 62 pre-existing tests continue to pass
- Total: 69 tests passing

### Measured parallelism (proof, not promise)

The `concurrent_reads_run_in_parallel` test measures real elapsed wall time:

- **1 reader**: ~12ms
- **8 readers in parallel**: ~37ms
- **8 readers serialized (what a Mutex would give)**: ~98ms

That's a **~2.6× speedup** on a modest workload. Larger tables and more cache hits would scale further.

### Known limitations

- **One transaction at a time, globally** — while session A is inside `BEGIN..COMMIT`, all other sessions block on the `txn_owner` gate for any operation, including reads. This is required for correctness without MVCC: A's uncommitted pages live in the shared pager cache, so a concurrent reader would see A's dirty data and violate isolation. MVCC (M10) removes this constraint via per-transaction snapshots.
- **No deadlock detection needed yet** — we use a strict lock order (txn_owner → RwLock) so deadlocks are impossible by construction. MVCC would introduce more lock interactions and may need detection.
- **No per-table concurrency** — the RwLock covers the whole database. Two writers to unrelated tables still serialize. Per-table locks would fix this but complicate the schema/metadata locking story.
- **Thread-per-connection doesn't scale past ~thousands of clients** — each thread costs ~2MB of stack. For tens of thousands of concurrent connections, async I/O would be required.

### What's next

- Milestone 10: MVCC — multi-writer transactions + snapshot isolation (no reader-writer blocking)
