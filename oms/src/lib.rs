use std::collections::HashMap;

use exchange::{Order, bybit::order::OrderHandler};

pub struct OrderManagementSystem {
    order_handler: OrderHandler,
    _active_orders: HashMap<String, Order>,
}
impl OrderManagementSystem {
    pub fn new() -> OrderManagementSystem {
        let base_url = "https://api-testnet.bybit.com";
        let api_key = "xxxxxxxx";
        let api_secret = "xxxxxxxxxxx";
        OrderManagementSystem {
            order_handler: OrderHandler::new(
                base_url.to_owned(),
                api_key.to_owned(),
                api_secret.to_owned(),
            ),
            _active_orders: HashMap::new(),
        }
    }

    pub fn submit_order(&self, order: Order) {
        self.order_handler.submit_order(order);
    }
}
