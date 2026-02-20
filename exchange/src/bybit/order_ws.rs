use std::env;
use std::sync::mpsc::Sender;

use bybit::ws::response::PrivateResponse;
use bybit::WebSocketApiClient;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::Order;

// TODO: Add automatic casting of `result` to various struct types like in bybit
// library.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CommonResponse<'a> {
    pub ret_code: u32,
    pub ret_msg: &'a str,
    pub result: Box<RawValue>,
    pub ret_ext_info: Box<RawValue>,
    pub time: u64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse<'a> {
    pub order_id: &'a str,
    pub order_link_id: &'a str,
}

#[derive(Debug)]
pub struct OrderWebSocket {
    api_key: String,
    api_secret: String,
    to_oms: Sender<Order>,
}
impl OrderWebSocket {
    // Temporary while secrets handling hasn't been implemented
    pub fn new(to_oms: Sender<Order>) -> Self {
        let api_key = env::var("API_KEY").expect("API_KEY env variable must not be blank.");
        let api_secret =
            env::var("API_SECRET").expect("API_SECRET env variable must not be blank.");
        OrderWebSocket {
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

        let callback = |res: PrivateResponse| match res {
            PrivateResponse::Order(res) => {
                let data = res.data;
                for order in data {
                    self.to_oms.send((&order).into()).unwrap();
                }
            }
            PrivateResponse::Op(res) => {
                if !res.success {
                    println!("{res:?}")
                }
            }
            x => println!("PrivateResponse::{x:?} not implemented"),
        };

        match client.run(callback) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", e),
        }
    }
}
