use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};

use exchange::bybit::order::OrderHandler;
use exchange::{Order, OrderBuilder};

#[derive(Debug)]
pub struct OrderManagementSystem {
    from_strategy: Receiver<OrderBuilder>,
    from_order_handler: Receiver<Order>,
    order_handler: OrderHandler,
    _active_orders: HashMap<String, OrderBuilder>,
}
impl OrderManagementSystem {
    pub fn new(from_strategy: Receiver<OrderBuilder>) -> OrderManagementSystem {
        let (to_oms, from_order_handler): (Sender<Order>, Receiver<Order>) = mpsc::channel();
        OrderManagementSystem {
            from_strategy,
            from_order_handler,
            order_handler: OrderHandler::new(to_oms),
            _active_orders: HashMap::new(),
        }
    }

    pub fn forward_orders(&self) {
        // NOTE: recv is blocking the thread.
        while let Ok(order) = self.from_strategy.recv() {
            self.order_handler.submit_order(order);
        }
    }
}
