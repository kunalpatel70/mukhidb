/// TCP server: accepts one client at a time, dispatches SQL to the executor.
///
/// Milestone 8 scope: single-client. Milestone 9 will add concurrency.

use std::io::{BufReader, BufWriter};
use std::net::{TcpListener, TcpStream};

use crate::executor::{execute, ExecuteResult};
use crate::parser::parse;
use crate::protocol::{read_message, write_message, Message};
use crate::storage::Storage;

/// Bind to the given address and serve clients one at a time.
/// Loops forever; returns only on bind error.
pub fn run(host: &str, port: u16) -> Result<(), String> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).map_err(|e| format!("Bind {} failed: {}", addr, e))?;

    // Storage is owned by the server and shared across client sessions.
    let mut storage = Storage::new();

    // Load any existing tables from disk.
    for path in crate::disk::find_db_files() {
        let name = path.trim_end_matches(".db").to_string();
        if let Err(e) = storage.open_table(&name, &path) {
            eprintln!("Warning: failed to load {}: {}", path, e);
        } else {
            println!("Loaded table '\''{}'\'' from {}", name, path);
        }
    }

    println!("mukhidb server listening on {}", addr);

    loop {
        match listener.accept() {
            Ok((stream, peer)) => {
                println!("Client connected: {}", peer);
                if let Err(e) = handle_session(stream, &mut storage) {
                    eprintln!("Session error ({}): {}", peer, e);
                }
                println!("Client disconnected: {}", peer);
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}

/// Handle one client session until the client disconnects.
///
/// If the client disconnects with an open transaction, it is rolled back so
/// state does not leak into the next session.
fn handle_session(stream: TcpStream, storage: &mut Storage) -> std::io::Result<()> {
    let read_stream = stream.try_clone()?;
    let mut reader = BufReader::new(read_stream);
    let mut writer = BufWriter::new(stream);

    let result = session_loop(&mut reader, &mut writer, storage);

    // Drop any transaction the client left open.
    let _ = storage.rollback();

    result
}

fn session_loop<R: std::io::Read, W: std::io::Write>(
    reader: &mut BufReader<R>,
    writer: &mut BufWriter<W>,
    storage: &mut Storage,
) -> std::io::Result<()> {
    loop {
        let msg = match read_message(reader) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Client closed the connection.
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        let response = match msg {
            Message::Query(sql) => dispatch(&sql, storage),
            other => Message::Error(format!("Unexpected message from client: {:?}", other)),
        };

        write_message(writer, &response)?;
    }
}

/// Parse + execute a SQL string, converting the result into a Message.
fn dispatch(sql: &str, storage: &mut Storage) -> Message {
    let sql = sql.trim();

    // Meta-commands that need server access (e.g. .btree <table>).
    if let Some(stripped) = sql.strip_prefix(".btree ") {
        let table = stripped.trim();
        return match storage.dump_btree(table) {
            Ok(tree) => Message::Ok(tree),
            Err(e) => Message::Error(e),
        };
    }

    let statement = parse(sql);
    match execute(statement, storage) {
        ExecuteResult::Message(s) if s.starts_with("Error:") => {
            // Executor prefixes errors with "Error:" — strip it and send as Error.
            Message::Error(s.trim_start_matches("Error:").trim().to_string())
        }
        ExecuteResult::Message(s) => Message::Ok(s),
        ExecuteResult::Rows { headers, rows } => Message::Rows { headers, rows },
    }
}