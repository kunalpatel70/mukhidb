# mukhidb

A SQL database built from scratch in Rust — an educational project to understand
how databases work from the ground up.

## Status

🟢 Milestone 1 complete: REPL + in-memory tables (CREATE TABLE, INSERT, SELECT *)

See [PROGRESS.md](PROGRESS.md) for the full build log.

## Getting Started

```bash
git clone https://github.com/YOUR_USERNAME/mukhidb
cd mukhidb
cargo run
```

Then try:
```sql
mukhidb> CREATE TABLE users (id INTEGER, name TEXT)
mukhidb> INSERT INTO users VALUES (1, 'Alice')
mukhidb> INSERT INTO users VALUES (2, 'Bob')
mukhidb> SELECT * FROM users
mukhidb> .exit
```

## Roadmap

- [x] Milestone 1 — REPL + in-memory storage
- [ ] Milestone 2 — Persist rows to disk (binary flat file)
- [ ] Milestone 3 — B+Tree storage engine
- [ ] Milestone 4 — WHERE clause filtering
- [ ] Milestone 5 — Multiple tables + JOIN
- [ ] Milestone 6 — Transactions + Write-Ahead Log
- [ ] Milestone 7 — TCP server + client
- [ ] Milestone 8 — Concurrency — handle multiple clients simultaneously

## Learning Resources

- [cstack's SQLite clone tutorial](https://cstack.github.io/db_tutorial/)
- [Build Your Own Database from Scratch in Go](https://build-your-own.org/database)
- [ToyDB — reference implementation in Rust](https://github.com/erikgrinaker/toydb)
- [codecrafters-io/build-your-own-x](https://github.com/codecrafters-io/build-your-own-x)
