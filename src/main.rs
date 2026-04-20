mod btree;
mod disk;
mod executor;
mod pager;
mod parser;
mod repl;
mod row;
mod storage;
mod types;
mod wal;

fn main() {
    repl::run();
}
