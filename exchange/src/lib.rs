use std::fmt::{Display, Formatter, Result};
use std::str::FromStr;

use ::bybit::ws::response::OrderbookItem;
use serde::{Deserialize, Serialize};

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

#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub enum OrderSide {
    BUY,
    SELL,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub enum OrderType {
    MARKET,
    LIMIT,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Order {
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub qty: f64,
    pub price: f64,
}
