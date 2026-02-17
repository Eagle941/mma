use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};

use exchange::bybit::order::OrderHandler;
use exchange::{Order, OrderBuilder};

#[derive(Debug)]
pub struct OrderManagementSystem {
    from_strategy: Receiver<OrderBuilder>,
    from_order_handler: Receiver<Order>,
    order_handler: OrderHandler,
    // TODO: add internal order_id instead of using the one supplied by the
    // exchange.
    active_orders: HashMap<String, Order>,
}
impl OrderManagementSystem {
    pub fn new(from_strategy: Receiver<OrderBuilder>) -> OrderManagementSystem {
        let (to_oms, from_order_handler): (Sender<Order>, Receiver<Order>) = mpsc::channel();
        OrderManagementSystem {
            from_strategy,
            from_order_handler,
            order_handler: OrderHandler::new(to_oms),
            active_orders: HashMap::new(),
        }
    }

    pub fn cycle(&mut self) {
        loop {
            self.forward_orders();
            self.order_response();
        }
    }

    pub fn forward_orders(&self) {
        while let Ok(order_builder) = self.from_strategy.try_recv() {
            self.order_handler.submit_order(order_builder);
        }
    }

    pub fn order_response(&mut self) {
        // The logic doesn't consider order updates yet (where the key exists
        // already).
        while let Ok(order) = self.from_order_handler.try_recv() {
            self.active_orders.insert(order.order_id.clone(), order);
        }
    }
}
