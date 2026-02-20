use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

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
        // This is a very simple risk management. Don't have more than two orders
        // running at the same time.
        let num_active_orders = self
            .active_orders
            .iter()
            .filter(|(_, o)| o.order_status.is_open())
            .count();
        while let Ok(order_builder) = self.from_strategy.try_recv() {
            if num_active_orders < 2 {
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
                    old_order.order_status = new_order.order_status;
                    old_order.filled_price = new_order.filled_price;
                    old_order.filled_qty = new_order.filled_qty;
                }
                None => {
                    // NOTE: `insert` returns an option, but we don't need the result, therefore
                    // `unwrap()` isn't called.
                    self.active_orders
                        .insert(new_order.order_id.clone(), new_order);
                }
            };
        }
    }
}
