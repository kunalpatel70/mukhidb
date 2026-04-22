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
fn where_eq_integer() {
    let dir = tmpdir("where_eq_int");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE users (id INTEGER, name TEXT)\n\
         INSERT INTO users VALUES (1, 'Alice')\n\
         INSERT INTO users VALUES (2, 'Bob')\n\
         INSERT INTO users VALUES (3, 'Charlie')\n\
         SELECT * FROM users WHERE id = 2\n\
         .exit\n",
    );
    assert!(out.contains("Bob"));
    assert!(!out.contains("Alice"));
    assert!(!out.contains("Charlie"));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn where_eq_text() {
    let dir = tmpdir("where_eq_text");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE users (id INTEGER, name TEXT)\n\
         INSERT INTO users VALUES (1, 'Alice')\n\
         INSERT INTO users VALUES (2, 'Bob')\n\
         SELECT * FROM users WHERE name = 'Alice'\n\
         .exit\n",
    );
    assert!(out.contains("Alice"));
    assert!(!out.contains("Bob"));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn where_gt() {
    let dir = tmpdir("where_gt");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (1, 'A')\n\
         INSERT INTO t VALUES (5, 'B')\n\
         INSERT INTO t VALUES (10, 'C')\n\
         SELECT * FROM t WHERE id > 3\n\
         .exit\n",
    );
    assert!(!out.contains("| A"));
    assert!(out.contains("B"));
    assert!(out.contains("C"));
    assert!(out.contains("(2 rows)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn where_lt() {
    let dir = tmpdir("where_lt");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (1, 'A')\n\
         INSERT INTO t VALUES (5, 'B')\n\
         INSERT INTO t VALUES (10, 'C')\n\
         SELECT * FROM t WHERE id < 5\n\
         .exit\n",
    );
    assert!(out.contains("A"));
    assert!(!out.contains("| B"));
    assert!(!out.contains("C"));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn where_no_match() {
    let dir = tmpdir("where_nomatch");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (1, 'A')\n\
         SELECT * FROM t WHERE id = 999\n\
         .exit\n",
    );
    assert!(out.contains("(0 rows)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn select_all_still_works() {
    let dir = tmpdir("select_all");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (1, 'A')\n\
         INSERT INTO t VALUES (2, 'B')\n\
         SELECT * FROM t\n\
         .exit\n",
    );
    assert!(out.contains("(2 rows)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn where_across_split() {
    let dir = tmpdir("where_split");
    // 20 rows forces a leaf split for (id INTEGER, name TEXT) schema (max 15 per leaf)
    let mut input = String::from("CREATE TABLE t (id INTEGER, name TEXT)\n");
    for i in 1..=20 {
        input.push_str(&format!("INSERT INTO t VALUES ({}, 'user{}')\n", i, i));
    }
    input.push_str("SELECT * FROM t WHERE id = 1\n");   // left leaf
    input.push_str("SELECT * FROM t WHERE id = 20\n");  // right leaf
    input.push_str("SELECT * FROM t WHERE id > 15\n");  // spans the split point
    input.push_str(".exit\n");

    let out = run_in_dir(&dir, &input);
    // id = 1 -> one row from left leaf
    assert!(out.contains("user1"));
    // id = 20 -> one row from right leaf
    assert!(out.contains("user20"));
    // id > 15 -> 5 rows (16..20)
    assert!(out.contains("(5 rows)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn join_basic() {
    let dir = tmpdir("join_basic");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE users (id INTEGER, name TEXT)\n\
         CREATE TABLE orders (id INTEGER, user_id INTEGER)\n\
         INSERT INTO users VALUES (1, 'Alice')\n\
         INSERT INTO users VALUES (2, 'Bob')\n\
         INSERT INTO orders VALUES (10, 1)\n\
         INSERT INTO orders VALUES (20, 2)\n\
         INSERT INTO orders VALUES (30, 1)\n\
         SELECT * FROM users JOIN orders ON users.id = orders.user_id\n\
         .exit\n",
    );
    // Alice matches orders 10 and 30, Bob matches order 20
    assert!(out.contains("Alice"));
    assert!(out.contains("Bob"));
    assert!(out.contains("(3 rows)"));
    // Headers should be prefixed
    assert!(out.contains("users.id"));
    assert!(out.contains("orders.user_id"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn join_with_where() {
    let dir = tmpdir("join_where");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE users (id INTEGER, name TEXT)\n\
         CREATE TABLE orders (id INTEGER, user_id INTEGER)\n\
         INSERT INTO users VALUES (1, 'Alice')\n\
         INSERT INTO users VALUES (2, 'Bob')\n\
         INSERT INTO orders VALUES (10, 1)\n\
         INSERT INTO orders VALUES (20, 2)\n\
         SELECT * FROM users JOIN orders ON users.id = orders.user_id WHERE users.name = 'Alice'\n\
         .exit\n",
    );
    assert!(out.contains("Alice"));
    assert!(!out.contains("Bob"));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn join_no_match() {
    let dir = tmpdir("join_nomatch");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE a (id INTEGER, val TEXT)\n\
         CREATE TABLE b (id INTEGER, a_id INTEGER)\n\
         INSERT INTO a VALUES (1, 'x')\n\
         INSERT INTO b VALUES (10, 999)\n\
         SELECT * FROM a JOIN b ON a.id = b.a_id\n\
         .exit\n",
    );
    assert!(out.contains("(0 rows)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn join_bare_column_names() {
    let dir = tmpdir("join_bare");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE users (id INTEGER, name TEXT)\n\
         CREATE TABLE orders (id INTEGER, user_id INTEGER)\n\
         INSERT INTO users VALUES (1, 'Alice')\n\
         INSERT INTO orders VALUES (10, 1)\n\
         SELECT * FROM users JOIN orders ON id = user_id\n\
         .exit\n",
    );
    assert!(out.contains("Alice"));
    assert!(out.contains("(1 row)"));
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

#[test]
fn transaction_commit() {
    let dir = tmpdir("txn_commit");
    let out = run_in_dir(
        &dir,
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
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn transaction_rollback() {
    let dir = tmpdir("txn_rollback");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (1, 'Alice')\n\
         BEGIN\n\
         INSERT INTO t VALUES (2, 'Bob')\n\
         INSERT INTO t VALUES (3, 'Charlie')\n\
         ROLLBACK\n\
         SELECT * FROM t\n\
         .exit\n",
    );
    assert!(out.contains("Transaction rolled back."));
    assert!(out.contains("Alice"));
    assert!(!out.contains("Bob"));
    assert!(!out.contains("Charlie"));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn transaction_rollback_persists_prior_data() {
    let dir = tmpdir("txn_rollback_persist");
    // Insert, commit implicitly, then rollback a second batch — first insert survives restart.
    run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         INSERT INTO t VALUES (1, 'Kept')\n\
         BEGIN\n\
         INSERT INTO t VALUES (2, 'Gone')\n\
         ROLLBACK\n\
         .exit\n",
    );
    let out = run_in_dir(
        &dir,
        "SELECT * FROM t\n\
         .exit\n",
    );
    assert!(out.contains("Kept"));
    assert!(!out.contains("Gone"));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn commit_persists_across_restart() {
    let dir = tmpdir("txn_commit_persist");
    run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER, name TEXT)\n\
         BEGIN\n\
         INSERT INTO t VALUES (1, 'Alice')\n\
         COMMIT\n\
         .exit\n",
    );
    let out = run_in_dir(
        &dir,
        "SELECT * FROM t\n\
         .exit\n",
    );
    assert!(out.contains("Alice"));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn double_begin_error() {
    let dir = tmpdir("txn_double_begin");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER)\n\
         BEGIN\n\
         BEGIN\n\
         .exit\n",
    );
    assert!(out.contains("Already in a transaction"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn commit_without_begin_error() {
    let dir = tmpdir("txn_no_begin_commit");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER)\n\
         COMMIT\n\
         .exit\n",
    );
    assert!(out.contains("No active transaction"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rollback_without_begin_error() {
    let dir = tmpdir("txn_no_begin_rollback");
    let out = run_in_dir(
        &dir,
        "CREATE TABLE t (id INTEGER)\n\
         ROLLBACK\n\
         .exit\n",
    );
    assert!(out.contains("No active transaction"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn long_text_value() {
    let dir = tmpdir("long_text");
    let long = "x".repeat(500);
    let input = format!(
        "CREATE TABLE t (id INTEGER, bio TEXT)\n\
         INSERT INTO t VALUES (1, '{}')\n\
         SELECT * FROM t\n\
         .exit\n",
        long
    );
    let out = run_in_dir(&dir, &input);
    assert!(out.contains(&long));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mixed_length_texts() {
    let dir = tmpdir("mixed_text");
    let short = "Hi";
    let medium = "a]".repeat(50);
    let long = "z".repeat(300);
    let input = format!(
        "CREATE TABLE t (id INTEGER, msg TEXT)\n\
         INSERT INTO t VALUES (1, '{}')\n\
         INSERT INTO t VALUES (2, '{}')\n\
         INSERT INTO t VALUES (3, '{}')\n\
         SELECT * FROM t\n\
         .exit\n",
        short, medium, long
    );
    let out = run_in_dir(&dir, &input);
    assert!(out.contains(short));
    assert!(out.contains(&medium));
    assert!(out.contains(&long));
    assert!(out.contains("(3 rows)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn long_text_persists_across_restart() {
    let dir = tmpdir("long_text_persist");
    let long = "persistence_test_".repeat(30);
    let input = format!(
        "CREATE TABLE t (id INTEGER, data TEXT)\n\
         INSERT INTO t VALUES (1, '{}')\n\
         .exit\n",
        long
    );
    run_in_dir(&dir, &input);

    let out = run_in_dir(
        &dir,
        "SELECT * FROM t\n\
         .exit\n",
    );
    assert!(out.contains(&long));
    assert!(out.contains("(1 row)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn many_rows_with_variable_text() {
    let dir = tmpdir("var_stress");
    let mut input = String::from("CREATE TABLE t (id INTEGER, name TEXT)\n");
    for i in 0..100 {
        // Vary text lengths: some short, some long
        let text = "a".repeat((i % 50) + 1);
        input.push_str(&format!("INSERT INTO t VALUES ({}, '{}')\n", i, text));
    }
    input.push_str("SELECT * FROM t\n.exit\n");
    let out = run_in_dir(&dir, &input);
    assert!(out.contains("(100 rows)"));
    let _ = fs::remove_dir_all(&dir);
}
