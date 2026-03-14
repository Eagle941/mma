use std::f64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use crossbeam_queue::ArrayQueue;
use exchange::bybit::order::OrderHandler;
use exchange::{Order, OrderBuilder, OrderExecution, OrderMessages, OrderSide};
use log::{info, warn};
use rustc_hash::FxHashMap;
use slab::Slab;

use crate::risk::{Outcome, RiskManager};

pub mod risk;

#[derive(Debug)]
pub struct OrderManagementSystem {
    from_strategy: Receiver<OrderBuilder>,
    from_order_handler: Receiver<OrderMessages>,
    to_strategy: Arc<ArrayQueue<f64>>,
    order_handler: OrderHandler,
    // TODO: the Slab will grow infinitely. It needs to be pruned when orders are completed.
    orders: Slab<Order>,
    // NOTE: at the moment it supports only one pair (ADAUSDT)
    // +ve --> purchased ADA coins
    // -ve --> sold ADA coins
    // A value of 0 shows no exposure to the market i.e. all positions closed.
    inventory: f64,
    avg_entry_price: f64,
    //
    id_map: FxHashMap<u64, usize>,
    id_generator: AtomicU64,
}
impl OrderManagementSystem {
    pub fn new(
        from_strategy: Receiver<OrderBuilder>,
        from_order_handler: Receiver<OrderMessages>,
        to_strategy: Arc<ArrayQueue<f64>>,
    ) -> OrderManagementSystem {
        let start_time_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System clock went backwards!")
            .as_micros() as u64;

        OrderManagementSystem {
            from_strategy,
            from_order_handler,
            to_strategy,
            order_handler: OrderHandler::new(),
            orders: Slab::with_capacity(5),
            // NOTE: may be useful to keep track of past_orders
            inventory: 0.0,
            avg_entry_price: 0.0,
            //
            id_map: FxHashMap::default(),
            id_generator: AtomicU64::new(start_time_micros),
        }
    }

    pub fn cycle(&mut self) {
        loop {
            crossbeam_channel::select! {
                recv(self.from_strategy) -> msg => {
                    if let Ok(order_builder) = msg {
                        info!("Received order {:?}", order_builder.side);
                        self.forward_orders(order_builder);
                    }
                },
                recv(self.from_order_handler) -> msg => {
                    if let Ok(new_order) = msg {
                        self.order_response(new_order);
                    }
                }
            }
        }
    }

    fn insert_new_order(&mut self, order: &OrderBuilder) -> u64 {
        let next_order_link_id = self.id_generator.fetch_add(1, Ordering::Relaxed);
        let entry = self.orders.vacant_entry();
        let slab_index = entry.key();
        entry.insert(order.build(next_order_link_id));
        self.id_map.insert(next_order_link_id, slab_index);
        next_order_link_id
    }

    /// This function calculates the new average entry price given the latest
    /// execution update from the exchange.
    /// This function takes the inventory value before it is updated with the
    /// execution update.
    /// The average entry price takes into account change of side from buy to
    /// sell and vice-versa.
    fn update_metrics(
        avg_entry_price: f64,
        inventory: f64,
        execution_update: &OrderExecution,
        order_side: OrderSide,
    ) -> (f64, f64) {
        let new_inventory = match order_side {
            OrderSide::Buy => inventory + execution_update.qty,
            OrderSide::Sell => inventory - execution_update.qty,
        };

        if inventory.abs() < 1e-8 {
            return (execution_update.price, new_inventory);
        } else if (inventory > 0.0 && order_side == OrderSide::Buy)
            || (inventory < 0.0 && order_side == OrderSide::Sell)
        {
            let total_value = (inventory.abs() * avg_entry_price)
                + (execution_update.qty * execution_update.price);
            return (total_value / new_inventory.abs(), new_inventory);
        } else if new_inventory.abs() < 1e-8 {
            return (0.0, new_inventory);
        } else {
            // NOTE: no need to worry about +/-0.0 because it is check in the first case.
            let crossed_zero = inventory.signum() != new_inventory.signum();

            if crossed_zero {
                return (execution_update.price, new_inventory);
            }
            // If we didn't cross zero avg_entry_price stays the same!
        }

        (avg_entry_price, new_inventory)
    }

    /// This function is responsible for receiving the order commands from the
    /// strategy and forwarding them to the exchange.
    pub fn forward_orders(&mut self, order_builder: OrderBuilder) {
        match RiskManager::submit_order(
            &self.orders,
            order_builder,
            self.inventory,
            self.avg_entry_price,
        ) {
            Outcome::NewOrder(order) => {
                let order_link_id = self.insert_new_order(&order);
                self.order_handler.submit_order(&order, order_link_id)
            }
            Outcome::AmendOrder(order) => self.order_handler.amend_order(&order),
            Outcome::DoNothing => (),
        };
    }

    /// This function is responsible for recording the latest updates to the
    /// orders submitted to the exchange. It populates the `active_orders`
    /// HashMap as soon as the order has been submitted successfully to the
    /// exchange. Further order updates are received from the orders WebSocket.
    pub fn order_response(&mut self, new_order: OrderMessages) {
        // TODO: optimise insert or update logic.
        match new_order {
            OrderMessages::OrderUpdate(order) => {
                let Some(slab_id) = self.id_map.get(&order.order_link_id) else {
                    warn!("DISCARDED updated order {}", &order.order_link_id);
                    return;
                };
                // NOTE: assuming order exists already!
                if let Some(old_order) = self.orders.get_mut(*slab_id) {
                    // NOTE: this is to prevent manual orders on the UI to
                    // affect the logic of the bot.
                    info!(
                        "Updated order {} {:?} {:.3} {:.0}",
                        order.order_link_id,
                        order.order_status,
                        order.filled_price,
                        order.filled_qty
                    );

                    old_order.price = order.price;
                    old_order.qty = order.qty;
                    old_order.order_status = order.order_status;
                    old_order.filled_price = order.filled_price;
                    old_order.filled_qty = order.filled_qty;
                    old_order.updated_time = order.updated_time;
                };
            }
            OrderMessages::ExecutionUpdate(order) => {
                let Some(slab_id) = self.id_map.get(&order.order_link_id) else {
                    warn!("DISCARDED execution order {}", &order.order_link_id);
                    return;
                };
                // NOTE: assuming order exists already!
                if let Some(old_order) = self.orders.get_mut(*slab_id) {
                    // NOTE: this is to prevent manual orders on the UI to
                    // affect the logic of the bot.
                    info!(
                        "Execution order {} {:.3} {:.0}",
                        order.order_link_id, order.price, order.qty
                    );

                    // NOTE: returning the new value because I can't borrow `self` twice as mutable.
                    (self.avg_entry_price, self.inventory) = Self::update_metrics(
                        self.avg_entry_price,
                        self.inventory,
                        &order,
                        old_order.side,
                    );
                    self.to_strategy.force_push(self.inventory);
                };
            }
        };

        info!("Inventory {:.3}", self.inventory);
    }
}

#[cfg(test)]
mod tests {
    use assert_approx_eq::assert_approx_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(0.0, 0.0, 0.567, 22.0, OrderSide::Buy, 0.567)]
    #[case(0.0, 0.0, 0.567, 22.0, OrderSide::Sell, 0.567)]
    #[case(1.0, 50.0, 2.0, 50.0, OrderSide::Buy, 1.5)]
    #[case(1.0, 50.0, 1.5, 100.0, OrderSide::Sell, 1.5)]
    fn test_avg_entry_price(
        #[case] avg_entry_price: f64,
        #[case] inventory: f64,
        #[case] price: f64,
        #[case] qty: f64,
        #[case] order_side: OrderSide,
        #[case] expected_avg_entry_price: f64,
    ) {
        let execution_update = OrderExecution {
            order_link_id: 1234,
            price,
            qty,
            remaining_qty: 50.0,
        };

        let new_metrics = OrderManagementSystem::update_metrics(
            avg_entry_price,
            inventory,
            &execution_update,
            order_side,
        );
        assert_approx_eq!(new_metrics.0, expected_avg_entry_price);
    }
}
