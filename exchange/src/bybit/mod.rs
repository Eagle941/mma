use std::fmt;
use std::str::FromStr;

use bybit::ws::response::{OrderbookItem, SpotPublicResponse};
use bybit::ws::spot::OrderbookDepth;
use bybit::WebSocketApiClient;
use triple_buffer::Input;

#[derive(Debug, Default)]
struct OwnedOrderBookItem(String, String);
impl<'a> From<&OrderbookItem<'a>> for OwnedOrderBookItem {
    fn from(value: &OrderbookItem) -> Self {
        OwnedOrderBookItem(value.0.to_owned(), value.1.to_owned())
    }
}
impl fmt::Display for OwnedOrderBookItem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({},{})", self.0, self.1)
    }
}

// TODO: set from the configuration package.
pub const ORDER_BOOK_LEVELS: usize = 50;

#[derive(Copy, Clone, Debug, Default)]
pub struct Level {
    // TODO: verify if f64 is suitable for correctness and efficiency.
    pub price: f64,
    pub size: f64,
}
impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({},{})", self.price, self.size)
    }
}
impl From<OwnedOrderBookItem> for Level {
    fn from(src: OwnedOrderBookItem) -> Self {
        // TODO: optimise parsing method from `String` to `f64`
        Level {
            price: unsafe { f64::from_str(&src.0).unwrap_unchecked() },
            size: unsafe { f64::from_str(&src.1).unwrap_unchecked() },
        }
    }
}
impl<'a> From<&OrderbookItem<'a>> for Level {
    fn from(src: &OrderbookItem) -> Self {
        let owned: OwnedOrderBookItem = src.into();
        owned.into()
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
    pub ts: f64,
    // The timestamp from the matching engine when this orderbook data is
    // produced. It can be correlated with T from public trade channel.
    pub cts: f64,
}
impl OrderBook {
    // TODO: extract callback in separate function for testing.
    pub fn subscribe(order_book_publisher: &mut Input<OrderBook>, symbol: &str) {
        let mut client = WebSocketApiClient::spot().build();

        client.subscribe_orderbook(symbol, OrderbookDepth::Level50);

        let callback = |res: SpotPublicResponse| {
            match res {
                SpotPublicResponse::Orderbook(res) => {
                    let order_book = order_book_publisher.input_buffer_mut();
                    // Once you have subscribed successfully, you will receive a snapshot.
                    // If you receive a new snapshot message, you will have to reset your local orderbook.
                    if res.type_ == "snapshot" || res.data.u == 1 {
                        order_book.asks = res.data.a.iter().map(|item| item.into()).collect();
                        order_book.bids = res.data.b.iter().map(|item| item.into()).collect();
                        return;
                    }

                    // Receive a delta message, update the orderbook.
                    // Note that asks and bids of a delta message **do not guarantee** to be ordered.

                    // process asks
                    let a = &res.data.a;
                    let mut i: usize = 0;
                    while i < a.len() {
                        let level: Level = (&a[i]).into();

                        let mut j: usize = 0;
                        while j < order_book.asks.len() {
                            let item = &mut order_book.asks[j];
                            let item_price: f64 = item.price;

                            if level.price < item_price {
                                order_book.asks.insert(j, level);
                                break;
                            }

                            if level.price == item_price {
                                if level.size != 0.0 {
                                    item.size = level.size;
                                } else {
                                    order_book.asks.remove(j);
                                }
                                break;
                            }

                            j += 1;
                        }

                        if j == order_book.asks.len() {
                            order_book.asks.push(level)
                        }

                        i += 1;
                    }

                    // process bids
                    let b = &res.data.b;
                    let mut i: usize = 0;
                    while i < b.len() {
                        let level: Level = (&b[i]).into();

                        let mut j: usize = 0;
                        while j < order_book.bids.len() {
                            let item = &mut order_book.bids[j];
                            let item_price: f64 = item.price;
                            if level.price > item_price {
                                order_book.bids.insert(j, level);
                                break;
                            }

                            if level.price == item_price {
                                if level.size != 0.0 {
                                    item.size = level.size;
                                } else {
                                    order_book.bids.remove(j);
                                }
                                break;
                            }

                            j += 1;
                        }

                        if j == order_book.bids.len() {
                            order_book.bids.push(level);
                        }

                        i += 1;
                    }
                }
                SpotPublicResponse::Op(res) => {
                    println!("{res:?}")
                }
                x => unreachable!("SpotPublicResponse::{x:?} not implemented"),
            }
            order_book_publisher.publish();
        };

        match client.run(callback) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", e),
        }
    }
}
