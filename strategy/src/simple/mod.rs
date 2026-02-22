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

    fn calculate_profits(&self, bid_price: f64, bid_qty: f64, ask_price: f64, ask_qty: f64) -> f64 {
        const MAKER_FEE: f64 = 0.0676; // %
        const BORROW_FEE_HOURLY: f64 = 0.00060432; // %

        let gross_profit = ask_qty * ask_price - bid_qty * bid_price;
        let buy_fees = (bid_qty * bid_price) * (MAKER_FEE / 100.0);
        let sell_fees = (ask_qty * ask_price) * (MAKER_FEE / 100.0);
        // NOTE: assuming I need to borrow ADA to make a sell order and I pay borrowing
        // fees for 5 hours.
        let borrow_fees = (ask_qty * ask_price) * (BORROW_FEE_HOURLY * 5.0 / 100.0);
        gross_profit - buy_fees - sell_fees - borrow_fees
    }

    pub fn execute(&self, order_book: &OrderBook) {
        if !order_book.bids.is_empty() && !order_book.asks.is_empty() {
            let first_bid = order_book.bids.first().unwrap();
            // let last_bid = order_book.bids.last().unwrap();
            let first_ask = order_book.asks.first().unwrap();
            // let last_ask = order_book.asks.last().unwrap();

            let decimal_digits = self.instrument_info.decimal_places;
            // println!(
            //     "B {:.*} {:.*} | A {:.*} {:.*} | S {:.*}",
            //     decimal_digits,
            //     last_bid.price,
            //     decimal_digits,
            //     first_bid.price,
            //     decimal_digits,
            //     first_ask.price,
            //     decimal_digits,
            //     last_ask.price,
            //     decimal_digits,
            //     if first_bid.price != 0.0 && first_ask.price != 0.0 {
            //         first_ask.price - first_bid.price
            //     } else {
            //         0.0
            //     }
            // );

            let precision = self.instrument_info.tick_size;

            let mid_price = (first_bid.price + first_ask.price) / 2.0;
            let bid_price = mid_price - precision * 2.0;
            let ask_price = mid_price + precision * 2.0;

            // let profits = self.calculate_profits(bid_price, self.size, ask_price,
            // self.size); println!("Expected {profits:.*} USDT",
            // decimal_digits);

            // TODO: Optimise String cloning
            // TODO: Make batch order submission
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
