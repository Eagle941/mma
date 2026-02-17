use std::collections::HashMap;

use exchange::Order;
use exchange::bybit::order::OrderHandler;

#[derive(Debug, Default)]
pub struct OrderManagementSystem {
    order_handler: OrderHandler,
    _active_orders: HashMap<String, Order>,
}
impl OrderManagementSystem {
    pub fn new() -> OrderManagementSystem {
        OrderManagementSystem {
            order_handler: OrderHandler::new(),
            _active_orders: HashMap::new(),
        }
    }

    pub fn submit_order(&self, order: Order) {
        self.order_handler.submit_order(order);
    }
}
