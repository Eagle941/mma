use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, process};

use clap::Parser;
use crossbeam_channel::{Receiver, Sender, unbounded};
use crossbeam_queue::ArrayQueue;
use env_logger::{Builder, Env};
use exchange::bybit::market::Trades;
use exchange::bybit::wallet::Wallet;
use exchange::{OrderBook, OrderBuilder, OrderEvent};
use exitcode::{OK, SOFTWARE};
use log::info;
use oms::OmsConfig;
use triple_buffer::TripleBuffer;

use crate::threads::{
    create_oms_thread,
    create_private_ws_thread,
    create_public_ws_thread,
    create_recorder_thread,
    create_strategy_thread,
};

mod threads;

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

fn load_oms_config() -> OmsConfig {
    let wallet = Wallet::new();
    // TODO: infer the coin from the `base_coin` field of instrument info.
    let coin = env::var("MMA_COIN").expect("MMA_COIN env variable must not be blank.");
    let inventory = wallet.coins.get(&coin).copied().unwrap_or(0.0);
    let avg_entry_price = if inventory == 0.0 {
        0.0
    } else {
        Trades::factory().last_price
    };
    let next_order_link_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System clock went backwards!")
        .as_micros() as u64;

    OmsConfig::new(coin, inventory, avg_entry_price, next_order_link_id)
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
    let oms_config = load_oms_config();

    let order_book = OrderBook::default();
    let (order_book_input, order_book_output) = TripleBuffer::new(&order_book).split();

    // NOTE: The queue has a length of 1 because only the most recent value of
    // order_book is useful. If the queue is full, the value is replaced.
    let order_book_queue: ArrayQueue<OrderBook> = ArrayQueue::new(1);
    let order_book_queue = Arc::new(order_book_queue);
    let public_ws_order_books = Arc::clone(&order_book_queue);
    let recorder_order_books = Arc::clone(&order_book_queue);

    let public_ws_thread = create_public_ws_thread(public_ws_order_books, order_book_input)?;

    let (strategy_orders_tx, strategy_orders_rx): (Sender<OrderBuilder>, Receiver<OrderBuilder>) =
        unbounded();
    let (order_events_tx, order_events_rx): (Sender<OrderEvent>, Receiver<OrderEvent>) =
        unbounded();
    let (recorder_events_tx, recorder_events_rx): (Sender<OrderEvent>, Receiver<OrderEvent>) =
        unbounded();

    // NOTE: The queue has a length of 1 because only the most recent value of
    // inventory is useful. If the queue is full, the value is replaced.
    let inventory_queue: ArrayQueue<f64> = ArrayQueue::new(1);
    let inventory_queue = Arc::new(inventory_queue);
    let strategy_inventory = Arc::clone(&inventory_queue);
    let oms_inventory = Arc::clone(&inventory_queue);

    let private_ws_thread = create_private_ws_thread(order_events_tx.clone(), recorder_events_tx)?;
    let oms_thread = create_oms_thread(
        runtime_handle,
        strategy_orders_rx,
        order_events_rx,
        order_events_tx,
        oms_inventory,
        oms_config,
    )?;
    let recorder_thread = create_recorder_thread(recorder_order_books, recorder_events_rx)?;

    // NOTE: start startegy last after everything else has initialised.
    // TODO: should I add a delay?
    let strategy_thread =
        create_strategy_thread(strategy_orders_tx, strategy_inventory, order_book_output)?;

    // TODO: Add a function that creates the communication channels and starts all
    // worker threads, returning their handles. Add a separate function that
    // monitors those handles for worker failures and coordinates graceful
    // shutdown, including cancellation of open orders.
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
