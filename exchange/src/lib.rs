use std::f64;
use std::fmt::{Display, Formatter, Result};
use std::str::FromStr;

use ::bybit::ws::response::{Execution, Order as BybitOrder, OrderbookItem};
use serde::{Deserialize, Serialize};
use strum::EnumString;

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
    pub ts: f64,
    // The timestamp from the matching engine when this orderbook data is
    // produced. It can be correlated with T from public trade channel.
    // UNUSED
    pub cts: f64,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, EnumString, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
    NotAvailable,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, EnumString, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
    NotAvailable,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, EnumString, PartialEq)]
pub enum OrderStatus {
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
    //
    NotAvailable,
}
impl OrderStatus {
    pub fn is_open(&self) -> bool {
        match self {
            OrderStatus::New | OrderStatus::PartiallyFilled | OrderStatus::Untriggered => true,
            _ => false,
        }
    }

    pub fn is_closed(&self) -> bool {
        !self.is_open()
    }
}

pub enum OrderMessages {
    NewOrder(Order),
    AmendedOrder(Order),             // TODO: change to its own struct
    OrderUpdate(Order),              // TODO: change to its own struct
    ExecutionUpdate(OrderExecution), // TODO: change to its own struct
}
impl<'a> From<&BybitOrder<'a>> for OrderMessages {
    fn from(src: &BybitOrder) -> Self {
        // TODO: this `try_into` is very dangerous. It needs to be improved.
        let order = Order {
            order_id: src.order_id.to_string(),
            order_status: src.order_status.try_into().unwrap(),
            symbol: src.symbol.to_string(),
            side: src.side.try_into().unwrap(),
            order_type: src.order_type.try_into().unwrap(),
            qty: f64::from_str(src.qty).unwrap(),
            price: f64::from_str(src.price).unwrap(),
            filled_qty: f64::from_str(src.cum_exec_qty).unwrap(),
            filled_price: f64::from_str(src.avg_price).unwrap_or(f64::NAN),
            updated_time: u64::from_str(src.updated_time).unwrap_or(0),
        };
        OrderMessages::OrderUpdate(order)
    }
}
impl<'a> From<&Execution<'a>> for OrderMessages {
    fn from(src: &Execution) -> Self {
        let order = OrderExecution {
            order_id: src.order_id.to_string(),
            qty: f64::from_str(src.exec_qty).unwrap(),
            price: f64::from_str(src.exec_price).unwrap(),
            remaining_qty: f64::from_str(src.leaves_qty).unwrap(),
        };
        OrderMessages::ExecutionUpdate(order)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct OrderBuilder {
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub qty: f64,
    pub price: String,
}
impl OrderBuilder {
    // TODO: should it be converted to an Into trait of `OrderMessages`?
    pub fn build(self, order_id: String) -> OrderMessages {
        let order = Order {
            order_id,
            order_status: OrderStatus::New,
            symbol: self.symbol,
            side: self.side,
            order_type: self.order_type,
            qty: self.qty,
            price: f64::from_str(self.price.as_str()).unwrap(),
            filled_qty: 0.0,
            filled_price: f64::NAN,
            updated_time: 0,
        };
        OrderMessages::NewOrder(order)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct OrderAmendedBuilder {
    pub symbol: String,
    pub order_id: String,
    pub qty: f64,
    pub price: String,
    pub new_price: bool,
    pub new_qty: bool,
}
impl OrderAmendedBuilder {
    // TODO: should it be converted to an Into trait of `OrderMessages`?
    pub fn build(self, order_id: String) -> OrderMessages {
        let order = Order {
            order_id,
            order_status: OrderStatus::NotAvailable,
            symbol: self.symbol,
            side: OrderSide::NotAvailable,
            order_type: OrderType::NotAvailable,
            qty: self.qty,
            price: f64::from_str(self.price.as_str()).unwrap(),
            filled_qty: 0.0,
            filled_price: f64::NAN,
            updated_time: 0,
        };
        OrderMessages::AmendedOrder(order)
    }
}

// TODO: Add order timestamps
// TODO: Is it better to keep price as String instead of f64?
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Order {
    pub order_id: String,
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
    pub order_id: String,
    // NOTE: price is the execution price
    pub price: f64,
    // NOTEL qty is the size of the execution
    pub qty: f64,
    pub remaining_qty: f64,
}
