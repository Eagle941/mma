use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use exchange::Order;
use exchange::bybit::order::OrderHandler;

#[derive(Debug)]
pub struct OrderManagementSystem {
    order_channel: Receiver<Order>,
    order_handler: OrderHandler,
    _active_orders: HashMap<String, Order>,
}
impl OrderManagementSystem {
    pub fn new(order_channel: Receiver<Order>) -> OrderManagementSystem {
        OrderManagementSystem {
            order_channel,
            order_handler: OrderHandler::new(),
            _active_orders: HashMap::new(),
        }
    }

    pub fn forward_orders(&self) {
        // NOTE: recv is blocking the thread.
        while let Ok(order) = self.order_channel.recv() {
            self.order_handler.submit_order(order);
        }
    }
}
