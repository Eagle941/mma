use std::env;
use std::str::FromStr;
use std::sync::mpsc::Sender;

use exchange::bybit::info::Info;
use exchange::{OrderBook, OrderBuilder, OrderSide, OrderType};

#[derive(Clone, Debug)]
pub struct SimpleStrategy {
    size: f64,
    instrument_info: Info,
    oms_channel: Sender<OrderBuilder>,
}
impl SimpleStrategy {
    pub fn new(oms_channel: Sender<OrderBuilder>, size: f64, symbol: &str) -> SimpleStrategy {
        let instrument_info = Info::new(symbol.to_string());
        println!("{instrument_info:#?}");
        SimpleStrategy {
            oms_channel,
            size,
            instrument_info,
        }
    }

    pub fn factory(oms_channel: Sender<OrderBuilder>) -> SimpleStrategy {
        let symbol = env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");

        // TODO: calculate minimum order size from the `Info` struct.
        let size =
            env::var("MMA_ORDER_SIZE").expect("MMA_ORDER_SIZE env variable must not be blank.");
        let size = f64::from_str(&size).expect("MMA_ORDER_SIZE is not a valid number.");

        SimpleStrategy::new(oms_channel, size, symbol.as_str())
    }

    pub fn execute(&self, order_book: &OrderBook) {
        if !order_book.bids.is_empty() && !order_book.asks.is_empty() {
            let first_bid = order_book.bids.first().unwrap();
            let last_bid = order_book.bids.last().unwrap();
            let first_ask = order_book.asks.first().unwrap();
            let last_ask = order_book.asks.last().unwrap();

            let decimal_digits = self.instrument_info.decimal_places;
            println!(
                "B {:.*} {:.*} | A {:.*} {:.*} | S {:.*}",
                decimal_digits,
                last_bid.price,
                decimal_digits,
                first_bid.price,
                decimal_digits,
                first_ask.price,
                decimal_digits,
                last_ask.price,
                decimal_digits,
                if first_bid.price != 0.0 && first_ask.price != 0.0 {
                    first_ask.price - first_bid.price
                } else {
                    0.0
                }
            );

            let precision = self.instrument_info.tick_size;

            if first_ask.price - first_bid.price > precision * 4.0 {
                let bid_price = first_bid.price - (precision * 2.0);
                let ask_price = first_ask.price + (precision * 2.0);

                let bid_price = (bid_price / precision).floor() * precision;
                let ask_price = (ask_price / precision).floor() * precision;

                // TODO: Optimise String cloning
                // TODO: Make parallel order submission
                // TODO: Deal with channel send errors
                let bid_order = OrderBuilder {
                    symbol: self.instrument_info.symbol.clone(),
                    side: OrderSide::Buy,
                    order_type: OrderType::Limit,
                    qty: self.size,
                    price: format!("{bid_price:.*}", decimal_digits),
                };
                self.oms_channel.send(bid_order).unwrap();

                let ask_order = OrderBuilder {
                    symbol: self.instrument_info.symbol.clone(),
                    side: OrderSide::Sell,
                    order_type: OrderType::Limit,
                    qty: self.size,
                    price: format!("{ask_price:.*}", decimal_digits),
                };
                self.oms_channel.send(ask_order).unwrap();
            }
        }
    }
}
