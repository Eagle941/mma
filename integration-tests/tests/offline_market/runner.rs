use std::sync::Arc;

use crossbeam_channel::unbounded;
use crossbeam_queue::ArrayQueue;
use exchange::{InstrumentInfo, OrderBook, OrderBuilder, OrderEvent};
use oms::risk::RiskManager;
use oms::{OmsConfig, OrderManagementSystem};
use recorder::{CompletedMarkout, MarkoutEngine};
use strategy::simple::SimpleStrategy;
use strategy::{STRATEGY_INTERVAL_MS, Strategy};

use super::simulated_exchange::SimulatedExchange;

const INITIAL_ORDER_LINK_ID: u64 = 90;

#[derive(Debug)]
pub(crate) struct StrategyDecision {
    pub(crate) cts: u64,
    pub(crate) inventory: f64,
    pub(crate) orders: Vec<OrderBuilder>,
}

pub(crate) struct OfflineRunner {
    market_data: std::vec::IntoIter<OrderBook>,
    strategy: SimpleStrategy,
    risk_manager: RiskManager,
    pub(crate) oms: OrderManagementSystem,
    pub(crate) simulated_exchange: SimulatedExchange,
    recorder: MarkoutEngine,
    pub(crate) strategy_decisions: Vec<StrategyDecision>,
    pub(crate) processed_events: Vec<OrderEvent>,
    pub(crate) completed_markouts: Vec<(u64, CompletedMarkout)>,
}

impl OfflineRunner {
    pub(crate) fn new(order_books: Vec<OrderBook>) -> Self {
        let simulated_exchange = SimulatedExchange::default();
        let (_, strategy_orders_rx) = unbounded();
        let (_, order_events_rx) = unbounded();
        let inventory_updates = Arc::new(ArrayQueue::new(1));
        let oms = OrderManagementSystem::new(
            strategy_orders_rx,
            order_events_rx,
            inventory_updates,
            Box::new(simulated_exchange.clone()),
            OmsConfig::new("ADA", 0.0, 0.0, INITIAL_ORDER_LINK_ID),
        );

        let (_, recorder_events_rx) = unbounded();
        let recorder = MarkoutEngine::new(Arc::new(ArrayQueue::new(1)), recorder_events_rx);
        let instrument_info = InstrumentInfo::new(
            "ADAUSDT".to_string(),
            "ADA".to_string(),
            "USDT".to_string(),
            1.0,
            0.0001,
            0.0001,
            4,
        );

        Self {
            market_data: order_books.into_iter(),
            strategy: SimpleStrategy::new(100.0, instrument_info),
            risk_manager: RiskManager,
            oms,
            simulated_exchange,
            recorder,
            strategy_decisions: Vec::new(),
            processed_events: Vec::new(),
            completed_markouts: Vec::new(),
        }
    }

    pub(crate) fn run(&mut self) {
        let mut last_strategy_cts = None;

        while let Some(order_book) = self.market_data.next() {
            self.simulated_exchange.match_orders(&order_book);
            self.process_exchange_events();

            self.completed_markouts.extend(
                self.recorder
                    .process_order_book(order_book.clone())
                    .into_iter()
                    .map(|markout| (order_book.cts, markout)),
            );

            let strategy_is_due = last_strategy_cts.is_none_or(|last_cts| {
                order_book.cts.saturating_sub(last_cts) >= STRATEGY_INTERVAL_MS
            });
            if !strategy_is_due {
                continue;
            }
            last_strategy_cts = Some(order_book.cts);

            let inventory = self.oms.inventory();
            let orders = self.strategy.execute(&order_book, inventory);
            self.strategy_decisions.push(StrategyDecision {
                cts: order_book.cts,
                inventory,
                orders: orders.clone(),
            });
            for order in orders {
                self.oms.forward_orders(order, &self.risk_manager);
            }

            self.process_exchange_events();
        }
    }

    fn process_exchange_events(&mut self) {
        for event in self.simulated_exchange.drain_events() {
            if let OrderEvent::ExecutionUpdate(execution) = &event {
                self.recorder.update_trades(execution.clone());
            }
            self.oms.order_response(event.clone());
            self.processed_events.push(event);
        }
    }
}
