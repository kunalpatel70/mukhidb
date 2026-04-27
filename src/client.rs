/// TCP client: REPL that sends SQL to a mukhidb server and prints results.

use std::io::{self, BufReader, BufWriter, Write};
use std::net::TcpStream;

use crate::protocol::{read_message, write_message, Message};

/// Connect to a mukhidb server and run an interactive REPL.
pub fn run(host: &str, port: u16) -> Result<(), String> {
    let addr = format!("{}:{}", host, port);
    let stream = TcpStream::connect(&addr)
        .map_err(|e| format!("Connect to {} failed: {}", addr, e))?;

    let read_stream = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(read_stream);
    let mut writer = BufWriter::new(stream);

    println!("mukhidb client v0.4.0  |  Connected to {}  |  .exit to quit, .help for hints.", addr);

    loop {
        print!("mukhidb> ");
        io::stdout().flush().ok();

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        // Client-side meta-commands (don't go over the wire).
        match input {
            ".exit" | ".quit" => break,
            ".help" => {
                print_help();
                continue;
            }
            _ => {}
        }

        // Everything else (including .btree) is sent to the server.
        if let Err(e) = write_message(&mut writer, &Message::Query(input.to_string())) {
            eprintln!("Send failed: {}", e);
            break;
        }

        match read_message(&mut reader) {
            Ok(Message::Ok(s)) => println!("{}", s),
            Ok(Message::Error(s)) => println!("Error: {}", s),
            Ok(Message::Rows { headers, rows }) => print_table(&headers, &rows),
            Ok(other) => println!("Unexpected message from server: {:?}", other),
            Err(e) => {
                eprintln!("Connection lost: {}", e);
                break;
            }
        }
    }

    println!("
Bye!");
    Ok(())
}

fn print_help() {
    println!("Supported SQL:");
    println!("  CREATE TABLE <name> (<col> INTEGER|TEXT, ...)");
    println!("  INSERT INTO <name> VALUES (<val>, ...)");
    println!("  SELECT * FROM <name> [JOIN <name> ON <col> = <col>] [WHERE ...]");
    println!("  BEGIN / COMMIT / ROLLBACK");
    println!("Meta-commands:");
    println!("  .help            — show this message");
    println!("  .btree <table>   — visualise B+Tree structure (executed on server)");
    println!("  .exit            — disconnect");
}

/// Pretty-print query results as an aligned table.
fn print_table(headers: &[String], rows: &[Vec<String>]) {
    if rows.is_empty() {
        println!("(0 rows)");
        return;
    }

    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let header_line: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
        .collect();
    println!("{}", header_line.join(" | "));

    let divider: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    println!("{}", divider.join("-+-"));

    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let w = widths.get(i).copied().unwrap_or(cell.len());
                format!("{:<width$}", cell, width = w)
            })
            .collect();
        println!("{}", cells.join(" | "));
    }

    println!("({} row{})", rows.len(), if rows.len() == 1 { "" } else { "s" });
}
