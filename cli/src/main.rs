use clap::Parser;
use exchange::bybit::book::OrderBook;
use exitcode::{OK, SOFTWARE};
use std::str::FromStr;
use std::{env, process, thread, time::Duration};
use strategy::simple::SimpleStrategy;
use triple_buffer::TripleBuffer;

#[derive(Clone, Parser, Debug)]
pub struct Args {}

fn main() {
    // TODO: handle SIGTERM (^C) gracefully
    // TODO: evaluate whether to use any cli argument or use `.env` file only
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
    dotenvy::dotenv()?;

    let order_book = OrderBook::default();
    let (mut producer, mut consumer) = TripleBuffer::new(&order_book).split();

    let ws_thread = thread::spawn(move || {
        let symbol = env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");
        let mut order_book_local = OrderBook::default();
        order_book_local.subscribe(&mut producer, &symbol);
    });

    // Added sleep to give time to the websocket to retrieve the first order
    // book snapshot
    thread::sleep(Duration::from_millis(1000));

    let strategy_thread = thread::spawn(move || {
        let symbol = env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");

        let spread = env::var("MMA_SPREAD").expect("MMA_SPREAD env variable must not be blank.");
        let spread = f64::from_str(&spread).expect("MMA_SPREAD is not a valid number.");

        let size =
            env::var("MMA_ORDER_SIZE").expect("MMA_ORDER_SIZE env variable must not be blank.");
        let size = f64::from_str(&size).expect("MMA_ORDER_SIZE is not a valid number.");

        let simple_strategy = SimpleStrategy::new(spread, size, symbol.as_str());

        loop {
            let order_book = consumer.read();
            simple_strategy.execute(order_book);
            thread::sleep(Duration::from_millis(1000));
        }
    });

    ws_thread.join().expect("ws_thread has panicked");
    strategy_thread
        .join()
        .expect("strategy_thread has panicked");
    Ok(())
}
