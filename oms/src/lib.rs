use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use exchange::bybit::order::OrderHandler;
use exchange::{Order, OrderBuilder, OrderMessages, OrderSide, OrderStatus};
use rustc_hash::FxHashMap;
use slab::Slab;

use crate::risk::{Outcome, RiskManager};

pub mod risk;

#[derive(Debug)]
pub struct OrderManagementSystem {
    from_strategy: Receiver<OrderBuilder>,
    from_order_handler: Receiver<OrderMessages>,
    order_handler: OrderHandler,
    // TODO: add internal order_id instead of using the one supplied by the
    // exchange.
    active_orders: Slab<Order>,
    // NOTE: at the moment it supports only one pair (ADAUSDT)
    // +ve --> purchased ADA coins
    // -ve --> sold ADA coins
    // A value of 0 shows no exposure to the market i.e. all positions closed.
    inventory: f64,
    last_fill_buy: Option<Order>,
    last_fill_sell: Option<Order>,
    //
    id_map: FxHashMap<u64, usize>,
    id_generator: AtomicU64,
}
impl OrderManagementSystem {
    pub fn new(
        from_strategy: Receiver<OrderBuilder>,
        from_order_handler: Receiver<OrderMessages>,
    ) -> OrderManagementSystem {
        let start_time_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System clock went backwards!")
            .as_micros() as u64;

        OrderManagementSystem {
            from_strategy,
            from_order_handler,
            order_handler: OrderHandler::new(),
            active_orders: Slab::with_capacity(5),
            // NOTE: may be useful to keep track of past_orders
            inventory: 0.0,
            last_fill_buy: None,
            last_fill_sell: None,
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

    /// This function is responsible for receiving the order commands from the
    /// strategy and forwarding them to the exchange.
    pub fn forward_orders(&mut self, order_builder: OrderBuilder) {
        match RiskManager::submit_order(
            &self.active_orders,
            order_builder,
            self.inventory,
            self.last_fill_buy.as_ref(),
            self.last_fill_sell.as_ref(),
        ) {
            Outcome::NewOrder(order) => {
                // NOTE: can be moved in separate function and return the `next_order_link_id`
                let next_order_link_id = self.id_generator.fetch_add(1, Ordering::Relaxed);
                println!("next_order_link_id {next_order_link_id}");
                let entry = self.active_orders.vacant_entry();
                let slab_index = entry.key();
                entry.insert(order.build(next_order_link_id));
                self.id_map.insert(next_order_link_id, slab_index);
                self.order_handler.submit_order(&order, next_order_link_id)
            }
            Outcome::AmendOrder(order) => self.order_handler.amend_order(&order),
            Outcome::Nothing => (),
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
                    println!("DISCARDED updated order {}", &order.order_link_id);
                    return;
                };
                // NOTE: assuming order exists already!
                if let Some(old_order) = self.active_orders.get_mut(*slab_id) {
                    println!(
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

                    if order.order_status == OrderStatus::Filled {
                        match old_order.side {
                            OrderSide::Buy => self.last_fill_buy = Some(old_order.clone()),
                            OrderSide::Sell => self.last_fill_sell = Some(old_order.clone()),
                        }
                    }
                };
            }
            OrderMessages::ExecutionUpdate(order) => {
                let Some(slab_id) = self.id_map.get(&order.order_link_id) else {
                    println!("DISCARDED execution order {}", &order.order_link_id);
                    return;
                };
                // NOTE: assuming order exists already!
                if let Some(old_order) = self.active_orders.get_mut(*slab_id) {
                    // NOTE: this is to prevent manual orders on the UI to
                    // affect the logic of the bot.

                    println!(
                        "Execution order {} {:.3} {:.0}",
                        order.order_link_id, order.price, order.qty
                    );

                    match old_order.side {
                        OrderSide::Buy => self.inventory += order.qty,
                        OrderSide::Sell => self.inventory -= order.qty,
                    };
                };
            }
        };

        println!("Inventory {:.3}", self.inventory);
    }
}
