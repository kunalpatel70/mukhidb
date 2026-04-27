# mukhidb

A SQL database built from scratch in Rust — an educational project to understand
how databases work from the ground up.

## Status

🟢 Milestone 8 complete: TCP server + client

See [PROGRESS.md](PROGRESS.md) for the full build log.

## Architecture

```
  ┌─────────────┐ TCP  ┌─────────────┐
  │   Client    │ ◄──► │   Server    │  (or local REPL — same executor)
  │(client.rs)  │      │(server.rs)  │
  └─────────────┘      └──────┬──────┘
                              │ raw string
                              ▼
                        ┌─────────────────┐
                        │     Parser      │
                        │  (parser.rs)    │
                        └────────┬────────┘
                                 │ Statement enum
                                 ▼
                        ┌─────────────────┐
                        │    Executor     │
                        │ (executor.rs)   │
                        └────────┬────────┘
                                 │ storage API calls
                                 ▼
                        ┌─────────────────┐
                        │    Storage      │
                        │  (storage.rs)   │
                        │                 │
                        │  TableStore per │
                        │  table: schema  │
                        │  + root page    │
                        └────────┬────────┘
                                 │
                    ┌────────────┼────────────┐
                    ▼            ▼            ▼
             ┌───────────┐ ┌─────────┐ ┌──────────┐
             │   B+Tree  │ │   Row   │ │  Pager   │
             │(btree.rs) │ │(row.rs) │ │(pager.rs)│
             │           │ │serialize│ │ 4KB page │
             │ insert     │ │ / deser │ │  cache   │
             │ scan_all   │ └─────────┘ │  + I/O   │
             │ dump_tree  │             └────┬─────┘
             └───────────┘                   │
                                    ┌────────┼────────┐
                                    ▼                  ▼
                              ┌──────────┐      ┌───────────┐
                              │  <table> │      │  <table>  │
                              │   .db    │      │  .db.wal  │
                              │  (disk)  │      │   (WAL)   │
                              └──────────┘      └───────────┘
```

## Getting Started

```bash
git clone https://github.com/kunalpatel70/mukhidb
cd mukhidb
cargo build --release
```

### Three ways to run it

**Local REPL (no network):**
```bash
mukhidb repl
```

**Client–server (two terminals):**
```bash
# Terminal 1 — start the server
mukhidb server

# Terminal 2 — connect a client
mukhidb connect
```

Then try:
```sql
mukhidb> CREATE TABLE users (id INTEGER, name TEXT)
mukhidb> CREATE TABLE orders (id INTEGER, user_id INTEGER)
mukhidb> INSERT INTO users VALUES (1, 'Alice')
mukhidb> INSERT INTO users VALUES (2, 'Bob')
mukhidb> INSERT INTO orders VALUES (100, 1)
mukhidb> INSERT INTO orders VALUES (101, 2)
mukhidb> SELECT * FROM users
mukhidb> SELECT * FROM users WHERE id = 1
mukhidb> SELECT * FROM users JOIN orders ON users.id = orders.user_id
mukhidb> BEGIN
mukhidb> INSERT INTO users VALUES (3, 'Charlie')
mukhidb> ROLLBACK
mukhidb> SELECT * FROM users
mukhidb> .btree users
mukhidb> .exit
```

## Roadmap

- [x] Milestone 1 — REPL + in-memory storage
- [x] Milestone 2 — Persist rows to disk (delimiter-based flat file)
- [x] Milestone 3 — B+Tree storage engine (fixed-size rows)
- [x] Milestone 4 — WHERE clause filtering (`=`, `>`, `<`)
- [x] Milestone 5 — Multiple tables + INNER JOIN
- [x] Milestone 6 — Transactions + Write-Ahead Log
- [x] Milestone 7 — Variable-size rows (slotted pages)
- [x] Milestone 8 — TCP server + client
- [ ] Milestone 9 — Concurrency — handle multiple clients simultaneously

## Learning Resources

- [cstack's SQLite clone tutorial](https://cstack.github.io/db_tutorial/)
- [Build Your Own Database from Scratch in Go](https://build-your-own.org/database)
- [ToyDB — reference implementation in Rust](https://github.com/erikgrinaker/toydb)
- [codecrafters-io/build-your-own-x](https://github.com/codecrafters-io/build-your-own-x)
