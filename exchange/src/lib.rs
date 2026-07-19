use std::f64;
use std::fmt::{Display, Formatter, Result};
use std::str::FromStr;

use ::bybit::ws::response::{Execution, Order as BybitOrder, OrderbookItem};
use serde::{Deserialize, Serialize};
use strum::{Display as EnumDisplay, EnumString};

pub mod bybit;

// TODO: make `OrderBook` struct shared across all exchanges.
#[derive(Copy, Clone, Debug, Default)]
pub struct Level {
    // TODO: verify if f64 is suitable for correctness and efficiency.
    pub price: f64,
    pub size: f64,
}
impl Display for Level {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "({},{})", self.price, self.size)
    }
}
impl<'a> From<&OrderbookItem<'a>> for Level {
    fn from(src: &OrderbookItem) -> Self {
        // TODO: optimise parsing method from `String` to `f64`
        Level {
            price: f64::from_str(src.0).unwrap(),
            size: f64::from_str(src.1).unwrap(),
        }
    }
}

// TODO: investigate if it's possible to replace `Vec` with slice for bids and
// asks levels.
#[derive(Clone, Debug, Default)]
pub struct OrderBook {
    // Sorted by price in descending order.
    pub bids: Vec<Level>,
    // Sorted by price in ascending order.
    pub asks: Vec<Level>,
    // The timestamp (ms) that the system generates the data.
    // UNUSED
    pub ts: u64,
    // The timestamp from the matching engine when this orderbook data is
    // produced. It can be correlated with T from public trade channel.
    // UNUSED
    pub cts: u64,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, EnumDisplay, EnumString, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, EnumDisplay, EnumString, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, EnumDisplay, EnumString, PartialEq)]
pub enum OrderStatus {
    // The status Submitted is used for orders which have been sent to the exchange, but no
    // response has been received yet. They may be lost or rejected.
    Submitted,
    // Open Status
    New,
    PartiallyFilled,
    Untriggered,
    // Closed Status
    Rejected,
    PartiallyFilledCanceled,
    Filled,
    Cancelled,
    Triggered,
    Deactivated,
}
impl OrderStatus {
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            OrderStatus::Submitted
                | OrderStatus::New
                | OrderStatus::PartiallyFilled
                | OrderStatus::Untriggered
        )
    }

    pub fn is_closed(&self) -> bool {
        !self.is_open()
    }
}

pub enum OrderEvent {
    OrderUpdate(OrderUpdate),
    ExecutionUpdate(OrderExecution),
    SubmissionFailed { order_link_id: u64 },
}
impl<'a> From<&BybitOrder<'a>> for OrderEvent {
    fn from(src: &BybitOrder) -> Self {
        // TODO: this `try_into` is very dangerous. It needs to be improved.
        let order = OrderUpdate {
            order_link_id: u64::from_str(src.order_link_id).unwrap(),
            order_status: src.order_status.try_into().unwrap(),
            qty: f64::from_str(src.qty).unwrap(),
            price: f64::from_str(src.price).unwrap(),
            filled_qty: f64::from_str(src.cum_exec_qty).unwrap(),
            filled_price: f64::from_str(src.avg_price).unwrap_or(f64::NAN),
            updated_time: u64::from_str(src.updated_time).unwrap(),
        };
        OrderEvent::OrderUpdate(order)
    }
}
impl<'a> From<&Execution<'a>> for OrderEvent {
    fn from(src: &Execution) -> Self {
        let order = OrderExecution {
            order_id: src.order_id.to_string(),
            order_side: src.side.try_into().unwrap(),
            order_price: f64::from_str(src.order_price).unwrap(),
            order_link_id: u64::from_str(src.order_link_id).unwrap(),
            exec_id: src.exec_id.to_string(),
            exec_ts: u64::from_str(src.exec_time).unwrap(),
            exec_qty: f64::from_str(src.exec_qty).unwrap(),
            exec_price: f64::from_str(src.exec_price).unwrap(),
            exec_fee: f64::from_str(src.exec_fee).unwrap_or(0.0),
            remaining_qty: f64::from_str(src.leaves_qty).unwrap(),
        };
        OrderEvent::ExecutionUpdate(order)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct OrderBuilder {
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub qty: f64,
    pub price: String,
}
impl OrderBuilder {
    // TODO: should it be converted into an Into trait of `OrderEvent`?
    pub fn build(&self, order_link_id: u64) -> Order {
        Order {
            order_link_id,
            order_status: OrderStatus::Submitted,
            symbol: self.symbol.clone(),
            side: self.side,
            order_type: self.order_type,
            qty: self.qty,
            price: f64::from_str(self.price.as_str()).unwrap(),
            filled_qty: 0.0,
            filled_price: f64::NAN,
            updated_time: 0,
        }
    }
}

/// Dispatches order commands to an exchange.
///
/// These methods confirm that a command was dispatched, not that the exchange
/// accepted or executed it. Exchange responses arrive separately through the
/// private order stream.
pub trait OrderGateway: std::fmt::Debug {
    fn submit_order(&self, order: &OrderBuilder, order_link_id: u64);
    fn amend_order(&self, order: &OrderAmendedBuilder);
    fn repay_liability(&self, coin: &str);
    fn cancel_all(&self);
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct OrderAmendedBuilder {
    pub symbol: String,
    pub order_link_id: u64,
    pub qty: f64,
    pub price: String,
    pub new_price: bool,
    pub new_qty: bool,
}

// TODO: Add order timestamps
// TODO: Is it better to keep price as String instead of f64?
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Order {
    pub order_link_id: u64,
    pub order_status: OrderStatus,
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub qty: f64,
    pub price: f64,
    pub filled_qty: f64,
    // NOTE: this is the average price of the order execution
    pub filled_price: f64,
    pub updated_time: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct OrderExecution {
    pub order_link_id: u64,
    pub order_id: String,
    pub order_price: f64,
    pub order_side: OrderSide,
    //
    pub exec_id: String,
    pub exec_ts: u64, // ms
    pub exec_price: f64,
    pub exec_fee: f64,
    pub exec_qty: f64,
    pub remaining_qty: f64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct OrderAmend {
    pub order_link_id: u64,
    pub qty: f64,
    pub price: f64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct OrderUpdate {
    pub order_link_id: u64,
    pub order_status: OrderStatus,
    pub qty: f64,
    pub price: f64,
    pub filled_qty: f64,
    // NOTE: this is the average price of the order execution
    pub filled_price: f64,
    pub updated_time: u64,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn level_from_orderbook_item_parses_price_and_size() {
        let orderbook_item = OrderbookItem("0.567", "125.5");

        let level = Level::from(&orderbook_item);

        assert_eq!(level.price, 0.567);
        assert_eq!(level.size, 125.5);
    }

    #[rstest]
    #[case("0.566", 0.566)]
    #[case("", f64::NAN)]
    fn order_message_from_bybit_order_maps_order_update(
        #[case] avg_price: &str,
        #[case] expected_filled_price: f64,
    ) {
        let bybit_order = BybitOrder {
            category: "spot",
            order_id: "exchange-order-id",
            order_link_id: "1234",
            is_leverage: "0",
            block_trade_id: "",
            symbol: "ADAUSDT",
            price: "0.567",
            qty: "25.0",
            side: "Buy",
            position_idx: 0,
            order_status: "New",
            cancel_type: "",
            reject_reason: "",
            avg_price,
            leaves_qty: "15.0",
            leaves_value: "",
            cum_exec_qty: "10.0",
            cum_exec_value: "",
            cum_exec_fee: "",
            time_in_force: "PostOnly",
            order_type: "Limit",
            stop_order_type: "",
            order_iv: "",
            trigger_price: "",
            take_profit: "",
            stop_loss: "",
            tp_trigger_by: "",
            sl_trigger_by: "",
            trigger_direction: 0,
            trigger_by: "",
            last_price_on_created: "",
            reduce_only: false,
            close_on_trigger: false,
            created_time: "1773956505000",
            updated_time: "1773956505537",
        };

        let message = OrderEvent::from(&bybit_order);

        let OrderEvent::OrderUpdate(order) = message else {
            panic!("expected an order update");
        };
        assert_eq!(order.order_link_id, 1234);
        assert_eq!(order.order_status, OrderStatus::New);
        assert_eq!(order.qty, 25.0);
        assert_eq!(order.price, 0.567);
        assert_eq!(order.filled_qty, 10.0);
        if expected_filled_price.is_nan() {
            // NOTE: NaN values can't be compared, hence the if-statement
            assert!(order.filled_price.is_nan());
        } else {
            assert_eq!(order.filled_price, expected_filled_price);
        }
        assert_eq!(order.updated_time, 1773956505537);
    }

    #[rstest]
    #[case("0.01", 0.01)]
    #[case("", 0.0)]
    fn order_message_from_execution_maps_execution_update(
        #[case] exec_fee: &str,
        #[case] expected_exec_fee: f64,
    ) {
        let execution = Execution {
            category: "spot",
            symbol: "ADAUSDT",
            is_leverage: "0",
            order_id: "exchange-order-id",
            order_link_id: "1234",
            side: "Sell",
            order_price: "0.567",
            order_qty: "25.0",
            leaves_qty: "15.0",
            order_type: "Limit",
            stop_order_type: "",
            exec_fee,
            exec_id: "execution-id",
            exec_price: "0.566",
            exec_qty: "10.0",
            exec_type: "Trade",
            exec_value: "5.66",
            exec_time: "1773956505537",
            is_maker: true,
            fee_rate: "0.001",
            trade_iv: "",
            mark_iv: "",
            mark_price: "",
            index_price: "",
            underlying_price: "",
            block_trade_id: "",
        };

        let message = OrderEvent::from(&execution);

        let OrderEvent::ExecutionUpdate(order) = message else {
            panic!("expected an execution update");
        };
        assert_eq!(order.order_link_id, 1234);
        assert_eq!(order.order_id, "exchange-order-id");
        assert_eq!(order.order_price, 0.567);
        assert_eq!(order.order_side, OrderSide::Sell);
        assert_eq!(order.exec_id, "execution-id");
        assert_eq!(order.exec_ts, 1773956505537);
        assert_eq!(order.exec_price, 0.566);
        assert_eq!(order.exec_fee, expected_exec_fee);
        assert_eq!(order.exec_qty, 10.0);
        assert_eq!(order.remaining_qty, 15.0);
    }

    #[test]
    fn order_builder_builds_submitted_order() {
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };

        let order = order_builder.build(1234);

        assert_eq!(order.order_link_id, 1234);
        assert_eq!(order.order_status, OrderStatus::Submitted);
        assert_eq!(order.symbol, order_builder.symbol);
        assert_eq!(order.side, order_builder.side);
        assert_eq!(order.order_type, order_builder.order_type);
        assert_eq!(order.qty, order_builder.qty);
        assert_eq!(order.price, 0.567);
        assert_eq!(order.filled_qty, 0.0);
        assert!(order.filled_price.is_nan());
        assert_eq!(order.updated_time, 0);
    }

    #[rstest]
    #[case(OrderStatus::Submitted, true)]
    #[case(OrderStatus::New, true)]
    #[case(OrderStatus::PartiallyFilled, true)]
    #[case(OrderStatus::Untriggered, true)]
    #[case(OrderStatus::Rejected, false)]
    #[case(OrderStatus::PartiallyFilledCanceled, false)]
    #[case(OrderStatus::Filled, false)]
    #[case(OrderStatus::Cancelled, false)]
    #[case(OrderStatus::Triggered, false)]
    #[case(OrderStatus::Deactivated, false)]
    fn order_status_is_open_identifies_active_orders(
        #[case] status: OrderStatus,
        #[case] expected: bool,
    ) {
        assert_eq!(status.is_open(), expected);
    }

    #[rstest]
    #[case(OrderStatus::Submitted, false)]
    #[case(OrderStatus::New, false)]
    #[case(OrderStatus::PartiallyFilled, false)]
    #[case(OrderStatus::Untriggered, false)]
    #[case(OrderStatus::Rejected, true)]
    #[case(OrderStatus::PartiallyFilledCanceled, true)]
    #[case(OrderStatus::Filled, true)]
    #[case(OrderStatus::Cancelled, true)]
    #[case(OrderStatus::Triggered, true)]
    #[case(OrderStatus::Deactivated, true)]
    fn order_status_is_closed_identifies_non_open_orders(
        #[case] status: OrderStatus,
        #[case] expected: bool,
    ) {
        assert_eq!(status.is_closed(), expected);
    }
}
