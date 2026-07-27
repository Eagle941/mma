use std::sync::Arc;

use bybit::WebSocketApiClient;
use bybit::ws::response::{BasePublicResponse, Orderbook, SpotPublicResponse};
use bybit::ws::spot::{OrderbookDepth, SpotWebsocketApiClient};
use configuration::AppConfigProvider;
use crossbeam_queue::ArrayQueue;
use log::warn;
use triple_buffer::Input;

use crate::{Level, OrderBook};

// TODO: set from the configuration package.
pub const ORDER_BOOK_LEVELS: usize = 50;

#[derive(Debug)]
pub struct PublicWebSocket {
    testnet: bool,
    to_recorder: Arc<ArrayQueue<OrderBook>>,
    order_book: OrderBook,
}
impl PublicWebSocket {
    pub fn new(to_recorder: Arc<ArrayQueue<OrderBook>>, config: &dyn AppConfigProvider) -> Self {
        PublicWebSocket {
            testnet: config.testnet(),
            to_recorder,
            order_book: OrderBook::default(),
        }
    }

    fn get_ws_client(&self) -> SpotWebsocketApiClient {
        if self.testnet {
            return WebSocketApiClient::spot().testnet().build();
        }
        WebSocketApiClient::spot().build()
    }

    // TODO: Optimise order book updates
    #[expect(
        clippy::float_cmp,
        reason = "Bybit deltas use exact parsed prices as level IDs and exact zero sizes as \
                  deletion markers"
    )]
    fn process_delta(&mut self, data: &Orderbook) {
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

    fn process_orderbook_response(
        &mut self,
        response: &BasePublicResponse<'_, Orderbook<'_>>,
        order_book_publisher: &mut Input<OrderBook>,
    ) {
        // TODO: should it be response.cts? It's not available at the moment.
        self.order_book.cts = response.ts;
        self.order_book.ts = response.ts;
        // If you receive a new snapshot message, you will have to reset your local
        // orderbook.
        if response.type_ == "snapshot" || response.data.u == 1 {
            self.order_book.asks = response.data.a.iter().map(Into::into).collect();
            self.order_book.bids = response.data.b.iter().map(Into::into).collect();
        } else {
            // Receive a delta message, update the orderbook.
            // Note that asks and bids of a delta message **do not guarantee** to be
            // ordered.
            self.process_delta(&response.data);
        }

        // TODO: remove the cloning forced by the triple buffer consistency
        *order_book_publisher.input_buffer_mut() = self.order_book.clone();
        order_book_publisher.publish();

        self.to_recorder.force_push(self.order_book.clone());
    }

    pub fn subscribe(&mut self, order_book_publisher: &mut Input<OrderBook>, symbol: &str) {
        let mut client = self.get_ws_client();
        client.subscribe_orderbook(symbol, OrderbookDepth::Level50);

        let callback = |res: SpotPublicResponse| match res {
            SpotPublicResponse::Orderbook(response) => {
                self.process_orderbook_response(&response, order_book_publisher);
            }
            SpotPublicResponse::Op(res) => {
                if !res.success {
                    warn!("{res:?}");
                }
            }
            x => warn!("SpotPublicResponse::{x:?} not implemented"),
        };

        match client.run(callback) {
            Ok(()) => {}
            Err(e) => eprintln!("{e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use bybit::ws::response::OrderbookItem;
    use configuration::WriteStyle;
    use rstest::rstest;
    use triple_buffer::TripleBuffer;

    use super::*;

    struct TestConfig;
    impl AppConfigProvider for TestConfig {
        fn symbol(&self) -> &'static str {
            "ADAUSDT"
        }

        fn coin(&self) -> &'static str {
            "ADA"
        }

        fn order_size(&self) -> f64 {
            25.0
        }

        fn testnet(&self) -> bool {
            false
        }

        fn api_key(&self) -> &'static str {
            "api-key"
        }

        fn api_secret(&self) -> &'static str {
            "api-secret"
        }

        fn log_filter(&self) -> &'static str {
            "warn"
        }

        fn log_style(&self) -> WriteStyle {
            WriteStyle::Never
        }
    }

    fn create_public_websocket(
        recorder_order_books: Arc<ArrayQueue<OrderBook>>,
    ) -> PublicWebSocket {
        PublicWebSocket::new(recorder_order_books, &TestConfig)
    }

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

    #[rstest]
    #[case("snapshot", 2)]
    #[case("snapshot", 1)]
    #[case("delta", 1)]
    fn process_orderbook_response_replaces_and_publishes_snapshot(
        #[case] response_type: &str,
        #[case] update_id: u64,
    ) {
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        let mut public_websocket = create_public_websocket(Arc::clone(&recorder_order_books));
        public_websocket.order_book =
            create_order_book(&[(110.0, 1.0)], &[(90.0, 1.0)], 1000, 1000);
        let stale_strategy_order_book =
            create_order_book(&[(120.0, 1.0)], &[(80.0, 1.0)], 500, 500);
        let (mut order_book_publisher, mut strategy_order_books) =
            TripleBuffer::new(&stale_strategy_order_book).split();
        let response = BasePublicResponse {
            topic: "orderbook.50.ADAUSDT",
            type_: response_type,
            ts: 2000,
            data: Orderbook {
                s: "ADAUSDT",
                b: vec![OrderbookItem("99", "2"), OrderbookItem("98", "3")],
                a: vec![OrderbookItem("101", "2"), OrderbookItem("102", "3")],
                u: update_id,
                seq: Some(2),
            },
        };
        public_websocket.process_orderbook_response(&response, &mut order_book_publisher);

        let expected_order_book = create_order_book(
            &[(101.0, 2.0), (102.0, 3.0)],
            &[(99.0, 2.0), (98.0, 3.0)],
            2000,
            2000,
        );
        assert_eq!(public_websocket.order_book, expected_order_book);
        assert_eq!(strategy_order_books.read(), &expected_order_book);
        assert_eq!(recorder_order_books.pop(), Some(expected_order_book));
        assert!(recorder_order_books.is_empty());
    }

    #[test]
    fn process_orderbook_response_applies_delta_and_replaces_stale_publications() {
        let initial_order_book = create_order_book(&[(120.0, 1.0)], &[(80.0, 1.0)], 500, 500);
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        recorder_order_books
            .push(initial_order_book.clone())
            .expect("test recorder queue should have capacity");
        let (mut order_book_publisher, mut strategy_order_books) =
            TripleBuffer::new(&initial_order_book).split();
        let mut public_websocket = create_public_websocket(Arc::clone(&recorder_order_books));
        public_websocket.order_book =
            create_order_book(&[(101.0, 1.0)], &[(99.0, 1.0)], 1000, 1000);
        let response = BasePublicResponse {
            topic: "orderbook.50.ADAUSDT",
            type_: "delta",
            ts: 2000,
            data: Orderbook {
                s: "ADAUSDT",
                b: vec![OrderbookItem("99", "0"), OrderbookItem("98", "4")],
                a: vec![OrderbookItem("101", "2"), OrderbookItem("102", "3")],
                u: 2,
                seq: Some(2),
            },
        };
        public_websocket.process_orderbook_response(&response, &mut order_book_publisher);

        let expected_order_book =
            create_order_book(&[(101.0, 2.0), (102.0, 3.0)], &[(98.0, 4.0)], 2000, 2000);
        assert_eq!(public_websocket.order_book, expected_order_book);
        assert_eq!(strategy_order_books.read(), &expected_order_book);
        assert_eq!(recorder_order_books.pop(), Some(expected_order_book));
        assert!(recorder_order_books.is_empty());
    }

    #[test]
    fn process_delta_updates_existing_bid_and_ask_sizes() {
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        let mut public_websocket = create_public_websocket(recorder_order_books);
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

        public_websocket.process_delta(&delta);

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
        let mut public_websocket = create_public_websocket(recorder_order_books);
        public_websocket.order_book = create_order_book(&[(102.0, 2.0)], &[(98.0, 2.0)], 0, 0);
        let delta = Orderbook {
            s: "ADAUSDT",
            b: vec![OrderbookItem("97", "3"), OrderbookItem("99", "1")],
            a: vec![OrderbookItem("103", "3"), OrderbookItem("101", "1")],
            u: 2,
            seq: Some(2),
        };

        public_websocket.process_delta(&delta);

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
        let mut public_websocket = create_public_websocket(recorder_order_books);
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

        public_websocket.process_delta(&delta);

        assert_eq!(
            public_websocket.order_book,
            create_order_book(&[(102.0, 2.0)], &[(98.0, 2.0)], 0, 0)
        );
    }

    #[test]
    fn process_delta_applies_mixed_unordered_changes() {
        let recorder_order_books = Arc::new(ArrayQueue::new(1));
        let mut public_websocket = create_public_websocket(recorder_order_books);
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

        public_websocket.process_delta(&delta);

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
        let mut public_websocket = create_public_websocket(recorder_order_books);
        let initial_order_book = create_order_book(&[(101.0, 1.0)], &[(99.0, 1.0)], 1000, 1001);
        public_websocket.order_book = initial_order_book.clone();
        let delta = Orderbook {
            s: "ADAUSDT",
            b: Vec::new(),
            a: Vec::new(),
            u: 2,
            seq: Some(2),
        };

        public_websocket.process_delta(&delta);

        assert_eq!(public_websocket.order_book, initial_order_book);
    }
}
