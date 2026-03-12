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
