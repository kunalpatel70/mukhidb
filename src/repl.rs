use std::io::{self, Write};
use crate::executor::{execute, ExecuteResult};
use crate::parser::parse;
use crate::storage::Storage;

/// Start the REPL — reads SQL from stdin, executes it, prints results.
pub fn run() {
    let mut storage = Storage::new();

    // Restore persisted tables from .db files (now backed by pager + B+Tree)
    for path in crate::disk::find_db_files() {
        let name = path.trim_end_matches(".db").to_string();
        match storage.open_table(&name, &path) {
            Ok(_) => println!("Loaded table '{}' from {}", name, path),
            Err(e) => eprintln!("Warning: failed to load {}: {}", path, e),
        }
    }

    println!("mukhidb v0.4.0 (repl mode)  |  Type .exit to quit, .help for hints.");

    loop {
        print!("mukhidb> ");
        io::stdout().flush().expect("Failed to flush stdout");

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => break, // EOF (Ctrl-D)
            Ok(_) => {}
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        }

        let input = input.trim();
        if input.is_empty() { continue; }

        // Meta-commands (start with '.')
        if input.starts_with('.') {
            handle_meta(input, &mut storage);
            continue;
        }

        let statement = parse(input);
        let result    = execute(statement, &mut storage);

        match result {
            ExecuteResult::Message(msg) => println!("{}", msg),
            ExecuteResult::Rows { headers, rows } => print_table(headers, rows),
        }
    }

    println!("\nBye!");
}

fn handle_meta(cmd: &str, storage: &mut Storage) {
    match cmd {
        ".exit" | ".quit" => {
            println!("Bye!");
            std::process::exit(0);
        }
        ".help" => {
            println!("Supported SQL:");
            println!("  CREATE TABLE <name> (<col> INTEGER|TEXT, ...)");
            println!("  INSERT INTO <name> VALUES (<val>, ...)");
            println!("  SELECT * FROM <name> [JOIN <name> ON <col> = <col>] [WHERE ...]");
            println!("  BEGIN / COMMIT / ROLLBACK");
            println!("Meta-commands:");
            println!("  .help            — show this message");
            println!("  .btree <table>   — visualise B+Tree structure");
            println!("  .exit            — quit");
        }
        _ if cmd.starts_with(".btree") => {
            let table = cmd[".btree".len()..].trim();
            if table.is_empty() {
                println!("Usage: .btree <table_name>");
            } else {
                match storage.dump_btree(table) {
                    Ok(tree) => println!("{}", tree),
                    Err(e) => println!("Error: {}", e),
                }
            }
        }
        _ => println!("Unknown meta-command: '{}'", cmd),
    }
}

/// Pretty-print query results as an aligned table.
fn print_table(headers: Vec<String>, rows: Vec<Vec<String>>) {
    if rows.is_empty() {
        println!("(0 rows)");
        return;
    }

    // Calculate column widths
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    // Header row
    let header_line: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
        .collect();
    println!("{}", header_line.join(" | "));

    // Divider
    let divider: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    println!("{}", divider.join("-+-"));

    // Data rows
    for row in &rows {
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
