use std::sync::Arc;

use bybit::ws::response::{Orderbook, SpotPublicResponse};
use bybit::ws::spot::{OrderbookDepth, SpotWebsocketApiClient};
use bybit::WebSocketApiClient;
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
