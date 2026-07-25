use std::env;

use bybit::WebSocketApiClient;
use bybit::ws::private::PrivateWebsocketApiClient;
use bybit::ws::response::PrivateResponse;
use crossbeam_channel::Sender;
use log::warn;

use crate::OrderMessages;
use crate::bybit::utils::is_testnet;

#[derive(Debug)]
pub struct PrivateWebSocket {
    api_key: String,
    api_secret: String,
    to_oms: Sender<OrderMessages>,
    to_recorder: Sender<OrderMessages>,
}
impl PrivateWebSocket {
    // Temporary while secrets handling hasn't been implemented
    pub fn new(to_oms: Sender<OrderMessages>, to_recorder: Sender<OrderMessages>) -> Self {
        let api_key = env::var("API_KEY").expect("API_KEY env variable must not be blank.");
        let api_secret =
            env::var("API_SECRET").expect("API_SECRET env variable must not be blank.");
        PrivateWebSocket {
            to_oms,
            to_recorder,
            api_key,
            api_secret,
        }
    }

    fn get_ws_client(&self) -> PrivateWebsocketApiClient {
        if is_testnet() {
            return WebSocketApiClient::private()
                .testnet()
                .build_with_credentials(&self.api_key, &self.api_secret);
        }
        WebSocketApiClient::private().build_with_credentials(&self.api_key, &self.api_secret)
    }

    pub fn subscribe(&self) {
        let mut client = self.get_ws_client();
        client.subscribe_order();
        client.subscribe_execution();

        // TODO: Add subscription to Wallet stream.
        let callback = |res: PrivateResponse| match res {
            PrivateResponse::Order(res) => {
                let data = res.data;
                for order in data {
                    if order.order_link_id.is_empty() {
                        continue;
                    }
                    self.to_oms.send((&order).into()).unwrap();
                }
            }
            PrivateResponse::Execution(res) => {
                let data = res.data;
                for order in data {
                    if order.order_link_id.is_empty() {
                        continue;
                    }
                    self.to_oms.send((&order).into()).unwrap();
                    self.to_recorder.send((&order).into()).unwrap();
                }
            }
            PrivateResponse::Op(res) => {
                if !res.success {
                    warn!("{res:?}")
                }
            }
            PrivateResponse::Pong(_) => (),
            x => warn!("PrivateResponse::{x:?} not implemented"),
        };

        match client.run(callback) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", e),
        }
    }
}
