# Backlog

Known limitations, missing features, and deferred work in mukhidb. Each entry
has enough context that a fresh contributor (human or AI) can pick it up
without digging through the whole codebase.

## How to use this file

- **ID** — stable identifier. Don't renumber when items are resolved; mark them
  done in place.
- **Why it matters** — the user-visible symptom or design implication.
- **Current behavior** — exactly what happens today in the code.
- **Approach options** — one or more ways to fix it, with trade-offs.
- **Relevant files** — where in the codebase the fix belongs.
- **Acceptance criteria** — what "done" looks like, including tests.
- **Blocked by** — other backlog items or milestones, if any.

When resolving an item, change its **Status** to `Done`, add a line noting the
milestone or commit that closed it, and leave the rest of the entry intact as
historical record.

---

## Open bugs

*(none known as of 2026-04-28)*

---

## Concurrency

### CC-001 — Any open transaction blocks all other sessions

- **Status:** Open
- **Severity:** Medium (correctness: fine; performance: significant under contention)
- **Why it matters:** A read-only `BEGIN; SELECT; COMMIT` on session A prevents every other session from doing anything — not just writes, not just sessions touching the same table. In a multi-tenant or high-contention workload this serializes the whole database around the longest-running transaction.
- **Current behavior:** `Session::begin()` in `src/session.rs` acquires the global `txn_owner` Mutex, waits on `txn_cv` if another session owns it, then sets `*owner = Some(self.session_id)`. Every non-BEGIN operation first calls `wait_for_txn_clear()` which blocks until either no transaction is active or `self` owns it. Therefore *any* open transaction — read-only or not — blocks every other session's reads.
- **Why this is necessary today:** Without per-transaction snapshots, session A's dirty pages live in the shared pager cache (`Mutex<Cache>` inside `src/pager.rs`). If we let other readers through during A's transaction, `read_page()` would return A's uncommitted bytes, violating atomicity/isolation.
- **Approach options:**
  1. **MVCC with per-page versions (Milestone 10).** Each page has a chain of versions tagged with transaction IDs. Readers see the snapshot visible at their `BEGIN` time; writers create new versions. Big change: pager, btree, transaction manager. ~9-12 weekends for a full version, 4-6 for a toy.
  2. **Row-level locks + isolation downgrade.** Keep dirty-read risk but release the txn gate for reads that don't touch locked rows. Requires row-level metadata. Partial benefit, complex.
  3. **Copy-on-write at the page level.** When a transaction modifies a page, clone it; readers see the committed version until COMMIT swaps them. Cheaper to implement than full MVCC but still significant.
- **Relevant files:** `src/session.rs` (gate logic), `src/pager.rs` (page cache), `src/storage.rs` (transaction boundaries), `src/wal.rs` (commit semantics).
- **Acceptance criteria:** A new integration test in `tests/concurrency.rs` where session A runs a long `BEGIN; sleep; COMMIT` with only SELECTs, while session B runs plain SELECTs throughout — B's SELECTs should complete without blocking on A.
- **Blocked by:** Architectural decision on MVCC scope (M10).

### CC-002 — No per-table concurrency

- **Status:** Open
- **Severity:** Low (correctness: fine; performance: limits scaling)
- **Why it matters:** Two writers writing to completely unrelated tables still serialize. A workload writing `orders` and `inventory` from different services serializes as if they were one table.
- **Current behavior:** One `RwLock<Storage>` covers the whole database. `Storage` holds `HashMap<String, TableStore>`; any write to any table takes the write lock.
- **Approach options:**
  1. **Per-table RwLocks.** Change `Storage.tables` to `HashMap<String, RwLock<TableStore>>`. Writers acquire the single relevant table's lock. Complication: schema/metadata operations (CREATE, DROP) need a higher-level lock, so two-level locking. JOINs need to acquire multiple table locks — must use a consistent order (alphabetical by table name) to avoid deadlocks.
  2. **Shard by table group.** Simpler but less granular — group related tables under one lock.
- **Relevant files:** `src/storage.rs` (top-level container), `src/session.rs` (which locks to acquire for a given statement), `src/executor.rs` (multi-table statements like JOIN).
- **Acceptance criteria:** Timing test showing two concurrent writers on different tables complete in ~single-writer time.
- **Blocked by:** Arguably by CC-001 if we go MVCC instead — MVCC naturally gives per-table concurrency.

### CC-003 — Thread-per-connection scaling ceiling

- **Status:** Open
- **Severity:** Low (irrelevant below ~1000 clients)
- **Why it matters:** Each thread consumes ~2MB stack. At ~5000 concurrent connections, memory overhead becomes significant. Many web-scale systems use async I/O to avoid this.
- **Current behavior:** `src/server.rs` calls `thread::spawn` on each accept.
- **Approach options:**
  1. **Async I/O (tokio).** Biggest change — `tokio::net::TcpListener`, async fns, `tokio::sync` primitives instead of `std::sync`. All storage code needs to be `Send` across `.await` points. Adds a major dependency.
  2. **Thread pool with work queue.** Keep `std::thread` but bound the pool size; connections park on a queue when pool is saturated. Moderate change, stays zero-dep.
- **Relevant files:** `src/server.rs` (accept loop), `src/main.rs` (feature flag or subcommand variant).
- **Acceptance criteria:** Bench showing server sustains >1000 concurrent idle connections without exhausting memory.
- **Blocked by:** Requires decision on "zero deps" stance.

---

## Storage engine

### ST-001 — Row size capped at ~2KB (no overflow pages)

- **Status:** Open
- **Severity:** Medium (hard limit on TEXT column size)
- **Why it matters:** A blog post body or long description won't fit in a single row. Today an INSERT with a row >~2KB returns `Error: Row too large (X bytes). Maximum row size is Y bytes.`
- **Current behavior:** `src/storage.rs::insert` enforces `rsize <= btree::max_row_data_size()`. `btree::max_row_data_size()` is derived from the 4KB page minus the leaf header and slot directory overhead — around 2000 bytes.
- **Approach options:**
  1. **Overflow pages.** When a row exceeds an inline threshold (say 2KB), store only a fixed-size inline prefix + a pointer to an overflow chain on dedicated pages. Scan paths need to follow the chain. Moderate complexity; SQLite and most real engines do this.
  2. **Larger page size.** Raise `PAGE_SIZE` from 4KB to 16KB. Quick fix but wasteful for small rows and still a hard cap.
- **Relevant files:** `src/btree.rs` (cell format), `src/pager.rs` (page allocation — need a way to mark overflow pages), `src/row.rs` (serialize/deserialize — handle inline+overflow), `src/storage.rs::insert` (remove the hard error).
- **Acceptance criteria:** INSERT a 10KB TEXT value; SELECT it back verbatim; persists across restart; verified in `tests/integration.rs`.

### ST-002 — No DELETE statement

- **Status:** Open
- **Severity:** Medium (core SQL feature missing)
- **Why it matters:** Users can insert but never remove. For an educational DB this is a glaring gap.
- **Current behavior:** The parser doesn't recognize DELETE. `Statement` enum has no `Delete` variant.
- **Approach options:**
  1. **Tombstone deletes.** Mark cells as deleted in the leaf. Space not reclaimed until compaction. Simpler.
  2. **Physical deletes.** Actually remove the cell from the slotted page, shift slot directory. Rebalance leaf if it gets too empty. More work but no compaction debt.
  3. **Hybrid**: tombstone now, compaction later.
- **Relevant files:** `src/parser.rs` (recognize DELETE), `src/types.rs` (`Statement::Delete { table_name, where_clause }`), `src/executor.rs` (dispatch), `src/storage.rs::delete`, `src/btree.rs` (cell removal), `src/session.rs` (wire through).
- **Acceptance criteria:** `DELETE FROM t WHERE id = 5` removes the row; `SELECT * FROM t` confirms absence; persists; works inside a transaction with ROLLBACK restoring the row.

### ST-003 — No UPDATE statement

- **Status:** Open
- **Severity:** Medium (core SQL feature missing)
- **Why it matters:** Users must DELETE + INSERT to change a row. Not idiomatic SQL.
- **Current behavior:** Parser doesn't recognize UPDATE. `Statement` enum has no `Update` variant.
- **Approach options:**
  1. **Delete + insert.** Conceptually simplest given we will add DELETE anyway. Two B+Tree operations per update. Key changes are automatically handled (row moves).
  2. **In-place update.** When the new row fits in the same cell space, mutate in place. Otherwise do delete+insert. Faster for fixed-size updates.
- **Relevant files:** Same as ST-002 plus `src/storage.rs::update`.
- **Acceptance criteria:** `UPDATE t SET name = 'new' WHERE id = 1` modifies the row; persists; behaves correctly inside a transaction.
- **Blocked by:** Easier after ST-002 (reuse delete infrastructure).

### ST-004 — Deletions/rollbacks leave holes (no page compaction)

- **Status:** Deferred — will matter after ST-002 lands
- **Severity:** Low today, medium after DELETE
- **Why it matters:** Slotted pages with tombstones (or post-rollback freed space) accumulate gaps. Over time a B+Tree degrades in density, hurting scan performance.
- **Current behavior:** No compaction logic anywhere. Today this doesn't bite because there's no DELETE.
- **Approach options:**
  1. **On-demand compaction.** When inserting and free space is fragmented, rewrite the leaf with cells packed. Cheap amortized.
  2. **Background compaction.** Separate thread scans pages below a density threshold. Complicates the concurrency story.
- **Relevant files:** `src/btree.rs::insert` (trigger compaction on split path), new helper `compact_leaf`.
- **Acceptance criteria:** After deleting half a table's rows, a full scan touches roughly half the pages it did before (measurable via page-read counter added for the test).
- **Blocked by:** ST-002.

### ST-005 — No secondary indexes

- **Status:** Open
- **Severity:** Medium (performance)
- **Why it matters:** `WHERE non_primary_column = X` does a full table scan. Every secondary-key lookup is O(n).
- **Current behavior:** Only the primary key (first INTEGER column) gets a B+Tree. Other column filters go through `Storage::select_all` then a post-filter.
- **Approach options:**
  1. **Separate index trees per indexed column.** Each index is its own B+Tree mapping `indexed_value → primary_key`. Query planner picks the most selective index.
  2. **Hash indexes for equality lookups.** Cheaper to build, only useful for `=`.
- **Relevant files:** `src/parser.rs` (add `CREATE INDEX`), `src/storage.rs` (track index metadata per table), `src/btree.rs` (reuse for secondary indexes — keys can be i64 or bytes of the column value), `src/executor.rs` (query planner).
- **Acceptance criteria:** `CREATE INDEX idx_name ON t (name)` then `SELECT * FROM t WHERE name = 'x'` runs in O(log n) time demonstrably (test with 10k rows; compare timing).

### ST-006 — No ALTER TABLE

- **Status:** Open
- **Severity:** Low (workaround: drop + recreate)
- **Why it matters:** Schema evolution requires dumping and reloading data.
- **Current behavior:** Schema is set at CREATE and cannot change. No ALTER in parser.
- **Approach options:**
  1. **ALTER TABLE ADD COLUMN** (easiest). New column stored with a default value for existing rows. Need to handle reading older rows that don't have the new column.
  2. **ALTER TABLE DROP COLUMN** — trickier, need to rewrite all rows. Or tombstone the column.
  3. **Versioned schemas.** Each row tracks the schema version at insert time.
- **Relevant files:** `src/parser.rs`, `src/storage.rs` (metadata update), `src/row.rs` (per-row schema version?).

---

## SQL features

### SQL-001 — SELECT always returns all columns (`SELECT *` only)

- **Status:** Open
- **Severity:** Medium (core SQL feature missing)
- **Why it matters:** Users can't project to specific columns; always get the full row.
- **Current behavior:** Parser only accepts `SELECT * FROM ...`. `Statement::Select` has no projection list.
- **Approach options:**
  1. **Column projection list.** Parse comma-separated identifiers after SELECT; store as `Option<Vec<String>>` on `Statement::Select`. Executor filters and reorders cells before returning.
- **Relevant files:** `src/parser.rs`, `src/types.rs` (`Statement::Select`), `src/executor.rs`.
- **Acceptance criteria:** `SELECT name FROM users` returns only the `name` column in order.

### SQL-002 — No aggregate functions (COUNT, SUM, AVG, MIN, MAX)

- **Status:** Open
- **Severity:** Medium
- **Why it matters:** Can't answer `how many rows match?` without returning them all.
- **Current behavior:** Parser doesn't recognize function syntax in SELECT.
- **Approach options:**
  1. Special-case the common five as parser keywords. No `GROUP BY` yet — just global aggregate.
  2. Full expression tree for SELECT. Generalizes to arithmetic too.
- **Relevant files:** `src/parser.rs`, `src/types.rs` (richer Expr), `src/executor.rs` (aggregator).

### SQL-003 — No GROUP BY or ORDER BY

- **Status:** Open
- **Severity:** Medium
- **Why it matters:** Grouped/sorted output is basic SQL; every real query needs it.
- **Current behavior:** Output order is insertion order (actually: primary-key sort order via the B+Tree traversal). No grouping.
- **Approach options:**
  1. **ORDER BY** — post-scan sort. Easy. Then later: push sort into B+Tree traversal when the sort key matches an index.
  2. **GROUP BY** — depends on aggregate support (SQL-002). Sort-based grouping is simplest.
- **Relevant files:** `src/parser.rs`, `src/types.rs`, `src/executor.rs`.
- **Blocked by:** SQL-002 for GROUP BY.

### SQL-004 — WHERE supports only single comparisons (no AND/OR)

- **Status:** Open
- **Severity:** Medium
- **Why it matters:** `WHERE id = 1 AND name = 'x'` doesn't parse.
- **Current behavior:** `Expr` holds a single `{ column, op, value }` triple.
- **Approach options:**
  1. **Expression tree with AND/OR nodes.** Evaluated recursively. Short-circuit evaluation. Straightforward parser extension.
- **Relevant files:** `src/parser.rs`, `src/types.rs` (`Expr` becomes recursive), `src/executor.rs::evaluate`.

### SQL-005 — JOIN supports only INNER equi-join, two tables, one predicate

- **Status:** Open
- **Severity:** Low (covers the common case)
- **Why it matters:** No LEFT/RIGHT/FULL OUTER; no three-way joins; no `ON a.x = b.y AND a.z > b.w`.
- **Current behavior:** `JoinClause { right_table, left_col, right_col }` — strictly 2-table equi-join.
- **Approach options:**
  1. Add join type enum (Inner/Left/Right/Full). For outer joins, pad with nulls where appropriate.
  2. Generalize to N-way joins by chaining `JoinClause`s.
  3. Support arbitrary join predicates via Expr.
- **Relevant files:** `src/parser.rs`, `src/types.rs`, `src/executor.rs::execute_join`.

---

## Networking & security

### NET-001 — No authentication or authorization

- **Status:** Open
- **Severity:** High for any non-localhost deployment
- **Why it matters:** Anyone who can reach the port can read or modify any data.
- **Current behavior:** Server accepts every connection. No credential exchange.
- **Approach options:**
  1. **Simple password auth.** Client sends a `Login` message before queries. Server checks against a configured password. Store users/passwords in a config file.
  2. **Token-based.** Pre-shared token in a connect argument.
  3. **Role-based access.** GRANT/REVOKE style. Much later.
- **Relevant files:** `src/protocol.rs` (new message types: `Login`, `LoginOk`, `LoginFailed`), `src/server.rs` (require login before accepting queries), `src/client.rs` (send login), `src/main.rs` (accept `--password` arg).
- **Acceptance criteria:** Server started with a password refuses queries until client authenticates. Integration test covers this.

### NET-002 — No TLS

- **Status:** Open
- **Severity:** High for any non-localhost deployment
- **Why it matters:** All traffic in plaintext. Passwords (once NET-001 lands) transit in the clear.
- **Current behavior:** Plain `TcpListener`/`TcpStream` on both sides.
- **Approach options:**
  1. **rustls.** Industry-standard, safe. Adds dependency (breaks zero-deps stance but probably unavoidable for real-world use).
  2. **SSH tunnel as out-of-process solution.** Deploy behind `ssh -L`. Sidesteps the issue entirely.
- **Relevant files:** `src/server.rs`, `src/client.rs`.
- **Blocked by:** Decision on zero-deps stance.

### NET-003 — No query cancellation

- **Status:** Open
- **Severity:** Low
- **Why it matters:** Once a client sends a query, it can't take it back. A mistyped full-table scan on a big table runs to completion. Closing the connection doesn't actually stop the query on the server (it runs to completion and then fails to write the result).
- **Current behavior:** The thread handling the session doesn't check for cancellation signals during query execution.
- **Approach options:**
  1. **Cooperative cancellation.** Storage operations poll an `AtomicBool` cancellation flag. Connection close flips the flag. Needs cancellation checkpoints in B+Tree scans.
  2. **Protocol-level Cancel message** on a side channel. Used alongside (1).
- **Relevant files:** `src/session.rs` (add cancel flag), `src/btree.rs::scan_all` (check flag per page), `src/server.rs` (set flag on connection drop), `src/protocol.rs` (Cancel message).

### NET-004 — No connection timeouts or keep-alives

- **Status:** Open
- **Severity:** Low
- **Why it matters:** An idle client holds a server thread (and any open transaction) forever. A half-open TCP connection (e.g., client host crashed) is never detected.
- **Current behavior:** No timeouts set on `TcpStream`.
- **Approach options:**
  1. **Read timeout.** `stream.set_read_timeout(Some(Duration::from_secs(300)))`. Disconnect clients that go silent.
  2. **Application-level ping.** Periodic `Ping`/`Pong` messages.
- **Relevant files:** `src/server.rs`, `src/client.rs`, `src/protocol.rs`.

---

## Recovery & durability

### REC-001 — No checksumming of pages or WAL records

- **Status:** Open
- **Severity:** Medium (silent corruption risk)
- **Why it matters:** A partial disk write or bit flip could produce a silently corrupted page that nonetheless passes basic format checks. We would operate on corrupt data.
- **Current behavior:** Pages are raw bytes; WAL records have no integrity hash.
- **Approach options:**
  1. **CRC32C per page** (stored at a fixed page offset). Verify on load; refuse page if mismatch. Recompute on write.
  2. **CRC32 per WAL record.** Detect torn records during recovery; stop replay at first bad record.
- **Relevant files:** `src/pager.rs` (read/write paths), `src/wal.rs` (append/recover).
- **Acceptance criteria:** Test that manually corrupts a byte in a `.db` file and verifies the load returns a checksum error.

### REC-002 — WAL uses fixed-size records with full page data (space-inefficient)

- **Status:** Open
- **Severity:** Low (works fine; wasteful on disk)
- **Why it matters:** Each WAL record is 4109 bytes (fixed). A 1-byte change produces a 4KB+ WAL record.
- **Current behavior:** See `src/wal.rs` record format — `[type: 1][txn_id: 8][page_num: 4][page_data: 4096]`.
- **Approach options:**
  1. **Physiological logging.** Log logical operations (insert key K into page P at offset O, value V) instead of whole pages. Replay is more complex but records are tiny.
  2. **Compression.** Cheap — zstd the page bytes.
- **Relevant files:** `src/wal.rs`.

---

## Observability & tooling

### OBS-001 — No metrics or query logging

- **Status:** Open
- **Severity:** Low
- **Why it matters:** Hard to diagnose performance issues without query timings, cache hit rates, transaction counts.
- **Current behavior:** Server prints `Client connected`/`disconnected`. That's it.
- **Approach options:**
  1. **Query log file.** Every query with timing. Server-side config to enable.
  2. **In-memory metrics.** `.metrics` meta-command returns JSON of counters (queries, cache hits, txns). Useful in the REPL for learning.
- **Relevant files:** `src/server.rs`, `src/session.rs` (per-query timing), new `src/metrics.rs`.

### OBS-002 — No `EXPLAIN`

- **Status:** Open
- **Severity:** Medium (hard to reason about query plans)
- **Why it matters:** Users can't see whether a query is using an index or doing a full scan.
- **Current behavior:** No query planner separate from execution, so no plan to dump.
- **Approach options:**
  1. After adding indexes (ST-005), introduce a Plan struct and an EXPLAIN statement that prints it without executing.
- **Blocked by:** ST-005 (need indexes before EXPLAIN has anything interesting to say).

---

## How to add a new backlog item

Use the next free ID in the relevant category (CC-, ST-, SQL-, NET-, REC-, OBS-).
Copy the template:

```markdown
### XX-NNN — Short title

- **Status:** Open | In Progress | Done | Deferred
- **Severity:** Low | Medium | High
- **Why it matters:** User-visible symptom.
- **Current behavior:** Exact code behavior today, with file references.
- **Approach options:** Numbered list; include trade-offs.
- **Relevant files:** Paths to touch.
- **Acceptance criteria:** What "done" looks like; include test plan.
- **Blocked by:** (optional) other backlog items or milestones.
```
