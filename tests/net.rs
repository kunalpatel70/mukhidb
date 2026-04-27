/// Client-server integration tests.
///
/// These spawn the server binary, connect a client, run a scripted session, and
/// check the output. Each test uses port 0 trick via an OS-assigned port... but
/// since our CLI takes a fixed port, we just use unique high ports per test.

use std::fs;
use std::io::Write;
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

fn tmpdir(name: &str) -> String {
    let dir = format!("/tmp/mukhidb_net_test_{}", name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn binary() -> String {
    format!("{}/target/debug/mukhidb", env!("CARGO_MANIFEST_DIR"))
}

/// Start a server in the given dir on the given port. Returns the child process.
fn start_server(dir: &str, port: u16) -> Child {
    let child = Command::new(binary())
        .args(["server", "--port", &port.to_string()])
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn server");

    // Wait for the server to bind (poll the port).
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return child;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Server on port {} did not start in time", port);
}

/// Run a client session, feeding it a script on stdin. Returns the stdout.
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

#[test]
fn server_client_basic_query() {
    let dir = tmpdir("basic");
    let port = 47001;
    let mut server = start_server(&dir, port);

    let out = run_client(
        port,
        "CREATE TABLE users (id INTEGER, name TEXT)\n\
         INSERT INTO users VALUES (1, 'Alice')\n\
         INSERT INTO users VALUES (2, 'Bob')\n\
         SELECT * FROM users\n\
         .exit\n",
    );

    assert!(out.contains("Table 'users' created."));
    assert!(out.contains("Alice"));
    assert!(out.contains("Bob"));
    assert!(out.contains("(2 rows)"));

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn server_client_error_channel() {
    let dir = tmpdir("error");
    let port = 47002;
    let mut server = start_server(&dir, port);

    let out = run_client(
        port,
        "SELECT * FROM nonexistent\n\
         .exit\n",
    );

    assert!(out.contains("Error:"));
    assert!(out.contains("not found"));

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn server_client_transactions_over_wire() {
    let dir = tmpdir("txn");
    let port = 47003;
    let mut server = start_server(&dir, port);

    let out = run_client(
        port,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         BEGIN\n\
         INSERT INTO t VALUES (1, 'Alice')\n\
         INSERT INTO t VALUES (2, 'Bob')\n\
         COMMIT\n\
         SELECT * FROM t\n\
         .exit\n",
    );

    assert!(out.contains("Transaction started."));
    assert!(out.contains("Transaction committed."));
    assert!(out.contains("Alice"));
    assert!(out.contains("Bob"));
    assert!(out.contains("(2 rows)"));

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn server_client_sequential_sessions() {
    let dir = tmpdir("sequential");
    let port = 47004;
    let mut server = start_server(&dir, port);

    // First client: write data.
    let out1 = run_client(
        port,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (42, 'Persist')\n\
         .exit\n",
    );
    assert!(out1.contains("Table 't' created."));

    // Second client: read data (server kept it in memory AND on disk).
    let out2 = run_client(
        port,
        "SELECT * FROM t\n\
         .exit\n",
    );
    assert!(out2.contains("Persist"));
    assert!(out2.contains("(1 row)"));

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn server_client_btree_meta_command() {
    let dir = tmpdir("btree_meta");
    let port = 47005;
    let mut server = start_server(&dir, port);

    let out = run_client(
        port,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (1, 'A')\n\
         INSERT INTO t VALUES (2, 'B')\n\
         .btree t\n\
         .exit\n",
    );

    // Server executes .btree and returns the tree dump as an Ok message.
    assert!(out.contains("Leaf"));
    assert!(out.contains("key 1"));
    assert!(out.contains("key 2"));

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn server_client_long_text_over_wire() {
    let dir = tmpdir("long_text");
    let port = 47006;
    let mut server = start_server(&dir, port);

    let long = "x".repeat(800);
    let input = format!(
        "CREATE TABLE t (id INTEGER, body TEXT)\n\
         INSERT INTO t VALUES (1, '{}')\n\
         SELECT * FROM t\n\
         .exit\n",
        long
    );
    let out = run_client(port, &input);

    assert!(out.contains(&long));
    assert!(out.contains("(1 row)"));

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn server_rolls_back_transaction_on_disconnect() {
    // Regression: if a client BEGINs and disconnects without COMMIT/ROLLBACK,
    // the next client must not inherit the open transaction.
    let dir = tmpdir("txn_leak");
    let port = 47007;
    let mut server = start_server(&dir, port);

    // Client 1: BEGIN, then disconnect without COMMIT/ROLLBACK.
    let out1 = run_client(
        port,
        "CREATE TABLE t (id INTEGER)\n\
         BEGIN\n\
         .exit\n",
    );
    assert!(out1.contains("Transaction started."));

    // Client 2: COMMIT with no active transaction should fail cleanly.
    let out2 = run_client(
        port,
        "COMMIT\n\
         .exit\n",
    );
    assert!(out2.contains("Error:"));
    assert!(out2.contains("No active transaction"));

    server.kill().ok();
    server.wait().ok();
    let _ = fs::remove_dir_all(&dir);
}
