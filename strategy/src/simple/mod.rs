use exchange::bybit::book::OrderBook;

#[derive(Default)]
pub struct SimpleStrategy {
    spread: f64,
    size: f64,
}
impl SimpleStrategy {
    pub fn new(spread: f64, size: f64) -> SimpleStrategy {
        SimpleStrategy { spread, size }
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
            let _bid_price = mid_price - half_spread;
            let _ask_price = mid_price + half_spread;

            // let bid_order = OrderCommand::PlaceBid {
            //     price: bid_price,
            //     size: config.order_size,
            // };
            // let ask_order = OrderCommand::PlaceAsk {
            //     price: ask_price,
            //     size: config.order_size,
            // };
        }
    }
}
