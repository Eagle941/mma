use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::Receiver;
use crossbeam_queue::ArrayQueue;
use exchange::{
    Order,
    OrderBuilder,
    OrderEvent,
    OrderExecution,
    OrderGateway,
    OrderStatus,
    OrderUpdate,
};
use log::{info, warn};
use rustc_hash::{FxHashMap, FxHashSet};
use slab::Slab;

use crate::metrics::Metrics;
use crate::risk::{Outcome, RiskManager, RiskPolicy};

mod metrics;
pub mod risk;

#[derive(Clone, Debug, PartialEq)]
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
    from_order_handler: Receiver<OrderEvent>,
    to_strategy: Arc<ArrayQueue<f64>>,
    order_gateway: Box<dyn OrderGateway>,
    // TODO: the Slab will grow infinitely. It needs to be pruned when orders are completed.
    orders: Slab<Order>,
    metrics: Metrics,
    coin: String,
    //
    id_map: FxHashMap<u64, usize>,
    id_generator: AtomicU64,
    // TODO: Bound or persist execution IDs without allowing late duplicate executions.
    processed_executions: FxHashSet<(u64, String)>,
}
impl OrderManagementSystem {
    pub fn new(
        from_strategy: Receiver<OrderBuilder>,
        from_order_handler: Receiver<OrderEvent>,
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
            metrics: Metrics::new(config.inventory, config.avg_entry_price),
            coin: config.coin,
            //
            id_map: FxHashMap::default(),
            id_generator: AtomicU64::new(config.initial_order_link_id),
            processed_executions: FxHashSet::default(),
        }
    }

    pub fn cycle(&mut self) {
        let risk_manager = RiskManager;

        // TODO: if either channel disconnects, the thread should quit
        loop {
            crossbeam_channel::select! {
                recv(self.from_strategy) -> msg => {
                    if let Ok(order_builder) = msg {
                        info!("Received order {:?}", order_builder.side);
                        self.forward_orders(order_builder, &risk_manager);
                    }
                },
                recv(self.from_order_handler) -> msg => {
                    if let Ok(event) = msg {
                        self.order_response(event);
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

    fn register_execution(&mut self, order_link_id: u64, exec_id: &str) -> bool {
        self.processed_executions
            .insert((order_link_id, exec_id.to_string()))
    }

    /// This function is responsible for receiving the order commands from the
    /// strategy and forwarding them to the exchange.
    pub fn forward_orders(&mut self, order_builder: OrderBuilder, risk_policy: &dyn RiskPolicy) {
        match risk_policy.evaluate_order(
            &self.orders,
            order_builder,
            self.metrics.inventory(),
            self.metrics.average_entry_price(),
        ) {
            Outcome::NewOrder(order) => {
                let order_link_id = self.insert_new_order(&order);
                self.order_gateway.submit_order(&order, order_link_id)
            }
            Outcome::AmendOrder(order) => self.order_gateway.amend_order(&order),
            Outcome::DoNothing => (),
        };
    }

    /// Records order events received from the exchange.
    pub fn order_response(&mut self, event: OrderEvent) {
        match event {
            OrderEvent::OrderUpdate(order) => self.handle_order_update(order),
            OrderEvent::ExecutionUpdate(order) => self.handle_execution_update(order),
            OrderEvent::SubmissionFailed(order_link_id) => {
                self.handle_submission_failure(order_link_id);
            }
        }

        info!(
            "Inventory {:.3} | Avg price {:.3}",
            self.metrics.inventory(),
            self.metrics.average_entry_price()
        );
    }

    fn handle_order_update(&mut self, order: OrderUpdate) {
        let Some(slab_id) = self.id_map.get(&order.order_link_id) else {
            warn!("DISCARDED updated order {}", &order.order_link_id);
            return;
        };
        // NOTE: assuming order exists already!
        if let Some(old_order) = self.orders.get_mut(*slab_id) {
            if order.updated_time < old_order.updated_time {
                warn!("DISCARDED stale update for order {}", order.order_link_id);
                return;
            }

            // NOTE: this is to prevent manual orders on the UI to
            // affect the logic of the bot.
            info!(
                "Updated order {} {:?} {:.3} {:.0}",
                order.order_link_id, order.order_status, order.filled_price, order.filled_qty
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

    fn handle_execution_update(&mut self, order: OrderExecution) {
        if order.exec_id.is_empty() {
            warn!(
                "DISCARDED execution, empty ID for order {}",
                order.order_link_id
            );
            return;
        }

        let Some(&slab_id) = self.id_map.get(&order.order_link_id) else {
            warn!(
                "DISCARDED execution {}, order slab {} not found",
                order.exec_id, order.order_link_id
            );
            return;
        };

        if !self.orders.contains(slab_id) {
            warn!(
                "DISCARDED execution {}, order {} not found",
                order.exec_id, order.order_link_id
            );
            return;
        }

        if !self.register_execution(order.order_link_id, &order.exec_id) {
            warn!(
                "DISCARDED duplicate execution {} for order {}",
                order.exec_id, order.order_link_id
            );
            return;
        }

        // NOTE: this is to prevent manual orders on the UI to
        // affect the logic of the bot.
        info!(
            "Execution order {} {:.3} {:.0}",
            order.order_link_id, order.exec_price, order.exec_qty
        );

        let old_inventory = self.metrics.inventory();
        self.metrics.update(
            order.exec_price,
            order.exec_qty,
            order.exec_fee,
            order.order_side,
        );
        if old_inventory.is_sign_negative() && self.metrics.inventory().is_sign_positive() {
            // NOTE: The new inventory is positive, therefore we can repay the borrowed
            // money. It is assumed it is triggered less than 1
            // time per second.
            self.order_gateway.repay_liability(&self.coin);
        }
        self.to_strategy.force_push(self.metrics.inventory());
    }

    fn handle_submission_failure(&mut self, order_link_id: u64) {
        let Some(&slab_id) = self.id_map.get(&order_link_id) else {
            warn!("DISCARDED submission failure for order {order_link_id}");
            return;
        };
        let Some(old_order) = self.orders.get_mut(slab_id) else {
            warn!("DISCARDED submission failure for missing order {order_link_id}");
            return;
        };
        // TODO: can the following check be removed?
        if old_order.order_status != OrderStatus::Submitted {
            warn!(
                "DISCARDED submission failure for order {order_link_id} in state {:?}",
                old_order.order_status
            );
            return;
        }

        old_order.order_status = OrderStatus::Rejected;
        warn!("Order {order_link_id} rejected during submission");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use assert_approx_eq::assert_approx_eq;
    use crossbeam_channel::unbounded;
    use exchange::{
        OrderAmendedBuilder,
        OrderExecution,
        OrderSide,
        OrderStatus,
        OrderType,
        OrderUpdate,
    };

    use super::*;

    #[derive(Clone, Debug, Default)]
    struct TestOrderGateway {
        submitted: Rc<RefCell<Vec<(OrderBuilder, u64)>>>,
        repaid_coins: Rc<RefCell<Vec<String>>>,
    }
    impl TestOrderGateway {
        fn assert_submitted_once(
            &self,
            expected_order: &OrderBuilder,
            expected_order_link_id: u64,
        ) {
            let submitted = self.submitted.borrow();
            assert_eq!(
                submitted.as_slice(),
                &[(expected_order.clone(), expected_order_link_id)]
            );
        }

        fn assert_no_submissions(&self) {
            assert!(self.submitted.borrow().is_empty());
        }

        fn assert_repaid_once(&self, expected_coin: &str) {
            let repaid_coins = self.repaid_coins.borrow();
            assert_eq!(repaid_coins.as_slice(), &[expected_coin]);
        }

        fn assert_no_repayments(&self) {
            assert!(self.repaid_coins.borrow().is_empty());
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

        fn repay_liability(&self, coin: &str) {
            self.repaid_coins.borrow_mut().push(coin.to_string());
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
            Self::with_config(OmsConfig::new("ADA", 0.0, 0.0, initial_order_link_id))
        }

        fn with_config(config: OmsConfig) -> OmsTestBench {
            let (_, from_strategy) = unbounded();
            let (_, from_order_handler) = unbounded();
            let to_strategy = Arc::new(ArrayQueue::new(1));
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

        fn stored_order(&self, order_link_id: u64) -> &Order {
            let slab_index = self
                .oms
                .id_map
                .get(&order_link_id)
                .expect("order link ID should be indexed");

            self.oms
                .orders
                .get(*slab_index)
                .expect("indexed order should exist in the slab")
        }

        fn submit_new_order(&mut self, order: &OrderBuilder) -> u64 {
            let order_link_id = self.oms.id_generator.load(Ordering::Relaxed);
            self.oms.forward_orders(order.clone(), &NewOrderRiskPolicy);
            order_link_id
        }

        fn assert_metrics(&self, expected_inventory: f64, expected_average_entry_price: f64) {
            assert_approx_eq!(self.oms.metrics.inventory(), expected_inventory);
            assert_approx_eq!(
                self.oms.metrics.average_entry_price(),
                expected_average_entry_price
            );
        }

        fn assert_published_inventory(&self, expected_inventory: f64) {
            let published_inventory = self
                .oms
                .to_strategy
                .pop()
                .expect("OMS should publish inventory");
            assert_approx_eq!(published_inventory, expected_inventory);
            assert!(
                self.oms.to_strategy.is_empty(),
                "OMS should publish only the latest inventory"
            );
        }
    }

    fn execution_update(
        order_link_id: u64,
        exec_id: &str,
        side: OrderSide,
        price: f64,
        quantity: f64,
        fee: f64,
    ) -> OrderExecution {
        OrderExecution {
            order_link_id,
            order_id: "exchange-order-id".to_string(),
            order_price: price,
            order_side: side,
            exec_id: exec_id.to_string(),
            exec_ts: 1_773_956_505_537,
            exec_price: price,
            exec_fee: fee,
            exec_qty: quantity,
            remaining_qty: 0.0,
        }
    }

    fn assert_order_matches_builder(order: &Order, order_link_id: u64, builder: &OrderBuilder) {
        assert_eq!(order.order_link_id, order_link_id);
        assert_eq!(order.symbol, builder.symbol);
        assert_eq!(order.side, builder.side);
        assert_eq!(order.order_type, builder.order_type);
        assert_eq!(order.qty, builder.qty);
        assert_eq!(
            order.price,
            builder
                .price
                .parse::<f64>()
                .expect("test order price should be valid")
        );
    }

    fn assert_submitted_order_matches_builder(
        order: &Order,
        order_link_id: u64,
        builder: &OrderBuilder,
    ) {
        assert_order_matches_builder(order, order_link_id, builder);
        assert_eq!(order.order_status, OrderStatus::Submitted);
        assert_eq!(order.filled_qty, 0.0);
        assert!(order.filled_price.is_nan());
        assert_eq!(order.updated_time, 0);
    }

    fn assert_order_matches_update(order: &Order, builder: &OrderBuilder, update: &OrderUpdate) {
        assert_eq!(order.order_link_id, update.order_link_id);
        assert_eq!(order.symbol, builder.symbol);
        assert_eq!(order.side, builder.side);
        assert_eq!(order.order_type, builder.order_type);
        assert_eq!(order.order_status, update.order_status);
        assert_eq!(order.qty, update.qty);
        assert_eq!(order.price, update.price);
        assert_eq!(order.filled_qty, update.filled_qty);
        assert_eq!(order.filled_price, update.filled_price);
        assert_eq!(order.updated_time, update.updated_time);
    }

    #[test]
    fn new_initializes_oms_from_non_default_config() {
        let (_, from_strategy) = unbounded();
        let (_, from_order_handler) = unbounded();
        let to_strategy = Arc::new(ArrayQueue::new(1));
        let order_gateway = Box::new(TestOrderGateway::default());
        let coin = "BTC";
        let inventory = 12.5;
        let avg_entry_price = 67_500.0;
        let initial_order_link_id = 42;
        let config = OmsConfig::new(coin, inventory, avg_entry_price, initial_order_link_id);

        let oms = OrderManagementSystem::new(
            from_strategy,
            from_order_handler,
            to_strategy,
            order_gateway,
            config,
        );

        assert_eq!(oms.coin, coin);
        assert_eq!(oms.metrics.inventory(), inventory);
        assert_eq!(oms.metrics.average_entry_price(), avg_entry_price);
        assert_eq!(
            oms.id_generator.load(Ordering::Relaxed),
            initial_order_link_id
        );
        assert!(oms.orders.is_empty());
        assert!(oms.id_map.is_empty());
        assert_eq!(oms.to_strategy.pop(), Some(inventory));
        assert!(oms.to_strategy.is_empty());
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

        test_bench
            .oms
            .forward_orders(order_builder.clone(), &NewOrderRiskPolicy);

        assert_eq!(test_bench.oms.orders.len(), 1);
        assert_eq!(test_bench.oms.id_map.len(), 1);
        assert_submitted_order_matches_builder(
            test_bench.stored_order(initial_order_link_id),
            initial_order_link_id,
            &order_builder,
        );
        test_bench
            .order_gateway
            .assert_submitted_once(&order_builder, initial_order_link_id);
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
        let initial_inventory = test_bench.oms.metrics.inventory();
        let initial_avg_entry_price = test_bench.oms.metrics.average_entry_price();
        let initial_coin = test_bench.oms.coin.clone();
        let initial_next_order_link_id = test_bench.oms.id_generator.load(Ordering::Relaxed);

        test_bench
            .oms
            .forward_orders(order_builder, &DoNothingRiskPolicy);

        assert!(test_bench.oms.orders.is_empty());
        assert!(test_bench.oms.id_map.is_empty());
        assert_eq!(test_bench.oms.metrics.inventory(), initial_inventory);
        assert_eq!(
            test_bench.oms.metrics.average_entry_price(),
            initial_avg_entry_price
        );
        assert_eq!(test_bench.oms.coin, initial_coin);
        assert_eq!(
            test_bench.oms.id_generator.load(Ordering::Relaxed),
            initial_next_order_link_id
        );
        test_bench.order_gateway.assert_no_submissions();
    }

    #[test]
    fn submission_failure_rejects_submitted_order() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);

        test_bench
            .oms
            .order_response(OrderEvent::SubmissionFailed(order_link_id));

        assert_eq!(
            test_bench.stored_order(order_link_id).order_status,
            OrderStatus::Rejected
        );
    }

    #[test]
    fn submission_failure_for_unknown_order_does_not_change_oms() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);

        test_bench
            .oms
            .order_response(OrderEvent::SubmissionFailed(9999));

        assert!(test_bench.oms.orders.is_empty());
        assert!(test_bench.oms.id_map.is_empty());
        assert_eq!(
            test_bench.oms.id_generator.load(Ordering::Relaxed),
            initial_order_link_id
        );
    }

    #[test]
    fn submission_failure_does_not_override_confirmed_order() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        let order_update = OrderUpdate {
            order_link_id,
            order_status: OrderStatus::New,
            qty: order_builder.qty,
            price: order_builder.price.parse().unwrap(),
            filled_qty: 0.0,
            filled_price: f64::NAN,
            updated_time: 1_773_956_505_537,
        };
        test_bench
            .oms
            .order_response(OrderEvent::OrderUpdate(order_update));

        test_bench
            .oms
            .order_response(OrderEvent::SubmissionFailed(order_link_id));

        assert_eq!(
            test_bench.stored_order(order_link_id).order_status,
            OrderStatus::New
        );
    }

    #[test]
    fn order_response_updates_existing_order() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        assert_submitted_order_matches_builder(
            test_bench.stored_order(order_link_id),
            order_link_id,
            &order_builder,
        );

        let order_update = OrderUpdate {
            order_link_id: initial_order_link_id,
            order_status: OrderStatus::PartiallyFilled,
            qty: 30.0,
            price: 0.568,
            filled_qty: 10.0,
            filled_price: 0.5675,
            updated_time: 1773956505537,
        };
        test_bench
            .oms
            .order_response(OrderEvent::OrderUpdate(order_update));

        assert_order_matches_update(
            test_bench.stored_order(order_link_id),
            &order_builder,
            &order_update,
        );
    }

    #[test]
    fn order_response_ignores_order_update_older_than_stored_state() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        let newer_update = OrderUpdate {
            order_link_id,
            order_status: OrderStatus::Filled,
            qty: 30.0,
            price: 0.568,
            filled_qty: 30.0,
            filled_price: 0.5675,
            updated_time: 200,
        };
        let older_update = OrderUpdate {
            order_link_id,
            order_status: OrderStatus::PartiallyFilled,
            qty: 25.0,
            price: 0.567,
            filled_qty: 10.0,
            filled_price: 0.567,
            updated_time: 100,
        };
        test_bench
            .oms
            .order_response(OrderEvent::OrderUpdate(newer_update));

        test_bench
            .oms
            .order_response(OrderEvent::OrderUpdate(older_update));

        assert_order_matches_update(
            test_bench.stored_order(order_link_id),
            &order_builder,
            &newer_update,
        );
    }

    #[test]
    fn order_response_ignores_unknown_order_update() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        let initial_inventory = test_bench.oms.metrics.inventory();
        let initial_avg_entry_price = test_bench.oms.metrics.average_entry_price();
        let initial_coin = test_bench.oms.coin.clone();
        let initial_next_order_link_id = test_bench.oms.id_generator.load(Ordering::Relaxed);
        let unknown_order_update = OrderUpdate {
            order_link_id: 9999,
            order_status: OrderStatus::Filled,
            qty: 50.0,
            price: 0.6,
            filled_qty: 50.0,
            filled_price: 0.6,
            updated_time: 1773956505537,
        };

        test_bench
            .oms
            .order_response(OrderEvent::OrderUpdate(unknown_order_update));

        assert_eq!(test_bench.oms.orders.len(), 1);
        assert_eq!(test_bench.oms.id_map.len(), 1);
        assert_submitted_order_matches_builder(
            test_bench.stored_order(order_link_id),
            order_link_id,
            &order_builder,
        );
        test_bench.assert_metrics(initial_inventory, initial_avg_entry_price);
        assert_eq!(test_bench.oms.coin, initial_coin);
        assert_eq!(
            test_bench.oms.id_generator.load(Ordering::Relaxed),
            initial_next_order_link_id
        );
        test_bench
            .order_gateway
            .assert_submitted_once(&order_builder, order_link_id);
    }

    #[test]
    fn order_response_ignores_unknown_execution_update() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let execution_update =
            execution_update(9999, "execution-id", OrderSide::Buy, 0.566, 10.0, 0.01);

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update));

        assert!(test_bench.oms.orders.is_empty());
        assert!(test_bench.oms.id_map.is_empty());
        test_bench.assert_metrics(0.0, 0.0);
        assert_eq!(
            test_bench.oms.id_generator.load(Ordering::Relaxed),
            initial_order_link_id
        );
    }

    #[test]
    fn order_response_does_not_register_execution_for_unknown_order() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let execution_update = execution_update(
            initial_order_link_id,
            "execution-id",
            OrderSide::Buy,
            0.566,
            10.0,
            0.01,
        );

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));
        test_bench.assert_metrics(0.0, 0.0);

        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        assert_eq!(order_link_id, initial_order_link_id);
        test_bench.assert_published_inventory(0.0);

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));

        let expected_inventory = execution_update.exec_qty - execution_update.exec_fee;
        test_bench.assert_metrics(expected_inventory, execution_update.exec_price);
        test_bench.assert_published_inventory(expected_inventory);
    }

    #[test]
    fn order_response_applies_execution_update() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        test_bench.assert_published_inventory(0.0);
        let execution_update = execution_update(
            order_link_id,
            "execution-id",
            OrderSide::Buy,
            0.566,
            10.0,
            0.01,
        );

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));

        let expected_inventory = execution_update.exec_qty - execution_update.exec_fee;
        test_bench.assert_metrics(expected_inventory, execution_update.exec_price);
        test_bench.assert_published_inventory(expected_inventory);
        assert_eq!(test_bench.oms.orders.len(), 1);
        assert_eq!(test_bench.oms.id_map.len(), 1);
        test_bench.order_gateway.assert_no_repayments();
    }

    #[test]
    fn order_response_does_not_apply_duplicate_execution_to_metrics() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        test_bench.assert_published_inventory(0.0);
        let execution_update = execution_update(
            order_link_id,
            "execution-id",
            OrderSide::Buy,
            0.566,
            10.0,
            0.01,
        );

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));
        let expected_inventory = execution_update.exec_qty - execution_update.exec_fee;
        test_bench.assert_metrics(expected_inventory, execution_update.exec_price);
        test_bench.assert_published_inventory(expected_inventory);

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));

        test_bench.assert_metrics(expected_inventory, execution_update.exec_price);
    }

    #[test]
    fn order_response_does_not_publish_inventory_for_duplicate_execution() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        test_bench.assert_published_inventory(0.0);
        let execution_update = execution_update(
            order_link_id,
            "execution-id",
            OrderSide::Buy,
            0.566,
            10.0,
            0.01,
        );

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));
        let expected_inventory = execution_update.exec_qty - execution_update.exec_fee;
        test_bench.assert_published_inventory(expected_inventory);

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update));

        assert!(test_bench.oms.to_strategy.is_empty());
    }

    #[test]
    fn order_response_applies_sell_execution_update() {
        let initial_order_link_id = 1000;
        let initial_inventory = 50.0;
        let initial_avg_entry_price = 0.5;
        let mut test_bench = OmsTestBench::with_config(OmsConfig::new(
            "ADA",
            initial_inventory,
            initial_avg_entry_price,
            initial_order_link_id,
        ));
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        test_bench.assert_published_inventory(initial_inventory);

        let execution_update = execution_update(
            order_link_id,
            "execution-id",
            OrderSide::Sell,
            0.566,
            10.0,
            0.01,
        );
        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));

        let expected_inventory = initial_inventory - execution_update.exec_qty;
        test_bench.assert_metrics(expected_inventory, initial_avg_entry_price);
        test_bench.assert_published_inventory(expected_inventory);
        test_bench.order_gateway.assert_no_repayments();
    }

    #[test]
    fn order_response_repays_liability_when_execution_crosses_from_short_to_long() {
        let initial_order_link_id = 1000;
        let coin = "ADA";
        let initial_inventory = -10.0;
        let mut test_bench = OmsTestBench::with_config(OmsConfig::new(
            coin,
            initial_inventory,
            0.5,
            initial_order_link_id,
        ));
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        test_bench.assert_published_inventory(initial_inventory);
        test_bench.order_gateway.assert_no_repayments();

        let execution_update = execution_update(
            order_link_id,
            "execution-id",
            OrderSide::Buy,
            0.566,
            22.0,
            0.01,
        );
        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));

        let expected_inventory =
            initial_inventory + execution_update.exec_qty - execution_update.exec_fee;
        assert!(expected_inventory.is_sign_positive());
        test_bench.assert_metrics(expected_inventory, execution_update.exec_price);
        test_bench.assert_published_inventory(expected_inventory);
        test_bench.order_gateway.assert_repaid_once(coin);
    }

    #[test]
    fn order_response_replaces_stale_strategy_inventory() {
        let initial_order_link_id = 1000;
        let initial_inventory = 50.0;
        let mut test_bench = OmsTestBench::with_config(OmsConfig::new(
            "ADA",
            initial_inventory,
            0.5,
            initial_order_link_id,
        ));
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        let execution_update = execution_update(
            order_link_id,
            "execution-id",
            OrderSide::Sell,
            0.566,
            10.0,
            0.01,
        );
        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(execution_update.clone()));

        let expected_inventory = initial_inventory - execution_update.exec_qty;
        assert_eq!(test_bench.oms.to_strategy.pop(), Some(expected_inventory));
        assert!(test_bench.oms.to_strategy.is_empty());
    }

    #[test]
    fn order_response_accumulates_multiple_execution_updates() {
        let initial_order_link_id = 1000;
        let mut test_bench = OmsTestBench::new(initial_order_link_id);
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "1.0".to_string(),
        };
        let order_link_id = test_bench.submit_new_order(&order_builder);
        test_bench.assert_published_inventory(0.0);
        let first_execution = execution_update(
            order_link_id,
            "first-execution-id",
            OrderSide::Buy,
            1.0,
            10.0,
            0.1,
        );
        let second_execution = execution_update(
            order_link_id,
            "second-execution-id",
            OrderSide::Buy,
            0.5,
            5.0,
            0.1,
        );

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(first_execution.clone()));
        let inventory_after_first_execution = first_execution.exec_qty - first_execution.exec_fee;
        test_bench.assert_metrics(inventory_after_first_execution, first_execution.exec_price);

        test_bench
            .oms
            .order_response(OrderEvent::ExecutionUpdate(second_execution.clone()));

        let expected_inventory = 14.8;
        let expected_avg_entry_price = 0.833333;
        test_bench.assert_metrics(expected_inventory, expected_avg_entry_price);
        test_bench.assert_published_inventory(expected_inventory);
        test_bench.order_gateway.assert_no_repayments();
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
        assert_submitted_order_matches_builder(
            test_bench.oms.orders.get(slab_index).unwrap(),
            order_link_id,
            &order_builder,
        );
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

        assert_order_matches_builder(
            test_bench.oms.orders.get(buy_slab_index).unwrap(),
            buy_id,
            &buy_order,
        );
        assert_order_matches_builder(
            test_bench.oms.orders.get(sell_slab_index).unwrap(),
            sell_id,
            &sell_order,
        );
    }
}
