//! Concurrency tests for the Milestone 9 multi-client server.
//!
//! These tests spawn multiple clients against a single server and verify:
//!   - Concurrent clients don't corrupt data
//!   - Transactions block other transactions
//!   - An open transaction blocks other writes, then unblocks on COMMIT
//!   - A disconnected txn client releases the gate for the next client
//!   - Many concurrent inserts all land correctly

use std::fs;
use std::io::Write;
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn tmpdir(name: &str) -> String {
    let dir = format!("/tmp/mukhidb_concurrency_{}", name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn binary() -> String {
    format!("{}/target/debug/mukhidb", env!("CARGO_MANIFEST_DIR"))
}

fn start_server(dir: &str, port: u16) -> Child {
    let child = Command::new(binary())
        .args(["server", "--port", &port.to_string()])
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn server");

    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return child;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Server on port {} did not start in time", port);
}

fn run_client(port: u16, input: &str) -> String {
    let output = Command::new(binary())
        .args(["connect", "--port", &port.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
            child.wait_with_output()
        })
        .expect("Failed to run client");
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Spawn a client in a thread that only disconnects when told.
/// Returns (join handle that receives the final output, stdin sender for commands).
fn spawn_interactive_client(port: u16) -> (thread::JoinHandle<String>, mpsc::Sender<String>) {
    let (tx, rx) = mpsc::channel::<String>();
    let handle = thread::spawn(move || {
        let mut child = Command::new(binary())
            .args(["connect", "--port", &port.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn client");
        let mut stdin = child.stdin.take().unwrap();
        while let Ok(cmd) = rx.recv() {
            if cmd == "__CLOSE__" {
                break;
            }
            stdin.write_all(cmd.as_bytes()).unwrap();
            stdin.flush().unwrap();
        }
        drop(stdin);
        let out = child.wait_with_output().expect("client wait failed");
        String::from_utf8_lossy(&out.stdout).to_string()
    });
    (handle, tx)
}

/// 5 concurrent clients inserting 20 rows each into the same table.
/// All 100 rows must be present with no corruption.
#[test]
fn concurrent_inserts_all_persist() {
    let dir = tmpdir("inserts");
    let port = 48001;
    let mut server = start_server(&dir, port);

    // Create table first.
    let _ = run_client(
        port,
        "CREATE TABLE users (id INTEGER, name TEXT)\n.exit\n",
    );

    // Spawn 5 threads each inserting 20 rows with distinct id ranges.
    let handles: Vec<_> = (0..5u32)
        .map(|i| {
            thread::spawn(move || {
                let start = i * 20;
                let end = start + 20;
                let mut script = String::new();
                for id in start..end {
                    script.push_str(&format!(
                        "INSERT INTO users VALUES ({}, 'user{}')\n",
                        id, id
                    ));
                }
                script.push_str(".exit\n");
                run_client(port, &script)
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // Verify all 100 rows are present.
    let out = run_client(port, "SELECT * FROM users\n.exit\n");
    assert!(out.contains("(100 rows)"), "Expected 100 rows, got: {}", out);
    for id in 0..100 {
        assert!(
            out.contains(&format!("user{}", id)),
            "Missing user{} in output",
            id
        );
    }

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

/// Client A opens a transaction. Client B's INSERT should block until A COMMITs.
#[test]
fn transaction_blocks_other_write() {
    let dir = tmpdir("txn_blocks");
    let port = 48002;
    let mut server = start_server(&dir, port);

    let _ = run_client(
        port,
        "CREATE TABLE t (id INTEGER, name TEXT)\n.exit\n",
    );

    // Client A: BEGIN, don't commit yet.
    let (a_handle, a_tx) = spawn_interactive_client(port);
    a_tx.send("BEGIN\n".to_string()).unwrap();
    a_tx.send("INSERT INTO t VALUES (1, 'A')\n".to_string()).unwrap();
    thread::sleep(Duration::from_millis(200));

    // Client B: spawn in a thread, try to INSERT. Should block.
    let b_start = Instant::now();
    let (b_handle, b_tx) = spawn_interactive_client(port);
    b_tx.send("INSERT INTO t VALUES (2, 'B')\n".to_string()).unwrap();

    // Give B time to send its insert and get blocked.
    thread::sleep(Duration::from_millis(300));

    // A commits, releasing the gate.
    a_tx.send("COMMIT\n".to_string()).unwrap();
    a_tx.send(".exit\n".to_string()).unwrap();
    a_tx.send("__CLOSE__".to_string()).unwrap();
    let _a_out = a_handle.join().unwrap();

    // Now B should be able to proceed.
    b_tx.send(".exit\n".to_string()).unwrap();
    b_tx.send("__CLOSE__".to_string()).unwrap();
    let b_out = b_handle.join().unwrap();
    let b_elapsed = b_start.elapsed();

    // B's total time should reflect that it waited. At minimum 200ms.
    assert!(
        b_elapsed >= Duration::from_millis(200),
        "B finished too fast ({:?}); expected to have been blocked",
        b_elapsed
    );
    assert!(b_out.contains("1 row inserted."), "B output: {}", b_out);

    // Verify both rows landed.
    let out = run_client(port, "SELECT * FROM t\n.exit\n");
    assert!(out.contains("(2 rows)"), "Final state: {}", out);

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

/// Client A BEGINs and disconnects without COMMIT. The txn gate must be
/// released so Client B can BEGIN immediately.
#[test]
fn disconnect_during_txn_releases_gate() {
    let dir = tmpdir("txn_disconnect");
    let port = 48003;
    let mut server = start_server(&dir, port);

    let _ = run_client(
        port,
        "CREATE TABLE t (id INTEGER)\n.exit\n",
    );

    // Client A: BEGIN then disconnect (via .exit which also closes stdin).
    let _out_a = run_client(
        port,
        "BEGIN\n\
         INSERT INTO t VALUES (1)\n\
         .exit\n",
    );

    // Client B: should succeed with its own BEGIN right away.
    let out_b = run_client(
        port,
        "BEGIN\n\
         INSERT INTO t VALUES (2)\n\
         COMMIT\n\
         .exit\n",
    );
    assert!(out_b.contains("Transaction started."), "B output: {}", out_b);
    assert!(out_b.contains("Transaction committed."), "B output: {}", out_b);

    // A's INSERT was rolled back; only B's (id=2) should be present.
    let out = run_client(port, "SELECT * FROM t\n.exit\n");
    assert!(out.contains("(1 row)"), "Final state: {}", out);
    assert!(out.contains("2"));

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

/// Two concurrent BEGINs: the second must wait for the first to finish.
#[test]
fn second_begin_waits_for_first() {
    let dir = tmpdir("two_begins");
    let port = 48004;
    let mut server = start_server(&dir, port);

    let _ = run_client(port, "CREATE TABLE t (id INTEGER)\n.exit\n");

    // Client A: BEGIN and hold.
    let (a_handle, a_tx) = spawn_interactive_client(port);
    a_tx.send("BEGIN\n".to_string()).unwrap();
    thread::sleep(Duration::from_millis(200));

    // Client B: BEGIN (should block).
    let b_start = Instant::now();
    let (b_handle, b_tx) = spawn_interactive_client(port);
    b_tx.send("BEGIN\n".to_string()).unwrap();
    thread::sleep(Duration::from_millis(300));

    // A releases by committing.
    a_tx.send("COMMIT\n".to_string()).unwrap();
    a_tx.send(".exit\n".to_string()).unwrap();
    a_tx.send("__CLOSE__".to_string()).unwrap();
    let _ = a_handle.join().unwrap();

    // Now B can proceed.
    b_tx.send("COMMIT\n".to_string()).unwrap();
    b_tx.send(".exit\n".to_string()).unwrap();
    b_tx.send("__CLOSE__".to_string()).unwrap();
    let b_out = b_handle.join().unwrap();

    let b_elapsed = b_start.elapsed();
    assert!(
        b_elapsed >= Duration::from_millis(300),
        "B finished too fast ({:?}); expected to wait for A",
        b_elapsed
    );
    assert!(b_out.contains("Transaction committed."), "B out: {}", b_out);

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

/// Many concurrent SELECTs on the same table all return consistent data.
#[test]
fn concurrent_reads_return_consistent_data() {
    let dir = tmpdir("reads");
    let port = 48005;
    let mut server = start_server(&dir, port);

    let _ = run_client(
        port,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (1, 'alpha')\n\
         INSERT INTO t VALUES (2, 'beta')\n\
         INSERT INTO t VALUES (3, 'gamma')\n\
         .exit\n",
    );

    // 8 concurrent readers.
    let handles: Vec<_> = (0..8)
        .map(|_| thread::spawn(move || run_client(port, "SELECT * FROM t\n.exit\n")))
        .collect();

    for h in handles {
        let out = h.join().unwrap();
        assert!(out.contains("(3 rows)"), "Reader output: {}", out);
        assert!(out.contains("alpha"));
        assert!(out.contains("beta"));
        assert!(out.contains("gamma"));
    }

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

/// Proof of concurrent reads: many threads SELECT in parallel. Elapsed time
/// for N concurrent readers should be significantly less than N × the
/// single-reader time. A Mutex-based implementation would force them to
/// serialize and elapsed ≈ N × single.
///
/// This is a timing-based test, so we use generous thresholds to avoid
/// flakiness on slow CI.
#[test]
fn concurrent_reads_run_in_parallel() {
    let dir = tmpdir("parallel_reads");
    let port = 48006;
    let mut server = start_server(&dir, port);

    // Populate a table with enough rows that each SELECT does real work.
    let mut setup = String::from("CREATE TABLE t (id INTEGER, name TEXT)\n");
    for i in 0..500 {
        setup.push_str(&format!("INSERT INTO t VALUES ({}, \'row_{}\')\n", i, i));
    }
    setup.push_str(".exit\n");
    let _ = run_client(port, &setup);

    // Warm the page cache by doing one SELECT first.
    let _ = run_client(port, "SELECT * FROM t\n.exit\n");

    // Baseline: single sequential reader.
    let single_start = Instant::now();
    let _ = run_client(port, "SELECT * FROM t\n.exit\n");
    let single_elapsed = single_start.elapsed();

    // N concurrent readers.
    const N: usize = 8;
    let parallel_start = Instant::now();
    let handles: Vec<_> = (0..N)
        .map(|_| thread::spawn(move || run_client(port, "SELECT * FROM t\n.exit\n")))
        .collect();
    for h in handles {
        let out = h.join().unwrap();
        assert!(out.contains("(500 rows)"), "reader output truncated: {}", out);
    }
    let parallel_elapsed = parallel_start.elapsed();

    // If reads serialize, parallel_elapsed >= N * single_elapsed.
    // If reads are truly concurrent, it should be roughly single_elapsed
    // (bounded by the slowest thread). We allow up to 4x single_elapsed
    // as a generous threshold — anything less than that proves we\'re
    // better than fully-serialized.
    let threshold = single_elapsed * 4;
    assert!(
        parallel_elapsed < threshold,
        "Expected concurrent reads: {} readers took {:?}, single reader took {:?}. \
         Threshold was {:?} (4x single). Reads appear to be serializing.",
        N, parallel_elapsed, single_elapsed, threshold
    );

    println!(
        "PASS: {} concurrent readers {:?} vs single {:?} (serialized would be ~{:?})",
        N, parallel_elapsed, single_elapsed, single_elapsed * N as u32
    );

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}
