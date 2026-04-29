mod btree;
mod client;
mod disk;
mod executor;
mod pager;
mod parser;
mod protocol;
mod repl;
mod row;
mod server;
mod session;
mod storage;
mod types;
mod wal;

const DEFAULT_PORT: u16 = 4567;
const DEFAULT_HOST: &str = "127.0.0.1";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("repl");

    let rest = &args[args.len().min(2)..];

    match cmd {
        "repl" => repl::run(),

        "server" => {
            let port = parse_port(rest);
            if let Err(e) = server::run(DEFAULT_HOST, port) {
                eprintln!("Server error: {}", e);
                std::process::exit(1);
            }
        }

        "connect" => {
            let (host, port) = parse_host_port(rest);
            if let Err(e) = client::run(&host, port) {
                eprintln!("Client error: {}", e);
                std::process::exit(1);
            }
        }

        "--help" | "-h" | "help" => print_usage(),

        other => {
            eprintln!("Unknown command: {}", other);
            print_usage();
            std::process::exit(2);
        }
    }
}

fn print_usage() {
    println!("mukhidb v0.5.0");
    println!();
    println!("Usage:");
    println!("  mukhidb repl                         Local REPL (default, no network)");
    println!("  mukhidb server [--port N]            Run as TCP server (default port {})", DEFAULT_PORT);
    println!("  mukhidb connect [--host H] [--port N]  Connect to a server");
    println!("  mukhidb help                         Show this message");
}

fn parse_port(args: &[String]) -> u16 {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--port" && i + 1 < args.len() {
            if let Ok(p) = args[i + 1].parse() {
                return p;
            }
        }
        i += 1;
    }
    DEFAULT_PORT
}

fn parse_host_port(args: &[String]) -> (String, u16) {
    let mut host = DEFAULT_HOST.to_string();
    let mut port = DEFAULT_PORT;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--host" && i + 1 < args.len() {
            host = args[i + 1].clone();
            i += 2;
        } else if args[i] == "--port" && i + 1 < args.len() {
            if let Ok(p) = args[i + 1].parse() {
                port = p;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    (host, port)
}