use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use std::{env, process, thread};

use clap::Parser;
use exchange::bybit::book::DataHandler;
use exchange::{Order, OrderBook};
use exitcode::{OK, SOFTWARE};
use oms::OrderManagementSystem;
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
        let mut data_handler = DataHandler::default();
        data_handler.subscribe(&mut producer, &symbol);
    });

    // Added sleep to give time to the websocket to retrieve the first order
    // book snapshot
    thread::sleep(Duration::from_millis(1000));

    let (tx, rx): (Sender<Order>, Receiver<Order>) = mpsc::channel();
    let strategy_thread = thread::spawn(move || {
        let simple_strategy = SimpleStrategy::factory(tx.clone());
        loop {
            let order_book = consumer.read();
            simple_strategy.execute(order_book);
            thread::sleep(Duration::from_millis(1000));
        }
    });

    let oms_thread = thread::spawn(move || {
        let oms = OrderManagementSystem::new(rx);
        oms.forward_orders();
    });

    oms_thread.join().expect("oms_thread has panicked");
    ws_thread.join().expect("ws_thread has panicked");
    strategy_thread
        .join()
        .expect("strategy_thread has panicked");
    Ok(())
}
