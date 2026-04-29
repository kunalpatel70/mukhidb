# mukhidb

A SQL database built from scratch in Rust — an educational project to understand
how databases work from the ground up.

## Status

🟢 Milestone 9 complete: concurrent multi-client server with parallel reads

See [PROGRESS.md](PROGRESS.md) for the full build log and [BACKLOG.md](BACKLOG.md) for known limitations and planned work.

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
                                 │ session API calls
                                 ▼
                         ┌─────────────────────────┐
                         │  Session (session.rs)   │
                         │  per-client handle.     │
                         │  Holds Arc<Shared>.     │
                         │  Gates ops on txn_owner │
                         │  + acquires RwLock.     │
                         └────────────┬────────────┘
                                     │
                         ┌────────────▼────────────┐
                         │  Shared: RwLock<Storage>│
                         │        + txn_owner      │
                         │        + txn_cv         │
                         └────────────┬────────────┘
                                     │
                         ┌────────────▼────────────┐
                         │        Storage          │
                         │     (storage.rs)        │
                         │  TableStore per table:  │
                         │  schema + root page     │
                        └────────┬────────────────┘
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

## Concurrency guarantees

Multiple clients can connect simultaneously — the server spawns a thread per connection. Access to the shared database is coordinated by two primitives:

- `RwLock<Storage>` — per-statement data access.
- `txn_owner` Mutex + Condvar — at most one transaction active globally.

Concretely, you can rely on:

- **Multiple readers run in parallel.** A plain `SELECT` (outside a transaction) takes the RwLock in read mode. Many readers run concurrently; two threads scanning the same page briefly serialize on the pager's cache Mutex (microseconds per 4KB fetch), but all row processing runs fully in parallel. Measured: 8 concurrent readers ~2.6× faster than they would be with a plain Mutex.
- **Writes serialize.** A non-transaction `INSERT` or `CREATE TABLE` takes the RwLock in write mode, so only one writer runs at a time and active readers finish first.
- **Any open transaction blocks all other sessions.** `BEGIN` claims the `txn_owner` gate; every other session (reads and writes alike) waits on a Condvar until the transaction session calls `COMMIT` or `ROLLBACK` — or disconnects, which auto-rolls-back via `Drop`. This also applies to a purely read-only `BEGIN; SELECT; COMMIT` — it blocks others for the duration. Removing this coarse-grained locking is Milestone 10 (MVCC).
- **No deadlocks by construction.** Strict lock order: always `txn_owner` first, then `RwLock<Storage>`. No cycles possible.

What this means in practice:

| Scenario | Blocked? |
|---|---|
| Two SELECTs, neither in a transaction | No — run in parallel |
| SELECT while another session's INSERT is running | Yes — reader waits for writer briefly |
| INSERT while other sessions are SELECTing | Yes — writer waits for readers to finish |
| SELECT while another session is in BEGIN..COMMIT | Yes — blocked until that transaction ends |
| Your own SELECT inside your own transaction | No (you already hold the txn gate) |

## Roadmap

- [x] Milestone 1 — REPL + in-memory storage
- [x] Milestone 2 — Persist rows to disk (delimiter-based flat file)
- [x] Milestone 3 — B+Tree storage engine (fixed-size rows)
- [x] Milestone 4 — WHERE clause filtering (`=`, `>`, `<`)
- [x] Milestone 5 — Multiple tables + INNER JOIN
- [x] Milestone 6 — Transactions + Write-Ahead Log
- [x] Milestone 7 — Variable-size rows (slotted pages)
- [x] Milestone 8 — TCP server + client
- [x] Milestone 9 — Concurrency — multi-client server with `Arc<RwLock<Storage>>`, transaction-owner gate, and parallel reads via interior-mutable page cache
- [ ] Milestone 10 — MVCC — multi-writer transactions + snapshot isolation (no reader-writer blocking)
- [ ] Milestone 11 — Secondary indexes (`CREATE INDEX`) — separate B+Tree per index, cost-based lookup
- [ ] Milestone 12 — `DELETE` + `UPDATE` + page compaction — tombstones, in-place update, vacuum
- [ ] Milestone 13 — Query planner + `EXPLAIN` — statistics, selectivity, plan nodes, join ordering
- [ ] Milestone 14 — Richer SQL — projections, `GROUP BY`, `ORDER BY`, aggregates, compound `WHERE`, proper Pratt parser
- [ ] Milestone 15 — Checksums + crash recovery hardening — CRC32C on pages, torn-write detection, kill-mid-operation test harness
- [ ] Milestone 16 — Per-table locking — break free from the global `RwLock`, introduce 2PL + deadlock detection
- [ ] Milestone 17 — Replication — single-leader async replicas via WAL shipping, reads from followers
- [ ] Milestone 18 — Raft consensus — leader election, quorum writes, linearizable reads across 3+ nodes
- [ ] Milestone 19 — Sharding — range / hash partitioning, scatter-gather queries, rebalancing
- [ ] Milestone 20 — Distributed transactions — 2PC (and/or Calvin / deterministic ordering) across shards

## Learning Resources

- [cstack's SQLite clone tutorial](https://cstack.github.io/db_tutorial/)
- [Build Your Own Database from Scratch in Go](https://build-your-own.org/database)
- [ToyDB — reference implementation in Rust](https://github.com/erikgrinaker/toydb)
- [codecrafters-io/build-your-own-x](https://github.com/codecrafters-io/build-your-own-x)
