use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::{env, io};

use crossbeam_channel::{Receiver, Sender};
use crossbeam_queue::ArrayQueue;
use exchange::bybit::order::OrderHandler;
use exchange::bybit::private_ws::PrivateWebSocket;
use exchange::bybit::public_ws::PublicWebSocket;
use exchange::{OrderBook, OrderBuilder, OrderEvent};
use oms::{OmsConfig, OrderManagementSystem};
use recorder::MarkoutEngine;
use strategy::simple::SimpleStrategy;
use triple_buffer::{Input, Output};

pub(super) fn create_public_ws_thread(
    recorder_order_books: Arc<ArrayQueue<OrderBook>>,
    mut order_book_input: Input<OrderBook>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("public_ws_thread".to_string())
        .spawn(move || {
            let symbol =
                env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");
            let mut public_websocket = PublicWebSocket::new(recorder_order_books);
            public_websocket.subscribe(&mut order_book_input, &symbol);
        })
}

pub(super) fn create_private_ws_thread(
    order_events_tx: Sender<OrderEvent>,
    recorder_events_tx: Sender<OrderEvent>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("private_ws_thread".to_string())
        .spawn(move || {
            let private_websocket = PrivateWebSocket::new(order_events_tx, recorder_events_tx);
            private_websocket.subscribe();
        })
}

pub(super) fn create_oms_thread(
    runtime_handle: tokio::runtime::Handle,
    strategy_orders_rx: Receiver<OrderBuilder>,
    order_events_rx: Receiver<OrderEvent>,
    gateway_events_tx: Sender<OrderEvent>,
    oms_inventory: Arc<ArrayQueue<f64>>,
    oms_config: OmsConfig,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("oms_thread".to_string())
        .spawn(move || {
            let runtime_guard = runtime_handle.enter();

            let order_handler = Box::new(OrderHandler::new(gateway_events_tx));
            let mut oms = OrderManagementSystem::new(
                strategy_orders_rx,
                order_events_rx,
                oms_inventory,
                order_handler,
                oms_config,
            );
            oms.cycle();

            drop(runtime_guard)
        })
}

pub(super) fn create_recorder_thread(
    recorder_order_books: Arc<ArrayQueue<OrderBook>>,
    recorder_events_rx: Receiver<OrderEvent>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("recorder_thread".to_string())
        .spawn(move || {
            let mut recorder = MarkoutEngine::new(recorder_order_books, recorder_events_rx);
            recorder.cycle();
        })
}

pub(super) fn create_strategy_thread(
    strategy_orders_tx: Sender<OrderBuilder>,
    strategy_inventory: Arc<ArrayQueue<f64>>,
    mut order_book_output: Output<OrderBook>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("strategy_thread".to_string())
        .spawn(move || {
            let mut simple_strategy =
                SimpleStrategy::factory(strategy_orders_tx, strategy_inventory);
            loop {
                // NOTE: strategy is executed at around 1Hz for learning
                let order_book = order_book_output.read();
                simple_strategy.execute(order_book);
                thread::sleep(Duration::from_millis(1000));
            }
        })
}
