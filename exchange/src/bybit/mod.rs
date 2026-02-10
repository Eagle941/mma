use std::fmt;

use bybit::ws::response::{OrderbookItem, SpotPublicResponse};
use bybit::ws::spot;
use bybit::WebSocketApiClient;

// TODO: set from the configuration package.
pub const ORDER_BOOK_LEVELS: usize = 3;

#[derive(Debug, Default)]
pub struct Level {
    // TODO: verify if f64 is suitable for correctness and efficiency.
    pub price: f64,
    pub size: f64,
}

#[derive(Debug, Default)]
pub struct OrderBook {
    // Sorted by price in descending order.
    pub bids: [Level; ORDER_BOOK_LEVELS],
    // Sorted by price in ascending order.
    pub asks: [Level; ORDER_BOOK_LEVELS],
    // The timestamp (ms) that the system generates the data.
    pub ts: f64,
    // The timestamp from the matching engine when this orderbook data is
    // produced. It can be correlated with T from public trade channel.
    pub cts: f64,
}

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

impl OrderBook {
    pub fn subscribe(symbol: &str) {
        let mut client = WebSocketApiClient::spot().build();

        client.subscribe_orderbook(symbol, spot::OrderbookDepth::Level50);

        let mut asks: Vec<OwnedOrderBookItem> = Vec::new();
        let mut bids: Vec<OwnedOrderBookItem> = Vec::new();

        let callback = |res: SpotPublicResponse| {
            match res {
                SpotPublicResponse::Orderbook(res) => {
                    // > Once you have subscribed successfully, you will receive a snapshot.
                    // > If you receive a new snapshot message, you will have to reset your local orderbook.
                    if res.type_ == "snapshot" {
                        asks = res.data.a.iter().map(|item| item.into()).collect();
                        bids = res.data.b.iter().map(|item| item.into()).collect();
                        return;
                    }

                    // Receive a delta message, update the orderbook.
                    // Note that asks and bids of a delta message **do not guarantee** to be ordered.

                    // process asks
                    let a = &res.data.a;
                    let mut i: usize = 0;

                    while i < a.len() {
                        let OrderbookItem(price, qty) = a[i];

                        let mut j: usize = 0;
                        while j < asks.len() {
                            let item = &mut asks[j];
                            let item_price: &str = &item.0;

                            if price < item_price {
                                asks.insert(
                                    j,
                                    OwnedOrderBookItem(price.to_owned(), qty.to_owned()),
                                );
                                break;
                            }

                            if price == item_price {
                                if qty != "0" {
                                    item.1 = qty.to_owned();
                                } else {
                                    asks.remove(j);
                                }
                                break;
                            }

                            j += 1;
                        }

                        if j == asks.len() {
                            asks.push(OwnedOrderBookItem(price.to_owned(), qty.to_owned()))
                        }

                        i += 1;
                    }

                    // process bids
                    let b = &res.data.b;
                    let mut i: usize = 0;

                    while i < b.len() {
                        let OrderbookItem(price, qty) = b[i];

                        let mut j: usize = 0;
                        while j < bids.len() {
                            let item = &mut bids[j];
                            let item_price: &str = &item.0;
                            if price > item_price {
                                bids.insert(
                                    j,
                                    OwnedOrderBookItem(price.to_owned(), qty.to_owned()),
                                );
                                break;
                            }

                            if price == item_price {
                                if qty != "0" {
                                    item.1 = qty.to_owned();
                                } else {
                                    bids.remove(j);
                                }
                                break;
                            }

                            j += 1;
                        }

                        if j == bids.len() {
                            bids.push(OwnedOrderBookItem(price.to_owned(), qty.to_owned()));
                        }

                        i += 1;
                    }
                }
                SpotPublicResponse::Op(res) => {
                    println!("{res:?}")
                }
                x => unreachable!("SpotPublicResponse::{x:?} not implemented"),
            }

            if !bids.is_empty() && !asks.is_empty() {
                println!("BID {} | ASK {}", bids[0], asks[0]);
            }
        };

        match client.run(callback) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", e),
        }
    }
}
