use std::io;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use configuration::SharedAppConfig;
use crossbeam_channel::{Receiver, Sender};
use crossbeam_queue::ArrayQueue;
use exchange::bybit::market::{Info, Trades};
use exchange::bybit::order::OrderHandler;
use exchange::bybit::private_ws::PrivateWebSocket;
use exchange::bybit::public_ws::PublicWebSocket;
use exchange::bybit::wallet::Wallet;
use exchange::{OrderBook, OrderBuilder, OrderEvent};
use oms::{OmsConfig, OrderManagementSystem};
use recorder::MarkoutEngine;
use strategy::simple::SimpleStrategy;
use triple_buffer::{Input, Output};

fn spawn_named(name: &str, worker: impl FnOnce() + Send + 'static) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().name(name.to_string()).spawn(worker)
}

pub(super) fn create_public_ws_thread(
    recorder_order_books: Arc<ArrayQueue<OrderBook>>,
    mut order_book_input: Input<OrderBook>,
    config: SharedAppConfig,
) -> io::Result<JoinHandle<()>> {
    spawn_named("public_ws_thread", move || {
        let mut public_websocket = PublicWebSocket::new(recorder_order_books, config.as_ref());
        public_websocket.subscribe(&mut order_book_input, config.symbol());
    })
}

pub(super) fn create_private_ws_thread(
    order_events_tx: Sender<OrderEvent>,
    recorder_events_tx: Sender<OrderEvent>,
    config: SharedAppConfig,
) -> io::Result<JoinHandle<()>> {
    spawn_named("private_ws_thread", move || {
        let private_websocket =
            PrivateWebSocket::new(order_events_tx, recorder_events_tx, config.as_ref());
        private_websocket.subscribe();
    })
}

pub(super) fn create_oms_thread(
    runtime_handle: tokio::runtime::Handle,
    strategy_orders_rx: Receiver<OrderBuilder>,
    order_events_rx: Receiver<OrderEvent>,
    gateway_events_tx: Sender<OrderEvent>,
    oms_inventory: Arc<ArrayQueue<f64>>,
    config: SharedAppConfig,
) -> io::Result<JoinHandle<()>> {
    spawn_named("oms_thread", move || {
        let runtime_guard = runtime_handle.enter();

        let wallet = Wallet::new(config.as_ref());
        // TODO: infer the coin from the `base_coin` field of instrument info.
        let inventory = wallet.coins.get(config.coin()).copied().unwrap_or(0.0);
        let avg_entry_price = if inventory == 0.0 {
            0.0
        } else {
            Trades::new(config.as_ref()).last_price
        };
        let next_order_link_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System clock went backwards!")
            .as_micros() as u64;
        let oms_config = OmsConfig::new(
            config.coin(),
            inventory,
            avg_entry_price,
            next_order_link_id,
        );
        let order_handler = Box::new(OrderHandler::new(gateway_events_tx, config.as_ref()));
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
    spawn_named("recorder_thread", move || {
        let mut recorder = MarkoutEngine::new(recorder_order_books, recorder_events_rx);
        recorder.cycle();
    })
}

pub(super) fn create_strategy_thread(
    strategy_orders_tx: Sender<OrderBuilder>,
    strategy_inventory: Arc<ArrayQueue<f64>>,
    mut order_book_output: Output<OrderBook>,
    config: SharedAppConfig,
) -> io::Result<JoinHandle<()>> {
    spawn_named("strategy_thread", move || {
        let instrument_info = Info::new(config.as_ref());
        let mut simple_strategy = SimpleStrategy::new(
            strategy_orders_tx,
            strategy_inventory,
            config.as_ref(),
            instrument_info,
        );
        loop {
            // NOTE: strategy is executed at around 1Hz for learning
            let order_book = order_book_output.read();
            simple_strategy.execute(order_book);
            thread::sleep(Duration::from_secs(1));
        }
    })
}
