mod btree;
mod disk;
mod executor;
mod pager;
mod parser;
mod repl;
mod row;
mod storage;
mod types;

fn main() {
    repl::run();
}
