use std::{process, thread, time::Duration};

use clap::Parser;
use exchange::bybit::OrderBook;
use exitcode::{OK, SOFTWARE};
use triple_buffer::TripleBuffer;

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
    let order_book = OrderBook::default();
    let (mut producer, mut consumer) = TripleBuffer::new(&order_book).split();

    let ws_thread = thread::spawn(move || {
        let symbol = "ETHUSDT";
        OrderBook::subscribe(&mut producer, symbol);
    });

    let strategy_thread = thread::spawn(move || loop {
        let order_book = consumer.read();
        println!("Hello! {:?}", order_book.bids.first());
        thread::sleep(Duration::from_millis(1000));
    });

    ws_thread.join().expect("ws_thread has panicked");
    strategy_thread
        .join()
        .expect("strategy_thread has panicked");
    Ok(())
}
