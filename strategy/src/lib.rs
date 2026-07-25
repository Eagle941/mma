use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
use crossbeam_queue::ArrayQueue;
use exchange::{OrderBook, OrderBuilder};
use triple_buffer::Output;

pub mod simple;

pub trait Strategy {
    fn execute(&mut self, order_book: &OrderBook, inventory: f64) -> Vec<OrderBuilder>;
}

pub struct StrategyRunner {
    strategy: Box<dyn Strategy>,
    orders_tx: Sender<OrderBuilder>,
    inventory: Arc<ArrayQueue<f64>>,
    order_book: Output<OrderBook>,
}

impl StrategyRunner {
    pub fn new(
        strategy: Box<dyn Strategy>,
        orders_tx: Sender<OrderBuilder>,
        inventory: Arc<ArrayQueue<f64>>,
        order_book: Output<OrderBook>,
    ) -> Self {
        Self {
            strategy,
            orders_tx,
            inventory,
            order_book,
        }
    }

    pub fn cycle(&mut self) {
        let mut inventory = self.inventory.pop().unwrap_or(0.0);
        loop {
            // NOTE: strategy is executed at around 1Hz for learning
            inventory = self.inventory.pop().unwrap_or(inventory);
            let order_book = self.order_book.read();
            self.strategy
                .execute(order_book, inventory)
                .into_iter()
                .for_each(|order| self.orders_tx.send(order).unwrap());
            thread::sleep(Duration::from_secs(1));
        }
    }
}
