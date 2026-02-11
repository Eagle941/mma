use std::process;

use clap::Parser;
use exchange::bybit::OrderBook;
use exitcode::{OK, SOFTWARE};

#[derive(Clone, Parser, Debug)]
pub struct Args {}

fn main() {
    // TODO: handle SIGTERM (^C) gracefully

    let args = Args::parse();

    match run(args) {
        Ok(()) => process::exit(OK),
        Err(e) => {
            eprintln!("Internal software error: {e}");
            process::exit(SOFTWARE);
        }
    }
}

fn run(_args: Args) -> anyhow::Result<()> {
    let symbol = "ETHUSDT";
    let mut order_book = OrderBook::default();
    order_book.subscribe(symbol);
    Ok(())
}
