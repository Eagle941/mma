use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use exchange::bybit::order::OrderHandler;
use exchange::{Order, OrderBuilder, OrderMessages};

use crate::risk::RiskManager;

pub mod risk;

#[derive(Debug)]
pub struct OrderManagementSystem {
    from_strategy: Receiver<OrderBuilder>,
    from_order_handler: Receiver<OrderMessages>,
    order_handler: OrderHandler,
    // TODO: add internal order_id instead of using the one supplied by the
    // exchange.
    active_orders: HashMap<String, Order>,
}
impl OrderManagementSystem {
    pub fn new(
        from_strategy: Receiver<OrderBuilder>,
        from_order_handler: Receiver<OrderMessages>,
        to_oms: Sender<OrderMessages>,
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
            match new_order {
                OrderMessages::NewOrder(order) => {
                    // NOTE: skipping check if the order_id exists already!
                    self.active_orders.insert(order.order_id.clone(), order);
                }
                OrderMessages::AmendedOrder(order) => {
                    // NOTE: assuming order exists already!
                    let old_order = self.active_orders.get_mut(&order.order_id).unwrap();
                    old_order.price = order.price;
                    old_order.qty = order.qty;
                }
                OrderMessages::OrderUpdate(order) => {
                    // NOTE: assuming order exists already!
                    let old_order = self.active_orders.get_mut(&order.order_id).unwrap();
                    old_order.order_status = order.order_status;
                    old_order.filled_price = order.filled_price;
                    old_order.filled_qty = order.filled_qty;
                }
                OrderMessages::ExecutionUpdate(order) => {
                    // NOTE: assuming order exists already!
                    let old_order = self.active_orders.get_mut(&order.order_id).unwrap();
                    old_order.filled_price = order.filled_price;
                    old_order.filled_qty = order.filled_qty;
                }
            };
        }
    }
}
