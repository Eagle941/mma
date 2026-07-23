use bybit::WebSocketApiClient;
use bybit::ws::private::PrivateWebsocketApiClient;
use bybit::ws::response::PrivateResponse;
use configuration::AppConfigProvider;
use crossbeam_channel::Sender;
use log::warn;

use crate::OrderEvent;

#[derive(Debug)]
pub struct PrivateWebSocket {
    testnet: bool,
    api_key: String,
    api_secret: String,
    to_oms: Sender<OrderEvent>,
    to_recorder: Sender<OrderEvent>,
}
impl PrivateWebSocket {
    pub fn new(
        to_oms: Sender<OrderEvent>,
        to_recorder: Sender<OrderEvent>,
        config: &dyn AppConfigProvider,
    ) -> Self {
        PrivateWebSocket {
            testnet: config.testnet(),
            to_oms,
            to_recorder,
            api_key: config.api_key().to_string(),
            api_secret: config.api_secret().to_string(),
        }
    }

    fn get_ws_client(&self) -> PrivateWebsocketApiClient {
        if self.testnet {
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
