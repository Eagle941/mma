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
        let mut order_book_local = OrderBook::default();
        order_book_local.subscribe(&mut producer, symbol);
    });

    // Added sleep to give time to the websocket to retrieve the first order
    // book snapshot
    thread::sleep(Duration::from_millis(1000));

    let strategy_thread = thread::spawn(move || loop {
        let order_book = consumer.read();
        if order_book.bids.len() != 0 && order_book.asks.len() != 0 {
            let first_bid = order_book.bids.first().unwrap();
            let last_bid = order_book.bids.last().unwrap();
            let first_ask = order_book.asks.first().unwrap();
            let last_ask = order_book.asks.last().unwrap();

            println!(
                "B {:.2} {:.2} | A {:.2} {:.2} | S {:.2}",
                last_bid.price,
                first_bid.price,
                first_ask.price,
                last_ask.price,
                if first_bid.price != 0.0 && first_ask.price != 0.0 {
                    first_ask.price - first_bid.price
                } else {
                    0.0
                }
            );
        }
        thread::sleep(Duration::from_millis(1000));
    });

    ws_thread.join().expect("ws_thread has panicked");
    strategy_thread
        .join()
        .expect("strategy_thread has panicked");
    Ok(())
}
