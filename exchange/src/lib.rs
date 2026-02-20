use std::f64;
use std::fmt::{Display, Formatter, Result};
use std::str::FromStr;

use ::bybit::ws::response::{Order as BybitOrder, OrderbookItem};
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
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, EnumString, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
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
}
impl OrderStatus {
    pub fn is_open(&self) -> bool {
        match self {
            OrderStatus::New | OrderStatus::PartiallyFilled | OrderStatus::Untriggered => true,
            _ => false,
        }
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
    pub fn build(self) -> Order {
        Order {
            order_id: String::new(),
            order_status: OrderStatus::New,
            symbol: self.symbol,
            side: self.side,
            order_type: self.order_type,
            qty: self.qty,
            price: f64::from_str(self.price.as_str()).unwrap(),
            filled_qty: 0.0,
            filled_price: f64::NAN,
            from_bot: true,
        }
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
    pub filled_price: f64,
    pub from_bot: bool,
}
impl<'a> From<&BybitOrder<'a>> for Order {
    fn from(src: &BybitOrder) -> Self {
        // TODO: this `try_into` is very dangerous. It needs to be improved.
        Order {
            order_id: src.order_id.to_string(),
            order_status: src.order_status.try_into().unwrap(),
            symbol: src.symbol.to_string(),
            side: src.side.try_into().unwrap(),
            order_type: src.order_type.try_into().unwrap(),
            qty: f64::from_str(src.qty).unwrap(),
            price: f64::from_str(src.price).unwrap(),
            filled_qty: f64::from_str(src.cum_exec_qty).unwrap(),
            filled_price: f64::from_str(src.avg_price).unwrap_or(f64::NAN),
            from_bot: false,
        }
    }
}
