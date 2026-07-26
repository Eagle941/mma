use std::cell::{Ref, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use exchange::{
    OrderAmendedBuilder,
    OrderBook,
    OrderBuilder,
    OrderEvent,
    OrderExecution,
    OrderGateway,
    OrderSide,
    OrderStatus,
    OrderUpdate,
};

const MAKER_FEE_RATE: f64 = 0.001; // 0.1%

#[derive(Clone, Copy, Debug)]
struct CurrentBook {
    best_bid: f64,
    best_ask: f64,
    cts: u64,
}

impl CurrentBook {
    fn is_crossed_by(self, order: &OrderBuilder, price: f64) -> bool {
        match order.side {
            OrderSide::Buy => price >= self.best_ask,
            OrderSide::Sell => price <= self.best_bid,
        }
    }
}

fn order_price(order: &OrderBuilder) -> f64 {
    order
        .price
        .parse::<f64>()
        .expect("simulated order price must be valid")
}

fn new_order_acknowledgement(
    order_link_id: u64,
    order: &OrderBuilder,
    price: f64,
    updated_time: u64,
) -> OrderEvent {
    OrderEvent::OrderUpdate(OrderUpdate {
        order_link_id,
        order_status: OrderStatus::New,
        qty: order.qty,
        price,
        filled_qty: 0.0,
        filled_price: f64::NAN,
        updated_time,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TimedSubmission {
    pub(crate) cts: u64,
    pub(crate) order_link_id: u64,
    pub(crate) order: OrderBuilder,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TimedAmendment {
    pub(crate) cts: u64,
    pub(crate) amendment: OrderAmendedBuilder,
}

#[derive(Debug)]
pub(crate) struct SimulatedExchangeState {
    pub(crate) submitted_orders: Vec<TimedSubmission>,
    pub(crate) amendment_requests: Vec<TimedAmendment>,
    pub(crate) rejected_amendments: Vec<OrderAmendedBuilder>,
    pub(crate) open_orders: BTreeMap<u64, OrderBuilder>,
    pending_events: VecDeque<OrderEvent>,
    next_execution_id: u64,
    current_book: Option<CurrentBook>,
    pub(crate) repaid_coins: Vec<String>,
    pub(crate) cancel_all_calls: usize,
}

impl Default for SimulatedExchangeState {
    fn default() -> Self {
        Self {
            submitted_orders: Vec::new(),
            amendment_requests: Vec::new(),
            rejected_amendments: Vec::new(),
            open_orders: BTreeMap::new(),
            pending_events: VecDeque::new(),
            next_execution_id: 1,
            current_book: None,
            repaid_coins: Vec::new(),
            cancel_all_calls: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SimulatedExchange {
    state: Rc<RefCell<SimulatedExchangeState>>,
}

impl SimulatedExchange {
    pub(crate) fn state(&self) -> Ref<'_, SimulatedExchangeState> {
        self.state.borrow()
    }

    pub(crate) fn match_orders(&self, order_book: &OrderBook) {
        let best_bid = order_book.bids.first().expect("book must have a bid").price;
        let best_ask = order_book
            .asks
            .first()
            .expect("book must have an ask")
            .price;
        let mut state = self.state.borrow_mut();
        let current_book = CurrentBook {
            best_bid,
            best_ask,
            cts: order_book.cts,
        };
        state.current_book = Some(current_book);

        let matching_order_ids: Vec<_> = state
            .open_orders
            .iter()
            .filter_map(|(order_link_id, order)| {
                current_book
                    .is_crossed_by(order, order_price(order))
                    .then_some(*order_link_id)
            })
            .collect();

        for order_link_id in matching_order_ids {
            let order = state
                .open_orders
                .remove(&order_link_id)
                .expect("matching order must remain open until filled");
            let exec_price = order_price(&order);
            let exec_id = format!("sim-execution-{}", state.next_execution_id);
            state.next_execution_id += 1;
            let exec_fee = match order.side {
                OrderSide::Buy => order.qty * MAKER_FEE_RATE,
                OrderSide::Sell => order.qty * exec_price * MAKER_FEE_RATE,
            };

            state
                .pending_events
                .push_back(OrderEvent::ExecutionUpdate(OrderExecution {
                    order_link_id,
                    order_id: format!("sim-order-{order_link_id}"),
                    order_price: exec_price,
                    order_side: order.side,
                    exec_id,
                    exec_ts: order_book.cts,
                    exec_price,
                    exec_fee,
                    exec_qty: order.qty,
                    remaining_qty: 0.0,
                }));
            state
                .pending_events
                .push_back(OrderEvent::OrderUpdate(OrderUpdate {
                    order_link_id,
                    order_status: OrderStatus::Filled,
                    qty: order.qty,
                    price: exec_price,
                    filled_qty: order.qty,
                    filled_price: exec_price,
                    updated_time: order_book.cts,
                }));
        }
    }

    pub(crate) fn drain_events(&self) -> Vec<OrderEvent> {
        self.state.borrow_mut().pending_events.drain(..).collect()
    }
}

impl OrderGateway for SimulatedExchange {
    fn submit_order(&self, order: &OrderBuilder, order_link_id: u64) {
        let mut state = self.state.borrow_mut();
        let current_book = state
            .current_book
            .expect("market data must be available before submitting an order");
        state.submitted_orders.push(TimedSubmission {
            cts: current_book.cts,
            order_link_id,
            order: order.clone(),
        });
        let price = order_price(order);

        if current_book.is_crossed_by(order, price) {
            state
                .pending_events
                .push_back(OrderEvent::SubmissionFailed(order_link_id));
            return;
        }

        state.open_orders.insert(order_link_id, order.clone());
        state.pending_events.push_back(new_order_acknowledgement(
            order_link_id,
            order,
            price,
            current_book.cts,
        ));
    }

    fn amend_order(&self, amendment: &OrderAmendedBuilder) {
        let mut state = self.state.borrow_mut();
        let current_book = state
            .current_book
            .expect("market data must be available before amending an order");
        state.amendment_requests.push(TimedAmendment {
            cts: current_book.cts,
            amendment: amendment.clone(),
        });
        let Some(existing_order) = state.open_orders.get(&amendment.order_link_id) else {
            return;
        };
        let mut amended_order = existing_order.clone();
        if amendment.new_price {
            amended_order.price.clone_from(&amendment.price);
        }
        if amendment.new_qty {
            amended_order.qty = amendment.qty;
        }

        let price = order_price(&amended_order);
        if current_book.is_crossed_by(&amended_order, price) {
            state.rejected_amendments.push(amendment.clone());
            return;
        }

        state
            .open_orders
            .insert(amendment.order_link_id, amended_order.clone());
        state.pending_events.push_back(new_order_acknowledgement(
            amendment.order_link_id,
            &amended_order,
            price,
            current_book.cts,
        ));
    }

    fn repay_liability(&self, coin: &str) {
        self.state.borrow_mut().repaid_coins.push(coin.to_string());
    }

    fn cancel_all(&self) {
        self.state.borrow_mut().cancel_all_calls += 1;
    }
}
