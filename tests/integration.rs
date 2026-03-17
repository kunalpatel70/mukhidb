use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn run_in_dir(dir: &str, input: &str) -> String {
    let _ = fs::create_dir_all(dir);
    let binary = format!("{}/target/debug/mukhidb", env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(&binary)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
            child.wait_with_output()
        })
        .expect("Failed to run mukhidb");
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn tmpdir(name: &str) -> String {
    let dir = format!("/tmp/mukhidb_test_{}", name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn create_insert_select() {
    let dir = tmpdir("create_insert_select");
    let out = run_in_dir(
        &dir,
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
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn persistence_across_restarts() {
    let dir = tmpdir("persistence");

    run_in_dir(
        &dir,
        "CREATE TABLE people (id INTEGER, name TEXT)\n\
         INSERT INTO people VALUES (10, 'Persist')\n\
         .exit\n",
    );

    let out = run_in_dir(
        &dir,
        "SELECT * FROM people\n\
         .exit\n",
    );
    assert!(out.contains("Loaded table 'people'"));
    assert!(out.contains("Persist"));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sorted_output() {
    let dir = tmpdir("sorted");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (3, 'Charlie')\n\
         INSERT INTO t VALUES (1, 'Alice')\n\
         INSERT INTO t VALUES (2, 'Bob')\n\
         SELECT * FROM t\n\
         .exit\n",
    );
    let alice_pos = out.find("Alice").unwrap();
    let bob_pos = out.find("Bob").unwrap();
    let charlie_pos = out.find("Charlie").unwrap();
    assert!(alice_pos < bob_pos);
    assert!(bob_pos < charlie_pos);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn type_mismatch_error() {
    let dir = tmpdir("type_mismatch");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES ('oops', 'Alice')\n\
         .exit\n",
    );
    assert!(out.contains("Error:"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_table_error() {
    let dir = tmpdir("dup_table");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER)\n\
         CREATE TABLE t (id INTEGER)\n\
         .exit\n",
    );
    assert!(out.contains("already exists"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn many_rows_stress() {
    let dir = tmpdir("stress");
    let mut input = String::from("CREATE TABLE t (id INTEGER, name TEXT)\n");
    for i in 0..200 {
        input.push_str(&format!("INSERT INTO t VALUES ({}, 'row{}')\n", i, i));
    }
    input.push_str("SELECT * FROM t\n.exit\n");

    let out = run_in_dir(&dir, &input);
    assert!(out.contains("(200 rows)"));
    let _ = fs::remove_dir_all(&dir);
}
