use std::sync::Arc;

use bybit::WebSocketApiClient;
use bybit::ws::response::{Orderbook, SpotPublicResponse};
use bybit::ws::spot::{OrderbookDepth, SpotWebsocketApiClient};
use crossbeam_queue::ArrayQueue;
use log::warn;
use triple_buffer::Input;

use crate::bybit::utils::is_testnet;
use crate::{Level, OrderBook};

// TODO: set from the configuration package.
pub const ORDER_BOOK_LEVELS: usize = 50;

#[derive(Debug)]
pub struct PublicWebSocket {
    to_recorder: Arc<ArrayQueue<OrderBook>>,
    order_book: OrderBook,
}
impl PublicWebSocket {
    pub fn new(to_recorder: Arc<ArrayQueue<OrderBook>>) -> Self {
        PublicWebSocket {
            to_recorder,
            order_book: OrderBook::default(),
        }
    }

    fn get_ws_client(&self) -> SpotWebsocketApiClient {
        if is_testnet() {
            return WebSocketApiClient::spot().testnet().build();
        }
        WebSocketApiClient::spot().build()
    }

    // TODO: Optimise order book updates
    fn process_delta(&mut self, data: Orderbook) {
        // process asks
        for ask in &data.a {
            let ask: Level = ask.into();
            match self
                .order_book
                .asks
                .iter_mut()
                .find(|x| x.price == ask.price)
            {
                Some(item) => item.size = ask.size,
                None => self.order_book.asks.push(ask),
            }
        }

        // process bids
        for bid in &data.b {
            let bid: Level = bid.into();
            match self
                .order_book
                .bids
                .iter_mut()
                .find(|x| x.price == bid.price)
            {
                Some(item) => item.size = bid.size,
                None => self.order_book.bids.push(bid),
            }
        }

        self.order_book.bids.retain(|b| b.size != 0.0);
        self.order_book.asks.retain(|a| a.size != 0.0);

        self.order_book
            .asks
            .sort_by(|a, b| a.price.total_cmp(&b.price));
        self.order_book
            .bids
            .sort_by(|a, b| b.price.total_cmp(&a.price));
    }

    // TODO: extract callback in separate function for testing.
    pub fn subscribe(&mut self, order_book_publisher: &mut Input<OrderBook>, symbol: &str) {
        let mut client = self.get_ws_client();
        client.subscribe_orderbook(symbol, OrderbookDepth::Level50);

        let callback = |res: SpotPublicResponse| {
            match res {
                SpotPublicResponse::Orderbook(res) => {
                    // TODO: should it be res.cts? It's not available at the moment.
                    self.order_book.cts = res.ts;
                    self.order_book.ts = res.ts;
                    // If you receive a new snapshot message, you will have to reset your local
                    // orderbook.
                    if res.type_ == "snapshot" || res.data.u == 1 {
                        self.order_book.asks = res.data.a.iter().map(|item| item.into()).collect();
                        self.order_book.bids = res.data.b.iter().map(|item| item.into()).collect();
                        return;
                    }

                    // Receive a delta message, update the orderbook.
                    // Note that asks and bids of a delta message **do not guarantee** to be
                    // ordered.
                    self.process_delta(res.data);

                    // TODO: remove the cloning forced by the triple buffer consistency
                    let order_book = order_book_publisher.input_buffer_mut();
                    order_book.asks = self.order_book.asks.clone();
                    order_book.bids = self.order_book.bids.clone();
                    order_book_publisher.publish();

                    self.to_recorder.force_push(self.order_book.clone());
                }
                SpotPublicResponse::Op(res) => {
                    if !res.success {
                        warn!("{res:?}")
                    }
                }
                x => warn!("SpotPublicResponse::{x:?} not implemented"),
            }
        };

        match client.run(callback) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use bybit::ws::response::OrderbookItem;

    use super::*;

    fn create_order_book(asks: &[(f64, f64)], bids: &[(f64, f64)], ts: u64, cts: u64) -> OrderBook {
        OrderBook {
            asks: asks
                .iter()
                .map(|&(price, size)| Level { price, size })
                .collect(),
            bids: bids
                .iter()
                .map(|&(price, size)| Level { price, size })
                .collect(),
            ts,
            cts,
        }
    }

    #[test]
    fn process_delta_updates_existing_bid_and_ask_sizes() {
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        let mut public_websocket = PublicWebSocket::new(recorder_order_books);
        public_websocket.order_book = create_order_book(
            &[(101.0, 2.0), (102.0, 3.0)],
            &[(99.0, 4.0), (98.0, 5.0)],
            1000,
            1001,
        );
        let delta = Orderbook {
            s: "ADAUSDT",
            b: vec![OrderbookItem("99", "8")],
            a: vec![OrderbookItem("101", "7")],
            u: 2,
            seq: Some(2),
        };

        public_websocket.process_delta(delta);

        assert_eq!(
            public_websocket.order_book,
            create_order_book(
                &[(101.0, 7.0), (102.0, 3.0)],
                &[(99.0, 8.0), (98.0, 5.0)],
                1000,
                1001,
            )
        );
    }

    #[test]
    fn process_delta_inserts_and_sorts_new_levels() {
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        let mut public_websocket = PublicWebSocket::new(recorder_order_books);
        public_websocket.order_book = create_order_book(&[(102.0, 2.0)], &[(98.0, 2.0)], 0, 0);
        let delta = Orderbook {
            s: "ADAUSDT",
            b: vec![OrderbookItem("97", "3"), OrderbookItem("99", "1")],
            a: vec![OrderbookItem("103", "3"), OrderbookItem("101", "1")],
            u: 2,
            seq: Some(2),
        };

        public_websocket.process_delta(delta);

        assert_eq!(
            public_websocket.order_book,
            create_order_book(
                &[(101.0, 1.0), (102.0, 2.0), (103.0, 3.0)],
                &[(99.0, 1.0), (98.0, 2.0), (97.0, 3.0)],
                0,
                0,
            )
        );
    }

    #[test]
    fn process_delta_removes_zero_sized_levels() {
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        let mut public_websocket = PublicWebSocket::new(recorder_order_books);
        public_websocket.order_book = create_order_book(
            &[(101.0, 1.0), (102.0, 2.0)],
            &[(99.0, 1.0), (98.0, 2.0)],
            0,
            0,
        );
        let delta = Orderbook {
            s: "ADAUSDT",
            b: vec![OrderbookItem("99", "0"), OrderbookItem("97", "0")],
            a: vec![OrderbookItem("101", "0"), OrderbookItem("103", "0")],
            u: 2,
            seq: Some(2),
        };

        public_websocket.process_delta(delta);

        assert_eq!(
            public_websocket.order_book,
            create_order_book(&[(102.0, 2.0)], &[(98.0, 2.0)], 0, 0)
        );
    }

    #[test]
    fn process_delta_applies_mixed_unordered_changes() {
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        let mut public_websocket = PublicWebSocket::new(recorder_order_books);
        public_websocket.order_book = create_order_book(
            &[(101.0, 1.0), (102.0, 2.0), (104.0, 4.0)],
            &[(99.0, 1.0), (98.0, 2.0), (96.0, 4.0)],
            1000,
            1001,
        );
        let delta = Orderbook {
            s: "ADAUSDT",
            b: vec![
                OrderbookItem("97", "7"),
                OrderbookItem("96", "0"),
                OrderbookItem("99", "10"),
            ],
            a: vec![
                OrderbookItem("103", "3"),
                OrderbookItem("104", "0"),
                OrderbookItem("102", "20"),
            ],
            u: 2,
            seq: Some(2),
        };

        public_websocket.process_delta(delta);

        assert_eq!(
            public_websocket.order_book,
            create_order_book(
                &[(101.0, 1.0), (102.0, 20.0), (103.0, 3.0)],
                &[(99.0, 10.0), (98.0, 2.0), (97.0, 7.0)],
                1000,
                1001,
            )
        );
    }

    #[test]
    fn process_delta_with_no_levels_preserves_order_book() {
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        let mut public_websocket = PublicWebSocket::new(recorder_order_books);
        let initial_order_book = create_order_book(&[(101.0, 1.0)], &[(99.0, 1.0)], 1000, 1001);
        public_websocket.order_book = initial_order_book.clone();
        let delta = Orderbook {
            s: "ADAUSDT",
            b: Vec::new(),
            a: Vec::new(),
            u: 2,
            seq: Some(2),
        };

        public_websocket.process_delta(delta);

        assert_eq!(public_websocket.order_book, initial_order_book);
    }
}
