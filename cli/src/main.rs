use std::time::Duration;
use std::{env, process, thread};

use clap::Parser;
use crossbeam_channel::{Receiver, Sender, unbounded};
use exchange::bybit::private_ws::PrivateWebSocket;
use exchange::bybit::public_ws::PublicWebSocket;
use exchange::{OrderBook, OrderBuilder, OrderMessages};
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
    dotenvy::dotenv().expect(".env file must be present with configuration parameters.");
    dotenvy::from_filename(".secrets")
        .expect(".secrets file must be present with API_KEY and API_SECRET.");

    let order_book = OrderBook::default();
    let (mut producer, mut consumer) = TripleBuffer::new(&order_book).split();

    let book_ws_thread = thread::Builder::new()
        .name("book_ws_thread".to_string())
        .spawn(move || {
            let symbol =
                env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");
            let mut handler = PublicWebSocket::default();
            handler.subscribe(&mut producer, &symbol);
        })?;

    // Added sleep to give time to the websocket to retrieve the first order
    // book snapshot
    thread::sleep(Duration::from_millis(1000));

    let (order_builder_to_oms, from_strategy): (Sender<OrderBuilder>, Receiver<OrderBuilder>) =
        unbounded();
    let strategy_thread = thread::Builder::new()
        .name("strategy_thread".to_string())
        .spawn(move || {
            let simple_strategy = SimpleStrategy::factory(order_builder_to_oms.clone());
            loop {
                let order_book = consumer.read();
                simple_strategy.execute(order_book);
                thread::sleep(Duration::from_millis(1000));
            }
        })?;

    let (order_to_oms, from_order_handler): (Sender<OrderMessages>, Receiver<OrderMessages>) =
        unbounded();
    let order_ws_thread = thread::Builder::new()
        .name("order_ws_thread".to_string())
        .spawn(move || {
            let handler = PrivateWebSocket::new(order_to_oms);
            handler.subscribe();
        })?;

    let oms_thread = thread::Builder::new()
        .name("oms_thread".to_string())
        .spawn(move || {
            // TODO: Improve this nested use of channels. OMS takes both sender and receiver
            // channel.
            let mut oms = OrderManagementSystem::new(from_strategy, from_order_handler);
            oms.cycle();
        })?;

    oms_thread.join().expect("oms_thread has panicked");
    book_ws_thread.join().expect("book_ws_thread has panicked");
    order_ws_thread
        .join()
        .expect("order_ws_thread has panicked");
    strategy_thread
        .join()
        .expect("strategy_thread has panicked");
    Ok(())
}
