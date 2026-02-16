use std::fmt::{self, Display, Result};
use std::str::FromStr;

use bybit::ws::response::{Orderbook, OrderbookItem, SpotPublicResponse};
use bybit::ws::spot::OrderbookDepth;
use bybit::WebSocketApiClient;
use triple_buffer::Input;

// TODO: set from the configuration package.
pub const ORDER_BOOK_LEVELS: usize = 50;

#[derive(Copy, Clone, Debug, Default)]
pub struct Level {
    // TODO: verify if f64 is suitable for correctness and efficiency.
    pub price: f64,
    pub size: f64,
}
impl Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result {
        write!(f, "({},{})", self.price, self.size)
    }
}
impl<'a> From<&OrderbookItem<'a>> for Level {
    fn from(src: &OrderbookItem) -> Self {
        // TODO: optimise parsing method from `String` to `f64`
        Level {
            price: unsafe { f64::from_str(&src.0).unwrap_unchecked() },
            size: unsafe { f64::from_str(&src.1).unwrap_unchecked() },
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
impl OrderBook {
    // TODO: Optimise order book updates
    fn process_delta(&mut self, data: Orderbook) {
        // process asks
        for ask in &data.a {
            let ask: Level = ask.into();
            match self.asks.iter_mut().find(|x| x.price == ask.price) {
                Some(item) => item.size = ask.size,
                None => self.asks.push(ask),
            }
        }

        // process bids
        for bid in &data.b {
            let bid: Level = bid.into();
            match self.bids.iter_mut().find(|x| x.price == bid.price) {
                Some(item) => item.size = bid.size,
                None => self.bids.push(bid),
            }
        }

        self.bids.retain(|b| b.size != 0.0);
        self.asks.retain(|a| a.size != 0.0);

        self.asks.sort_by(|a, b| a.price.total_cmp(&b.price));
        self.bids.sort_by(|a, b| b.price.total_cmp(&a.price));
    }

    // TODO: extract callback in separate function for testing.
    pub fn subscribe(&mut self, order_book_publisher: &mut Input<OrderBook>, symbol: &str) {
        let mut client = WebSocketApiClient::spot().build();

        client.subscribe_orderbook(symbol, OrderbookDepth::Level50);

        let callback = |res: SpotPublicResponse| {
            match res {
                SpotPublicResponse::Orderbook(res) => {
                    // Once you have subscribed successfully, you will receive a snapshot.
                    // If you receive a new snapshot message, you will have to reset your local orderbook.
                    if res.type_ == "snapshot" || res.data.u == 1 {
                        self.asks = res.data.a.iter().map(|item| item.into()).collect();
                        self.bids = res.data.b.iter().map(|item| item.into()).collect();
                        return;
                    }

                    // Receive a delta message, update the orderbook.
                    // Note that asks and bids of a delta message **do not guarantee** to be ordered.
                    Self::process_delta(self, res.data);

                    // TODO: remove the cloning forced by the triple buffer consistency
                    let order_book = order_book_publisher.input_buffer_mut();
                    order_book.asks = self.asks.clone();
                    order_book.bids = self.bids.clone();
                    order_book_publisher.publish();
                }
                SpotPublicResponse::Op(res) => {
                    if !res.success {
                        println!("{res:?}")
                    }
                }
                x => unreachable!("SpotPublicResponse::{x:?} not implemented"),
            }
        };

        match client.run(callback) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", e),
        }
    }
}
