use std::sync::Arc;
use std::time::Duration;
use std::{env, process, thread};

use clap::Parser;
use crossbeam_channel::{Receiver, Sender, unbounded};
use crossbeam_queue::ArrayQueue;
use env_logger::{Builder, Env};
use exchange::bybit::private_ws::PrivateWebSocket;
use exchange::bybit::public_ws::PublicWebSocket;
use exchange::{OrderBook, OrderBuilder, OrderMessages};
use exitcode::{OK, SOFTWARE};
use log::info;
use oms::OrderManagementSystem;
use recorder::MarkoutEngine;
use strategy::simple::SimpleStrategy;
use triple_buffer::TripleBuffer;

#[derive(Clone, Parser, Debug)]
pub struct Args {}

#[tokio::main]
async fn main() {
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

    let env = Env::default()
        .filter_or("RUST_LOG", "warn")
        .write_style_or("RUST_LOG_STYLE", "always");
    Builder::from_env(env)
        .format_level(false)
        .format_timestamp_nanos()
        .init();

    info!("Started MMA");
    let runtime_handle = tokio::runtime::Handle::current();

    let order_book = OrderBook::default();
    let (mut producer, mut consumer) = TripleBuffer::new(&order_book).split();

    // NOTE: The queue has a length of 1 because only the most recent value of
    // order_book is useful. If the queue is full, the value is replaced.
    let order_book_queue: ArrayQueue<OrderBook> = ArrayQueue::new(1);
    let order_book_queue = Arc::new(order_book_queue);
    let to_recorder = Arc::clone(&order_book_queue);
    let from_book = Arc::clone(&order_book_queue);

    let public_ws_thread = thread::Builder::new()
        .name("public_ws_thread".to_string())
        .spawn(move || {
            let symbol =
                env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");
            let mut handler = PublicWebSocket::new(to_recorder);
            handler.subscribe(&mut producer, &symbol);
        })?;

    let (order_builder_to_oms, from_strategy): (Sender<OrderBuilder>, Receiver<OrderBuilder>) =
        unbounded();
    let (execution_to_oms, to_oms): (Sender<OrderMessages>, Receiver<OrderMessages>) = unbounded();
    let (execution_to_recorder, to_recorder): (Sender<OrderMessages>, Receiver<OrderMessages>) =
        unbounded();

    // NOTE: The queue has a length of 1 because only the most recent value of
    // inventory is useful. If the queue is full, the value is replaced.
    let inventory_queue: ArrayQueue<f64> = ArrayQueue::new(1);
    let inventory_queue = Arc::new(inventory_queue);
    let from_oms = Arc::clone(&inventory_queue);
    let to_strategy = Arc::clone(&inventory_queue);

    let private_ws_thread = thread::Builder::new()
        .name("private_ws_thread".to_string())
        .spawn(move || {
            let handler = PrivateWebSocket::new(execution_to_oms, execution_to_recorder);
            handler.subscribe();
        })?;

    let oms_thread = thread::Builder::new()
        .name("oms_thread".to_string())
        .spawn(move || {
            let guard = runtime_handle.enter();

            let mut oms = OrderManagementSystem::new(from_strategy, to_oms, to_strategy);
            oms.cycle();

            drop(guard)
        })?;

    let recorder_thread = thread::Builder::new()
        .name("recorder_thread".to_string())
        .spawn(move || {
            let mut recorder = MarkoutEngine::new(from_book, to_recorder);
            recorder.cycle();
        })?;

    // NOTE: start startegy last after everything else has initialised.
    // TODO: should I add a delay?
    let strategy_thread = thread::Builder::new()
        .name("strategy_thread".to_string())
        .spawn(move || {
            let mut simple_strategy = SimpleStrategy::factory(order_builder_to_oms, from_oms);
            loop {
                // NOTE: strategy is executed at around 1Hz for learning
                let order_book = consumer.read();
                simple_strategy.execute(order_book);
                thread::sleep(Duration::from_millis(1000));
            }
        })?;

    // TODO: close the program if either thread panics and crashes
    public_ws_thread
        .join()
        .expect("public_ws_thread has panicked");
    private_ws_thread
        .join()
        .expect("private_ws_thread has panicked");
    oms_thread.join().expect("oms_thread has panicked");
    recorder_thread
        .join()
        .expect("recorder_thread has panicked");
    strategy_thread
        .join()
        .expect("strategy_thread has panicked");
    Ok(())
}
