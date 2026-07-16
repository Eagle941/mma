use std::f64;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::Receiver;
use crossbeam_queue::ArrayQueue;
use exchange::{Order, OrderBuilder, OrderExecution, OrderGateway, OrderMessages, OrderSide};
use log::{info, warn};
use rustc_hash::FxHashMap;
use slab::Slab;

use crate::risk::{Outcome, RiskManager, RiskPolicy};

pub mod risk;

#[derive(Debug)]
pub struct OmsConfig {
    coin: String,
    inventory: f64,
    avg_entry_price: f64,
    initial_order_link_id: u64,
}
impl OmsConfig {
    pub fn new(
        coin: impl Into<String>,
        inventory: f64,
        avg_entry_price: f64,
        initial_order_link_id: u64,
    ) -> Self {
        OmsConfig {
            coin: coin.into(),
            inventory,
            avg_entry_price,
            initial_order_link_id,
        }
    }
}

#[derive(Debug)]
pub struct OrderManagementSystem {
    from_strategy: Receiver<OrderBuilder>,
    from_order_handler: Receiver<OrderMessages>,
    to_strategy: Arc<ArrayQueue<f64>>,
    order_gateway: Box<dyn OrderGateway>,
    // TODO: the Slab will grow infinitely. It needs to be pruned when orders are completed.
    orders: Slab<Order>,
    // NOTE: at the moment it supports only one pair (ADAUSDT)
    // +ve --> purchased ADA coins
    // -ve --> sold ADA coins
    // A value of 0 shows no exposure to the market i.e. all positions closed.
    inventory: f64,
    avg_entry_price: f64,
    coin: String,
    //
    id_map: FxHashMap<u64, usize>,
    id_generator: AtomicU64,
}
impl OrderManagementSystem {
    pub fn new(
        from_strategy: Receiver<OrderBuilder>,
        from_order_handler: Receiver<OrderMessages>,
        to_strategy: Arc<ArrayQueue<f64>>,
        order_gateway: Box<dyn OrderGateway>,
        config: OmsConfig,
    ) -> OrderManagementSystem {
        // NOTE: pushing to recover strategy with the correct inventory
        to_strategy.force_push(config.inventory);

        OrderManagementSystem {
            from_strategy,
            from_order_handler,
            to_strategy,
            order_gateway,
            orders: Slab::with_capacity(5),
            // NOTE: may be useful to keep track of past_orders
            inventory: config.inventory,
            avg_entry_price: config.avg_entry_price,
            coin: config.coin,
            //
            id_map: FxHashMap::default(),
            id_generator: AtomicU64::new(config.initial_order_link_id),
        }
    }

    pub fn cycle(&mut self) {
        let risk_manager = RiskManager;

        loop {
            crossbeam_channel::select! {
                recv(self.from_strategy) -> msg => {
                    if let Ok(order_builder) = msg {
                        info!("Received order {:?}", order_builder.side);
                        self.forward_orders(order_builder, &risk_manager);
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

    /// This function generates and assigns a new order link ID and stores the
    /// new order.
    /// Returns the assigned order link ID.
    fn insert_new_order(&mut self, order: &OrderBuilder) -> u64 {
        // TODO: Prevent counter overflow from reusing an order link ID and
        // overwriting its existing `id_map` entry.
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
            // NOTE: On a Buy, the fee is paid in the base asset (e.g., ADA). We must subtract it.
            // On a Sell, the fee is paid in the quote asset (USDT), no additional fee to be
            // removed.
            OrderSide::Buy => inventory + execution_update.exec_qty - execution_update.exec_fee,
            OrderSide::Sell => inventory - execution_update.exec_qty,
        };

        if inventory.abs() < 1e-8 {
            return (execution_update.exec_price, new_inventory);
        } else if (inventory > 0.0 && order_side == OrderSide::Buy)
            || (inventory < 0.0 && order_side == OrderSide::Sell)
        {
            let total_value = (inventory.abs() * avg_entry_price)
                + (execution_update.exec_qty * execution_update.exec_price);
            return (total_value / new_inventory.abs(), new_inventory);
        } else if new_inventory.abs() < 1e-8 {
            return (0.0, new_inventory);
        } else {
            // NOTE: no need to worry about +/-0.0 because it is check in the first case.
            let crossed_zero = inventory.signum() != new_inventory.signum();

            if crossed_zero {
                return (execution_update.exec_price, new_inventory);
            }
            // If we didn't cross zero avg_entry_price stays the same!
        }

        (avg_entry_price, new_inventory)
    }

    /// This function is responsible for receiving the order commands from the
    /// strategy and forwarding them to the exchange.
    pub fn forward_orders(&mut self, order_builder: OrderBuilder, risk_policy: &dyn RiskPolicy) {
        match risk_policy.evaluate_order(
            &self.orders,
            order_builder,
            self.inventory,
            self.avg_entry_price,
        ) {
            Outcome::NewOrder(order) => {
                let order_link_id = self.insert_new_order(&order);
                self.order_gateway.submit_order(&order, order_link_id)
            }
            Outcome::AmendOrder(order) => self.order_gateway.amend_order(&order),
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

                    // TODO: Add order removal from Slab when they are closed to
                    // clear up the memory.
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
                        order.order_link_id, order.exec_price, order.exec_qty
                    );

                    // NOTE: returning the new value because I can't borrow `self` twice as mutable.
                    let (avg_entry_price, inventory) = Self::update_metrics(
                        self.avg_entry_price,
                        self.inventory,
                        &order,
                        old_order.side,
                    );
                    if self.inventory.is_sign_negative() && inventory.is_sign_positive() {
                        // NOTE: The new inventory is positive, therefore we can repay the borrowed
                        // money. It is assumed it is triggered less than 1
                        // time per second.
                        self.order_gateway.repay_liability(&self.coin);
                    }
                    self.avg_entry_price = avg_entry_price;
                    self.inventory = inventory;
                    self.to_strategy.force_push(self.inventory);
                };
            }
        };

        info!(
            "Inventory {:.3} | Avg price {:.3}",
            self.inventory, self.avg_entry_price
        );
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Ref, RefCell};
    use std::rc::Rc;

    use assert_approx_eq::assert_approx_eq;
    use crossbeam_channel::unbounded;
    use exchange::{OrderAmendedBuilder, OrderStatus, OrderType};
    use rstest::rstest;

    use super::*;

    #[derive(Clone, Debug, Default)]
    struct TestOrderGateway {
        submitted: Rc<RefCell<Vec<(OrderBuilder, u64)>>>,
    }
    impl TestOrderGateway {
        fn submitted_orders(&self) -> Ref<'_, Vec<(OrderBuilder, u64)>> {
            self.submitted.borrow()
        }
    }
    impl OrderGateway for TestOrderGateway {
        fn submit_order(&self, order: &OrderBuilder, order_link_id: u64) {
            self.submitted
                .borrow_mut()
                .push((order.clone(), order_link_id));
        }

        fn amend_order(&self, _order: &OrderAmendedBuilder) {
            todo!()
        }

        fn repay_liability(&self, _coin: &str) {
            todo!()
        }

        fn cancel_all(&self) {
            todo!()
        }
    }

    struct NewOrderRiskPolicy;

    impl RiskPolicy for NewOrderRiskPolicy {
        fn evaluate_order(
            &self,
            _orders: &Slab<Order>,
            new_order: OrderBuilder,
            _inventory: f64,
            _average_entry_price: f64,
        ) -> Outcome {
            Outcome::NewOrder(new_order)
        }
    }

    struct DoNothingRiskPolicy;

    impl RiskPolicy for DoNothingRiskPolicy {
        fn evaluate_order(
            &self,
            _orders: &Slab<Order>,
            _new_order: OrderBuilder,
            _inventory: f64,
            _average_entry_price: f64,
        ) -> Outcome {
            Outcome::DoNothing
        }
    }

    struct OmsTestBench {
        oms: OrderManagementSystem,
        order_gateway: Box<TestOrderGateway>,
    }
    impl OmsTestBench {
        fn new(initial_order_link_id: u64) -> OmsTestBench {
            let (_, from_strategy) = unbounded();
            let (_, from_order_handler) = unbounded();
            let to_strategy = Arc::new(ArrayQueue::new(1));
            let config = OmsConfig::new("ADA", 0.0, 0.0, initial_order_link_id);
            let order_gateway = Box::new(TestOrderGateway::default());

            let oms = OrderManagementSystem::new(
                from_strategy,
                from_order_handler,
                to_strategy,
                order_gateway.clone(),
                config,
            );

            OmsTestBench { oms, order_gateway }
        }
    }

    #[test]
    fn forward_orders_stores_and_submits_new_order() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        assert!(test_bench.oms.orders.is_empty());

        let risk_policy = NewOrderRiskPolicy;
        test_bench
            .oms
            .forward_orders(order_builder.clone(), &risk_policy);

        assert_eq!(test_bench.oms.orders.len(), 1);
        assert_eq!(test_bench.oms.id_map.len(), 1);
        let slab_index = *test_bench.oms.id_map.get(&initial_order_link_id).unwrap();
        let stored_order = test_bench.oms.orders.get(slab_index).unwrap();
        assert_eq!(stored_order.order_link_id, initial_order_link_id);
        assert_eq!(stored_order.symbol, "ADAUSDT");
        assert_eq!(stored_order.side, OrderSide::Buy);
        assert_eq!(stored_order.order_type, OrderType::Limit);
        assert_eq!(stored_order.qty, 25.0);
        assert_eq!(stored_order.price, 0.567);

        let submitted_orders = test_bench.order_gateway.submitted_orders();
        assert_eq!(submitted_orders.len(), 1);
        let (submitted_order, submitted_order_link_id) = &submitted_orders[0];
        assert_eq!(*submitted_order_link_id, initial_order_link_id);
        assert_eq!(submitted_order, &order_builder);
    }

    #[test]
    fn forward_orders_does_not_change_state_when_risk_policy_does_nothing() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let initial_inventory = test_bench.oms.inventory;
        let initial_avg_entry_price = test_bench.oms.avg_entry_price;
        let initial_coin = test_bench.oms.coin.clone();
        let initial_next_order_link_id = test_bench.oms.id_generator.load(Ordering::Relaxed);

        let risk_policy = DoNothingRiskPolicy;
        test_bench.oms.forward_orders(order_builder, &risk_policy);

        assert!(test_bench.oms.orders.is_empty());
        assert!(test_bench.oms.id_map.is_empty());
        assert_eq!(test_bench.oms.inventory, initial_inventory);
        assert_eq!(test_bench.oms.avg_entry_price, initial_avg_entry_price);
        assert_eq!(test_bench.oms.coin, initial_coin);
        assert_eq!(
            test_bench.oms.id_generator.load(Ordering::Relaxed),
            initial_next_order_link_id
        );
        assert!(test_bench.order_gateway.submitted_orders().is_empty());
    }

    #[test]
    fn insert_new_order_stores_and_indexes_order() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };

        let order_link_id = test_bench.oms.insert_new_order(&order_builder);

        assert_eq!(order_link_id, initial_order_link_id);
        assert_eq!(test_bench.oms.orders.len(), 1);
        assert_eq!(test_bench.oms.id_map.len(), 1);

        let slab_index = *test_bench.oms.id_map.get(&order_link_id).unwrap();
        let stored_order = test_bench.oms.orders.get(slab_index).unwrap();
        assert_eq!(stored_order.order_link_id, order_link_id);
        assert_eq!(stored_order.symbol, order_builder.symbol);
        assert_eq!(stored_order.side, order_builder.side);
        assert_eq!(stored_order.order_type, order_builder.order_type);
        assert_eq!(stored_order.qty, order_builder.qty);
        assert_eq!(stored_order.price, 0.567);
        assert_eq!(stored_order.order_status, OrderStatus::Submitted);
        assert_eq!(stored_order.filled_qty, 0.0);
        assert!(stored_order.filled_price.is_nan());
        assert_eq!(stored_order.updated_time, 0);
    }

    #[test]
    fn insert_new_order_generates_distinct_sequential_ids() {
        let mut test_bench = OmsTestBench::new(1000);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };

        let first_id = test_bench.oms.insert_new_order(&order_builder);
        let second_id = test_bench.oms.insert_new_order(&order_builder);
        let third_id = test_bench.oms.insert_new_order(&order_builder);

        assert_eq!([first_id, second_id, third_id], [1000, 1001, 1002]);
        assert_eq!(test_bench.oms.orders.len(), 3);
        assert_eq!(test_bench.oms.id_map.len(), 3);

        for order_link_id in [first_id, second_id, third_id] {
            let slab_index = *test_bench.oms.id_map.get(&order_link_id).unwrap();
            assert_eq!(
                test_bench.oms.orders[slab_index].order_link_id,
                order_link_id
            );
        }
    }

    #[test]
    fn insert_new_order_keeps_distinct_orders_correctly_indexed() {
        let mut test_bench = OmsTestBench::new(1000);
        let buy_order = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let sell_order = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            qty: 40.0,
            price: "0.575".to_string(),
        };

        let buy_id = test_bench.oms.insert_new_order(&buy_order);
        let sell_id = test_bench.oms.insert_new_order(&sell_order);

        let buy_slab_index = *test_bench.oms.id_map.get(&buy_id).unwrap();
        let sell_slab_index = *test_bench.oms.id_map.get(&sell_id).unwrap();
        assert_ne!(buy_slab_index, sell_slab_index);

        let stored_buy = test_bench.oms.orders.get(buy_slab_index).unwrap();
        assert_eq!(stored_buy.order_link_id, buy_id);
        assert_eq!(stored_buy.side, OrderSide::Buy);
        assert_eq!(stored_buy.qty, 25.0);
        assert_eq!(stored_buy.price, 0.567);

        let stored_sell = test_bench.oms.orders.get(sell_slab_index).unwrap();
        assert_eq!(stored_sell.order_link_id, sell_id);
        assert_eq!(stored_sell.side, OrderSide::Sell);
        assert_eq!(stored_sell.qty, 40.0);
        assert_eq!(stored_sell.price, 0.575);
    }

    #[rstest]
    #[case(0.0, 0.0, 0.567, 22.0, OrderSide::Buy, 0.567)]
    #[case(0.0, 0.0, 0.567, 22.0, OrderSide::Sell, 0.567)]
    #[case(1.0, 50.0, 2.0, 50.0, OrderSide::Buy, 1.5)]
    #[case(1.0, 50.0, 1.5, 100.0, OrderSide::Sell, 1.5)]
    fn test_avg_entry_price(
        #[case] avg_entry_price: f64,
        #[case] inventory: f64,
        #[case] exec_price: f64,
        #[case] exec_qty: f64,
        #[case] order_side: OrderSide,
        #[case] expected_avg_entry_price: f64,
    ) {
        let execution_update = OrderExecution {
            order_link_id: 1234,
            exec_price,
            exec_fee: 0.0,
            exec_qty,
            remaining_qty: 50.0,
            exec_id: "abcd".to_string(),
            exec_ts: 1773956505537,
            order_id: "1773956505537".to_string(),
            order_price: exec_price,
            order_side,
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
