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
