use std::env;
use std::str::FromStr;
use std::sync::mpsc::Sender;

use exchange::{OrderBook, OrderBuilder, OrderSide, OrderType};

#[derive(Clone, Debug)]
pub struct SimpleStrategy {
    spread: f64,
    size: f64,
    symbol: String,
    oms_channel: Sender<OrderBuilder>,
}
impl SimpleStrategy {
    pub fn new(
        oms_channel: Sender<OrderBuilder>,
        spread: f64,
        size: f64,
        symbol: &str,
    ) -> SimpleStrategy {
        let symbol = symbol.to_string();
        SimpleStrategy {
            oms_channel,
            spread,
            size,
            symbol,
        }
    }

    pub fn factory(oms_channel: Sender<OrderBuilder>) -> SimpleStrategy {
        let symbol = env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");

        let spread = env::var("MMA_SPREAD").expect("MMA_SPREAD env variable must not be blank.");
        let spread = f64::from_str(&spread).expect("MMA_SPREAD is not a valid number.");

        let size =
            env::var("MMA_ORDER_SIZE").expect("MMA_ORDER_SIZE env variable must not be blank.");
        let size = f64::from_str(&size).expect("MMA_ORDER_SIZE is not a valid number.");

        SimpleStrategy::new(oms_channel, spread, size, symbol.as_str())
    }

    pub fn execute(&self, order_book: &OrderBook) {
        if !order_book.bids.is_empty() && !order_book.asks.is_empty() {
            let first_bid = order_book.bids.first().unwrap();
            let last_bid = order_book.bids.last().unwrap();
            let first_ask = order_book.asks.first().unwrap();
            let last_ask = order_book.asks.last().unwrap();

            println!(
                "B {:.3} {:.3} | A {:.3} {:.3} | S {:.3}",
                last_bid.price,
                first_bid.price,
                first_ask.price,
                last_ask.price,
                if first_bid.price != 0.0 && first_ask.price != 0.0 {
                    first_ask.price - first_bid.price
                } else {
                    0.0
                }
            );

            let mid_price = (first_ask.price + first_bid.price) / 2.0;
            let half_spread = mid_price * self.spread / 2.0;
            let bid_price = mid_price - half_spread;
            let ask_price = mid_price + half_spread;

            // TODO: Optimise String cloning
            // TODO: Make parallel order submission
            // TODO: Deal with channel send errors
            let bid_order = OrderBuilder {
                symbol: self.symbol.clone(),
                side: OrderSide::Buy,
                order_type: OrderType::Limit,
                qty: self.size,
                price: bid_price,
            };
            self.oms_channel.send(bid_order).unwrap();

            let ask_order = OrderBuilder {
                symbol: self.symbol.clone(),
                side: OrderSide::Sell,
                order_type: OrderType::Limit,
                qty: self.size,
                price: ask_price,
            };
            self.oms_channel.send(ask_order).unwrap();
        }
    }
}
