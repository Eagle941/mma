use std::env;
use std::sync::mpsc::Sender;

use bybit::WebSocketApiClient;
use bybit::ws::response::PrivateResponse;

use crate::OrderMessages;

#[derive(Debug)]
pub struct PrivateWebSocket {
    api_key: String,
    api_secret: String,
    to_oms: Sender<OrderMessages>,
}
impl PrivateWebSocket {
    // Temporary while secrets handling hasn't been implemented
    pub fn new(to_oms: Sender<OrderMessages>) -> Self {
        let api_key = env::var("API_KEY").expect("API_KEY env variable must not be blank.");
        let api_secret =
            env::var("API_SECRET").expect("API_SECRET env variable must not be blank.");
        PrivateWebSocket {
            to_oms,
            api_key,
            api_secret,
        }
    }

    pub fn subscribe(&self) {
        // TODO: add option to switch between testnet and production.
        let mut client = WebSocketApiClient::private()
            .testnet()
            .build_with_credentials(&self.api_key, &self.api_secret);
        client.subscribe_order();
        client.subscribe_execution();
        client.subscribe_wallet();

        let callback = |res: PrivateResponse| match res {
            PrivateResponse::Order(res) => {
                let data = res.data;
                for order in data {
                    self.to_oms.send((&order).into()).unwrap();
                }
            }
            PrivateResponse::Execution(res) => {
                let data = res.data;
                for order in data {
                    self.to_oms.send((&order).into()).unwrap();
                }
            }
            PrivateResponse::Wallet(res) => {
                let data = res.data;
                println!("{data:#?}");
            }
            PrivateResponse::Op(res) => {
                if !res.success {
                    println!("{res:?}")
                }
            }
            PrivateResponse::Pong(_) => (),
            x => println!("PrivateResponse::{x:?} not implemented"),
        };

        match client.run(callback) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", e),
        }
    }
}
