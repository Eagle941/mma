use std::process;
use std::sync::Arc;

use clap::Parser;
use configuration::{AppConfig, SharedAppConfig};
use crossbeam_channel::unbounded;
use crossbeam_queue::ArrayQueue;
use env_logger::Builder;
use exchange::{OrderBook, OrderBuilder, OrderEvent};
use exitcode::{OK, SOFTWARE};
use log::info;
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

fn run(_args: Args) -> anyhow::Result<()> {
    let config: SharedAppConfig = Arc::new(AppConfig::load()?);

    Builder::new()
        .parse_filters(config.log_filter())
        .write_style(config.log_style())
        .format_level(false)
        .format_timestamp_nanos()
        .init();

    info!("Started MMA");
    let runtime_handle = tokio::runtime::Handle::current();

    let order_book = OrderBook::default();
    let (order_book_input, order_book_output) = TripleBuffer::new(&order_book).split();

    // NOTE: The queue has a length of 1 because only the most recent value of
    // order_book is useful. If the queue is full, the value is replaced.
    let order_book_queue = Arc::new(ArrayQueue::<OrderBook>::new(1));
    let public_ws_order_books = Arc::clone(&order_book_queue);
    let recorder_order_books = Arc::clone(&order_book_queue);

    let public_ws_thread =
        create_public_ws_thread(public_ws_order_books, order_book_input, Arc::clone(&config))?;

    let (strategy_orders_tx, strategy_orders_rx) = unbounded::<OrderBuilder>();
    let (order_events_tx, order_events_rx) = unbounded::<OrderEvent>();
    let (recorder_events_tx, recorder_events_rx) = unbounded::<OrderEvent>();

    // NOTE: The queue has a length of 1 because only the most recent value of
    // inventory is useful. If the queue is full, the value is replaced.
    let inventory_queue = Arc::new(ArrayQueue::<f64>::new(1));
    let strategy_inventory = Arc::clone(&inventory_queue);
    let oms_inventory = Arc::clone(&inventory_queue);

    let private_ws_thread = create_private_ws_thread(
        order_events_tx.clone(),
        recorder_events_tx,
        Arc::clone(&config),
    )?;
    let oms_thread = create_oms_thread(
        runtime_handle,
        strategy_orders_rx,
        order_events_rx,
        order_events_tx,
        oms_inventory,
        Arc::clone(&config),
    )?;
    let recorder_thread = create_recorder_thread(recorder_order_books, recorder_events_rx)?;

    // NOTE: Start the strategy last, after everything else has initialized.
    // TODO: should I add a delay?
    let strategy_thread = create_strategy_thread(
        strategy_orders_tx,
        strategy_inventory,
        order_book_output,
        config,
    )?;

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
