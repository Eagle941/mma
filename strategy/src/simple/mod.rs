use exchange::bybit::{
    book::OrderBook,
    order::{Order, OrderManagementSystem, OrderSide, OrderType},
};
use std::env;
use std::str::FromStr;

pub struct SimpleStrategy {
    bybit: OrderManagementSystem,
    spread: f64,
    size: f64,
    symbol: String,
}
impl SimpleStrategy {
    pub fn new(spread: f64, size: f64, symbol: &str) -> SimpleStrategy {
        let base_url = "https://api-testnet.bybit.com";
        let api_key = "xxxxxxxx";
        let api_secret = "xxxxxxxxxxx";
        let bybit = OrderManagementSystem::new(
            base_url.to_owned(),
            api_key.to_owned(),
            api_secret.to_owned(),
        );
        let symbol = symbol.to_string();
        SimpleStrategy {
            bybit,
            spread,
            size,
            symbol,
        }
    }

    pub fn factory() -> SimpleStrategy {
        let symbol = env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");

        let spread = env::var("MMA_SPREAD").expect("MMA_SPREAD env variable must not be blank.");
        let spread = f64::from_str(&spread).expect("MMA_SPREAD is not a valid number.");

        let size =
            env::var("MMA_ORDER_SIZE").expect("MMA_ORDER_SIZE env variable must not be blank.");
        let size = f64::from_str(&size).expect("MMA_ORDER_SIZE is not a valid number.");

        SimpleStrategy::new(spread, size, symbol.as_str())
    }

    pub fn execute(&self, order_book: &OrderBook) {
        if order_book.bids.len() != 0 && order_book.asks.len() != 0 {
            let first_bid = order_book.bids.first().unwrap();
            let last_bid = order_book.bids.last().unwrap();
            let first_ask = order_book.asks.first().unwrap();
            let last_ask = order_book.asks.last().unwrap();

            println!(
                "B {:.2} {:.2} | A {:.2} {:.2} | S {:.2}",
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
            let bid_order = Order {
                symbol: self.symbol.clone(),
                side: OrderSide::BUY,
                order_type: OrderType::LIMIT,
                qty: self.size,
                price: bid_price,
            };
            self.bybit.submit_order(bid_order);

            let ask_order = Order {
                symbol: self.symbol.clone(),
                side: OrderSide::SELL,
                order_type: OrderType::LIMIT,
                qty: self.size,
                price: ask_price,
            };
            self.bybit.submit_order(ask_order);
        }
    }
}
