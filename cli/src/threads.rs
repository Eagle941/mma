use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::{env, io};

use crossbeam_channel::{Receiver, Sender};
use crossbeam_queue::ArrayQueue;
use exchange::bybit::private_ws::PrivateWebSocket;
use exchange::bybit::public_ws::PublicWebSocket;
use exchange::{OrderBook, OrderBuilder, OrderMessages};
use oms::OrderManagementSystem;
use recorder::MarkoutEngine;
use strategy::simple::SimpleStrategy;
use triple_buffer::{Input, Output};

pub(super) fn create_public_ws_thread(
    to_recorder: Arc<ArrayQueue<OrderBook>>,
    mut producer: Input<OrderBook>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("public_ws_thread".to_string())
        .spawn(move || {
            let symbol =
                env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");
            let mut handler = PublicWebSocket::new(to_recorder);
            handler.subscribe(&mut producer, &symbol);
        })
}

pub(super) fn create_private_ws_thread(
    to_oms: Sender<OrderMessages>,
    to_recorder: Sender<OrderMessages>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("private_ws_thread".to_string())
        .spawn(move || {
            let handler = PrivateWebSocket::new(to_oms, to_recorder);
            handler.subscribe();
        })
}

pub(super) fn create_oms_thread(
    runtime_handle: tokio::runtime::Handle,
    from_strategy: Receiver<OrderBuilder>,
    from_order_handler: Receiver<OrderMessages>,
    to_strategy: Arc<ArrayQueue<f64>>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("oms_thread".to_string())
        .spawn(move || {
            let guard = runtime_handle.enter();

            let mut oms =
                OrderManagementSystem::new(from_strategy, from_order_handler, to_strategy);
            oms.cycle();

            drop(guard)
        })
}

pub(super) fn create_recorder_thread(
    from_book: Arc<ArrayQueue<OrderBook>>,
    from_execution: Receiver<OrderMessages>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("recorder_thread".to_string())
        .spawn(move || {
            let mut recorder = MarkoutEngine::new(from_book, from_execution);
            recorder.cycle();
        })
}

pub(super) fn create_strategy_thread(
    to_oms: Sender<OrderBuilder>,
    from_oms: Arc<ArrayQueue<f64>>,
    mut consumer: Output<OrderBook>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("strategy_thread".to_string())
        .spawn(move || {
            let mut simple_strategy = SimpleStrategy::factory(to_oms, from_oms);
            loop {
                // NOTE: strategy is executed at around 1Hz for learning
                let order_book = consumer.read();
                simple_strategy.execute(order_book);
                thread::sleep(Duration::from_millis(1000));
            }
        })
}
