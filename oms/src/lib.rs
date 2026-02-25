use std::collections::HashMap;

use crossbeam_channel::{Receiver, Sender};
use exchange::bybit::order::OrderHandler;
use exchange::{Order, OrderBuilder, OrderMessages, OrderSide, OrderStatus};

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
    past_orders: HashMap<String, Order>,
    // NOTE: at the moment it supports only one pair (ADAUSDT)
    // +ve --> purchased ADA coins
    // -ve --> sold ADA coins
    // A value of 0 shows no exposure to the market i.e. all positions closed.
    inventory: f64,
    last_fill_buy: Option<Order>,
    last_fill_sell: Option<Order>,
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
            past_orders: HashMap::new(),
            inventory: 0.0,
            last_fill_buy: None,
            last_fill_sell: None,
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
    pub fn forward_orders(&self, order_builder: OrderBuilder) {
        match RiskManager::submit_order(
            &self.active_orders,
            order_builder,
            self.inventory,
            self.last_fill_buy.as_ref(),
            self.last_fill_sell.as_ref(),
        ) {
            risk::Outcome::NewOrder(order) => self.order_handler.submit_order(order),
            risk::Outcome::AmendOrder(order) => self.order_handler.amend_order(order),
            risk::Outcome::Nothing => (),
        }
    }

    /// This function is responsible for recording the latest updates to the
    /// orders submitted to the exchange. It populates the `active_orders`
    /// HashMap as soon as the order has been submitted successfully to the
    /// exchange. Further order updates are received from the orders WebSocket.
    pub fn order_response(&mut self, new_order: OrderMessages) {
        // TODO: optimise insert or update logic.
        match new_order {
            OrderMessages::NewOrder(order) => {
                // NOTE: skipping check if the order_id exists already!
                if let Some(old_order) = self.active_orders.get_mut(&order.order_id) {
                    // NOTE: this happens if an order if filled before it is added to the list
                    // of orders.
                    old_order.symbol = order.symbol;
                    old_order.side = order.side;
                    old_order.order_type = order.order_type;
                    old_order.qty = order.qty;
                    old_order.price = order.price;
                    return;
                }
                println!("New order {order:#?}");
                self.active_orders.insert(order.order_id.clone(), order);
            }
            OrderMessages::AmendedOrder(order) => {
                // NOTE: assuming order exists already!
                let Some(old_order) = self.active_orders.get_mut(&order.order_id) else {
                    // NOTE: this is to prevent manual orders on the UI to
                    // affect the logic of the bot.
                    println!("DISCARDED amended order {}", &order.order_id);
                    return;
                };
                old_order.price = order.price;
                old_order.qty = order.qty;
            }
            OrderMessages::OrderUpdate(order) => {
                // NOTE: assuming order exists already!
                match self.active_orders.get_mut(&order.order_id) {
                    Some(_) => (),
                    None => {
                        // NOTE: this is to prevent manual orders on the UI to
                        // affect the logic of the bot.

                        // NOTE: the order update can arrive faster than a new order is
                        // inserted.
                        println!("DISCARDED updated order {}", &order.order_id);
                        if order.order_status == OrderStatus::Filled {
                            match order.side {
                                OrderSide::Buy => self.inventory += order.filled_qty,
                                OrderSide::Sell => self.inventory -= order.filled_qty,
                                _ => (),
                            };
                        }
                        self.active_orders
                            .insert(order.order_id.clone(), order.clone());
                    }
                };
                let Some(old_order) = self.active_orders.get_mut(&order.order_id) else {
                    return;
                };

                println!(
                    "Updated order {} {:?} {:.3} {:.0}",
                    order.order_id, order.order_status, order.filled_price, order.filled_qty
                );

                old_order.order_status = order.order_status;
                old_order.filled_price = order.filled_price;
                old_order.filled_qty = order.filled_qty;
                old_order.updated_time = order.updated_time;

                if order.order_status.is_closed() {
                    let order = self.active_orders.remove(&order.order_id).unwrap();
                    if order.order_status == OrderStatus::Filled {
                        match order.side {
                            OrderSide::Buy => self.last_fill_buy = Some(order.clone()),
                            OrderSide::Sell => self.last_fill_sell = Some(order.clone()),
                            OrderSide::NotAvailable => (),
                        }
                    }
                    // NOTE: `past_orders` may not be needed.
                    self.past_orders.insert(order.order_id.clone(), order);
                }
            }
            OrderMessages::ExecutionUpdate(order) => {
                // NOTE: assuming order exists already!
                let Some(old_order) = self.active_orders.get_mut(&order.order_id) else {
                    // NOTE: this is to prevent manual orders on the UI to
                    // affect the logic of the bot.
                    println!("DISCARDED execution order {}", &order.order_id);
                    return;
                };

                println!(
                    "Execution order {} {:.3} {:.0}",
                    order.order_id, order.price, order.qty
                );

                match old_order.side {
                    OrderSide::Buy => self.inventory += order.qty,
                    OrderSide::Sell => self.inventory -= order.qty,
                    _ => (),
                };
            }
        };

        println!("Inventory {:.3}", self.inventory);
    }
}
