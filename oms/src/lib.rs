use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use exchange::bybit::order::OrderHandler;
use exchange::{Order, OrderBuilder, OrderStatus};

use crate::risk::RiskManager;

pub mod risk;

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
    pub fn new(
        from_strategy: Receiver<OrderBuilder>,
        from_order_handler: Receiver<Order>,
        to_oms: Sender<Order>,
    ) -> OrderManagementSystem {
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

    /// This function is responsible for receiving the order commands from the
    /// strategy and forwarding them to the exchange.
    pub fn forward_orders(&self) {
        while let Ok(order_builder) = self.from_strategy.try_recv() {
            if RiskManager::submit_order(&self.active_orders, &order_builder) {
                self.order_handler.submit_order(order_builder);
            }
        }
    }

    /// This function is responsible for recording the latest updates to the
    /// orders submitted to the exchange. It populates the `active_orders`
    /// HashMap as soon as the order has been submitted successfully to the
    /// exchange. Further order updates are received from the orders WebSocket.
    pub fn order_response(&mut self) {
        while let Ok(new_order) = self.from_order_handler.try_recv() {
            // TODO: optimise insert or update logic.
            match self.active_orders.get_mut(&new_order.order_id) {
                Some(old_order) => {
                    if new_order.order_status == OrderStatus::NotAvailable {
                        // This is an amended order.
                        old_order.price = new_order.price;
                        old_order.qty = new_order.price;
                    } else {
                        // This is an order update from the WebSocket
                        old_order.order_status = new_order.order_status;
                        old_order.filled_price = new_order.filled_price;
                        old_order.filled_qty = new_order.filled_qty;
                    }
                }
                None => {
                    // NOTE: `insert` returns an option, but we don't need the
                    // result, therefore `unwrap()` isn't called.
                    // NOTE: `from_bot` is set true only when the order is
                    // generated from the bot. This is to differentiate from
                    // orders generated from the UI.
                    if new_order.from_bot {
                        self.active_orders
                            .insert(new_order.order_id.clone(), new_order);
                    }
                }
            };
        }
    }
}
