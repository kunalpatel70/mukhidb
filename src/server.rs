//! TCP server: accepts clients concurrently, one thread per client.
//!
//! Milestone 9: thread-per-connection. Each client gets its own `Session`
//! which serializes access to the shared `Arc<Shared>` storage via RwLock +
//! transaction-owner gate. See `session.rs` for the concurrency model.

use std::io::{BufReader, BufWriter};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use crate::executor::{execute, ExecuteResult};
use crate::parser::parse;
use crate::protocol::{read_message, write_message, Message};
use crate::session::{Session, Shared};
use crate::storage::Storage;

/// Bind to the given address and serve clients concurrently.
/// Loops forever; returns only on bind error.
pub fn run(host: &str, port: u16) -> Result<(), String> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).map_err(|e| format!("Bind {} failed: {}", addr, e))?;

    // Build shared storage up front so the .db load happens once.
    let mut storage = Storage::new();
    for path in crate::disk::find_db_files() {
        let name = path.trim_end_matches(".db").to_string();
        if let Err(e) = storage.open_table(&name, &path) {
            eprintln!("Warning: failed to load {}: {}", path, e);
        } else {
            println!("Loaded table '{}' from {}", name, path);
        }
    }
    let shared = Shared::new(storage);

    println!("mukhidb server listening on {}", addr);

    loop {
        match listener.accept() {
            Ok((stream, peer)) => {
                println!("Client connected: {}", peer);
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    let mut session = Session::new(shared);
                    if let Err(e) = handle_session(stream, &mut session) {
                        eprintln!("Session error ({}): {}", peer, e);
                    }
                    println!("Client disconnected: {}", peer);
                    // Session drops here — auto-rollback if txn was left open.
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}

fn handle_session(stream: TcpStream, session: &mut Session) -> std::io::Result<()> {
    let read_stream = stream.try_clone()?;
    let mut reader = BufReader::new(read_stream);
    let mut writer = BufWriter::new(stream);

    session_loop(&mut reader, &mut writer, session)
}

fn session_loop<R: std::io::Read, W: std::io::Write>(
    reader: &mut BufReader<R>,
    writer: &mut BufWriter<W>,
    session: &mut Session,
) -> std::io::Result<()> {
    loop {
        let msg = match read_message(reader) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        let response = match msg {
            Message::Query(sql) => dispatch(&sql, session),
            other => Message::Error(format!("Unexpected message from client: {:?}", other)),
        };

        write_message(writer, &response)?;
    }
}

/// Parse + execute a SQL string, converting the result into a Message.
fn dispatch(sql: &str, session: &mut Session) -> Message {
    let sql = sql.trim();

    if let Some(stripped) = sql.strip_prefix(".btree ") {
        let table = stripped.trim();
        return match session.dump_btree(table) {
            Ok(tree) => Message::Ok(tree),
            Err(e) => Message::Error(e),
        };
    }

    let statement = parse(sql);
    match execute(statement, session) {
        ExecuteResult::Message(s) if s.starts_with("Error:") => {
            Message::Error(s.trim_start_matches("Error:").trim().to_string())
        }
        ExecuteResult::Message(s) => Message::Ok(s),
        ExecuteResult::Rows { headers, rows } => Message::Rows { headers, rows },
    }
}
