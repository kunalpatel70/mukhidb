//! Per-client session that coordinates access to the shared Storage.
//!
//! # Milestone 9 concurrency model
//!
//! The database lives inside `Arc<Shared>`, where:
//! - `Shared.storage: RwLock<Storage>` — holds the actual database.
//! - `Shared.txn_owner: Mutex<Option<u64>>` — session id with the active txn.
//! - `Shared.txn_cv: Condvar` — signalled when a transaction ends.
//!
//! ## Operation flow
//!
//! All non-BEGIN operations first pass through `wait_for_txn_clear()`:
//!   - If no transaction is active, proceed immediately.
//!   - If *our* session owns the active transaction, proceed.
//!   - Otherwise, block on `txn_cv` until the active transaction ends.
//!
//! `BEGIN` waits until `txn_owner` is `None`, then claims it.
//! `COMMIT` / `ROLLBACK` releases `txn_owner` and notifies waiters.
//!
//! ## Concurrent reads
//!
//! `SELECT` and `.btree` take the RwLock in *read* mode when no transaction
//! is active, so multiple readers truly run in parallel. The pager\'s page
//! cache is behind its own `Mutex`, so cache hits/misses serialize briefly
//! but the bulk of the scan runs concurrently across threads.
//!
//! Inside a transaction we take the write lock for all ops (reads included)
//! so the session observes its own uncommitted pages consistently.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

use crate::storage::Storage;
use crate::types::{Column, Row, Value};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// State shared across all concurrent sessions.
pub struct Shared {
    storage: RwLock<Storage>,
    txn_owner: Mutex<Option<u64>>,
    txn_cv: Condvar,
}

impl Shared {
    pub fn new(storage: Storage) -> Arc<Self> {
        Arc::new(Shared {
            storage: RwLock::new(storage),
            txn_owner: Mutex::new(None),
            txn_cv: Condvar::new(),
        })
    }

    fn with_write<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Storage) -> R,
    {
        let mut guard = self.storage.write().expect("storage RwLock poisoned");
        f(&mut *guard)
    }

    fn with_read<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Storage) -> R,
    {
        let guard = self.storage.read().expect("storage RwLock poisoned");
        f(&*guard)
    }
}

/// Handle for one client of the database. Each TCP connection or REPL
/// instance owns one Session.
pub struct Session {
    shared: Arc<Shared>,
    session_id: u64,
    in_transaction: bool,
}

impl Session {
    pub fn new(shared: Arc<Shared>) -> Self {
        Session {
            shared,
            session_id: NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst),
            in_transaction: false,
        }
    }

    /// Block until either no transaction is active, or we are the active
    /// transaction owner. This is the gate that non-BEGIN ops pass through.
    fn wait_for_txn_clear(&self) {
        let mut owner = self.shared.txn_owner.lock().expect("txn_owner poisoned");
        while let Some(oid) = *owner {
            if oid == self.session_id {
                return;
            }
            owner = self.shared.txn_cv.wait(owner).expect("txn_cv poisoned");
        }
    }

    /// Clear the txn_owner slot and notify all waiters.
    fn release_txn_gate(&self) {
        let mut owner = self.shared.txn_owner.lock().expect("txn_owner poisoned");
        if *owner == Some(self.session_id) {
            *owner = None;
        }
        self.shared.txn_cv.notify_all();
    }

    // --- User-facing operations ---

    pub fn create_table(&mut self, name: String, columns: Vec<Column>) -> Result<(), String> {
        self.wait_for_txn_clear();
        self.shared.with_write(|s| s.create_table(name, columns))
    }

    pub fn insert(&mut self, table: &str, values: Vec<Value>) -> Result<(), String> {
        self.wait_for_txn_clear();
        self.shared.with_write(|s| s.insert(table, values))
    }

    pub fn select_all(&mut self, table: &str) -> Result<(Vec<String>, Vec<Row>), String> {
        self.wait_for_txn_clear();
        if self.in_transaction {
            // Inside our own txn we already hold the txn_owner gate, so no
            // other session can run. Take the write lock for consistency
            // with our own INSERTs in the same txn.
            self.shared.with_write(|s| s.select_all(table))
        } else {
            // Concurrent readers welcome — this is the hot path.
            self.shared.with_read(|s| s.select_all(table))
        }
    }

    pub fn dump_btree(&mut self, table: &str) -> Result<String, String> {
        self.wait_for_txn_clear();
        if self.in_transaction {
            self.shared.with_write(|s| s.dump_btree(table))
        } else {
            self.shared.with_read(|s| s.dump_btree(table))
        }
    }

    pub fn begin(&mut self) -> Result<(), String> {
        if self.in_transaction {
            return Err("Already in a transaction.".to_string());
        }

        let mut owner = self.shared.txn_owner.lock().expect("txn_owner poisoned");
        while owner.is_some() {
            owner = self.shared.txn_cv.wait(owner).expect("txn_cv poisoned");
        }
        *owner = Some(self.session_id);
        drop(owner);

        let result = self.shared.with_write(|s| s.begin());
        if result.is_err() {
            self.release_txn_gate();
            return result;
        }
        self.in_transaction = true;
        Ok(())
    }

    pub fn commit(&mut self) -> Result<(), String> {
        if !self.in_transaction {
            return Err("No active transaction.".to_string());
        }
        let result = self.shared.with_write(|s| s.commit());
        self.in_transaction = false;
        self.release_txn_gate();
        result
    }

    pub fn rollback(&mut self) -> Result<(), String> {
        if !self.in_transaction {
            return Err("No active transaction.".to_string());
        }
        let result = self.shared.with_write(|s| s.rollback());
        self.in_transaction = false;
        self.release_txn_gate();
        result
    }
}

impl Drop for Session {
    /// If the client disconnects with an open transaction, roll it back so
    /// state does not leak and the txn gate is released.
    fn drop(&mut self) {
        if self.in_transaction {
            let _ = self.shared.with_write(|s| s.rollback());
            self.in_transaction = false;
            self.release_txn_gate();
        }
    }
}
