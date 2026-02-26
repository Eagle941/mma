use std::{env, thread};

use attohttpc::Session;
use chrono::Utc;
use hex;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;
use serde_json::value::RawValue;
use sha2::Sha256;

use crate::{OrderAmendedBuilder, OrderBuilder};

type HmacSha256 = Hmac<Sha256>;

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
pub struct OrderHandler {
    base_url: String,
    api_key: String,
    api_secret: String,
    recv_window: u32,
    session: Session,
}
impl OrderHandler {
    // Temporary while secrets handling hasn't been implemented
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        // TODO: add option to switch between testnet and production.
        let base_url = "https://api-testnet.bybit.com".to_string();
        let api_key = env::var("API_KEY").expect("API_KEY env variable must not be blank.");
        let api_secret =
            env::var("API_SECRET").expect("API_SECRET env variable must not be blank.");
        // how long an HTTP request is valid. It is also used to prevent replay
        // attacks.
        // A smaller X-BAPI-RECV-WINDOW is more secure, but your request may
        // fail if the transmission time is greater than your X-BAPI-RECV-WINDOW.
        let recv_window = 1000;
        let mut session = Session::new();
        session.header("X-BAPI-SIGN", &api_secret);
        session.header("X-BAPI-API-KEY", &api_key);
        session.header("X-BAPI-RECV-WINDOW", recv_window);
        OrderHandler {
            base_url,
            api_key,
            api_secret,
            recv_window,
            session,
        }
    }

    pub fn amend_order(&self, order_builder: &OrderAmendedBuilder) {
        // TODO: identify more efficient methods than `serde`
        // TODO: add support for all additional exchange non-mandatory parameters
        let url = format!("{}/v5/order/amend", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        // NOTE: always populate price and qty even if they don't change to allow the
        // OMS to be synced up correctly.
        let mut body = json!({
            "category": "spot",
            "symbol": order_builder.symbol,
            "orderLinkId": order_builder.order_link_id.to_string(),
        });
        if order_builder.new_qty {
            body["qty"] = json!(order_builder.qty);
        }
        if order_builder.new_price {
            body["price"] = json!(order_builder.price);
        }
        let signature = Self::generate_post_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window.to_string(),
            &body.to_string(),
            &self.api_secret,
        )
        .unwrap();
        // TODO: use non-blocking HTTP request and avoid creating a new thread.
        // TODO: add orderLinkId for optimisations
        // TODO: move from HTTP request to WebSocket
        // TODO: find a proper way to deal with failed orders
        thread::scope(|_| {
            let res = self
                .session
                .post(url)
                .header("X-BAPI-API-KEY", &self.api_key)
                .header("X-BAPI-SIGN", signature)
                .header("X-BAPI-TIMESTAMP", time_ms.to_string())
                .header("X-BAPI-RECV-WINDOW", self.recv_window.to_string())
                .json(&body)
                .unwrap()
                .send();
            match res {
                Ok(x) => {
                    if !x.is_success() {
                        panic!("Failed order amend response. Status code {}", x.status());
                    } else {
                        let content = x.text().unwrap();
                        let content: CommonResponse = serde_json::from_str(&content).unwrap();
                        if content.ret_code == 0 {
                            // Do nothing
                        } else if content.ret_code == 10002
                            || content.ret_code == 170194
                            || content.ret_code == 170193
                            || content.ret_code == 10001
                            || content.ret_code == 170213
                        {
                            // Timestamp for this request is outside of the
                            // recvWindow.
                            // NOTE: if the order request took too long to
                            // arrive, just skip the
                            // order and let the strategy send a new one in the
                            // next cycle with
                            // updated values.
                            // Sell order price cannot be lower than %s.
                            // Buy order price cannot be higher than %s.
                            // NOTE: This error occurs when order book changed
                            // while submitting the
                            // order. Wait for the next cycle to submit another
                            // order at a different
                            // price.
                            // The order remains unchanged as the parameters
                            // entered match the
                            // existing ones.
                            // NOTE: This error occurs
                            // when two identical amend orders are issued at the
                            // same time due to
                            // the latency to receive the HTTP response.
                            // Order does not exist.
                            // NOTE: This error occurs when an order is filled
                            // during the amend
                            // request.
                            println!(
                                "Amend order error. {} Code: {}. Msg: {}",
                                order_builder.order_link_id, content.ret_code, content.ret_msg
                            );
                        } else {
                            panic!(
                                "Failed order amend request. Code: {}. Msg: {}",
                                content.ret_code, content.ret_msg
                            );
                        }
                    }
                }
                Err(x) => {
                    panic!("Failed to send order amend request {x}\n{body:#?}");
                }
            }
        });
    }

    pub fn submit_order(&self, order_builder: &OrderBuilder, order_link_id: u64) {
        // TODO: identify more efficient methods than `serde`
        // TODO: add support for all additional exchange non-mandatory parameters
        let url = format!("{}/v5/order/create", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        // TODO: add timeInForce parameter
        let body = json!({
            "orderLinkId": order_link_id.to_string(),
            "category": "spot",
            "isLeverage": 1,
            "symbol": order_builder.symbol,
            "side": order_builder.side,
            "orderType": order_builder.order_type,
            "qty": order_builder.qty.to_string(),
            "price": order_builder.price
        });
        let signature = Self::generate_post_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window.to_string(),
            &body.to_string(),
            &self.api_secret,
        )
        .unwrap();
        // println!("Order {:#?}", body);
        // TODO: use non-blocking HTTP request and avoid creating a new thread.
        // TODO: add orderLinkId for optimisations
        // TODO: move from HTTP request to WebSocket
        // TODO: find a proper way to deal with failed orders
        thread::scope(|_| {
            let res = self
                .session
                .post(url)
                .header("X-BAPI-API-KEY", &self.api_key)
                .header("X-BAPI-SIGN", signature)
                .header("X-BAPI-TIMESTAMP", time_ms.to_string())
                .header("X-BAPI-RECV-WINDOW", self.recv_window.to_string())
                .json(&body)
                .unwrap()
                .send();
            match res {
                Ok(x) => {
                    if !x.is_success() {
                        panic!("Failed order response. Status code {}", x.status());
                    } else {
                        let content = x.text().unwrap();
                        let content: CommonResponse = serde_json::from_str(&content).unwrap();
                        if content.ret_code == 0 {
                            // Do nothing
                        } else if content.ret_code == 10002
                            || content.ret_code == 170194
                            || content.ret_code == 170193
                        {
                            // Timestamp for this request is outside of the
                            // recvWindow.
                            // NOTE: if the order request took too long to
                            // arrive, just skip the
                            // order and let the strategy send a new one in the
                            // next cycle with
                            // updated values.
                            // Sell order price cannot be lower than %s.
                            // Buy order price cannot be higher than %s.
                            // NOTE: This error occurs when order book changed
                            // while submitting the
                            // order. Wait for the next cycle to submit another
                            // order at a different
                            // price.
                            println!(
                                "Submit order error. {} Code: {}. Msg: {}",
                                order_link_id, content.ret_code, content.ret_msg
                            );
                        } else {
                            panic!(
                                "Failed order request. Code: {}. Msg: {}",
                                content.ret_code, content.ret_msg
                            );
                        }
                    }
                }
                Err(x) => {
                    panic!("Failed to send order request {x}\n{body:#?}");
                }
            }
        });
    }

    fn generate_post_signature(
        timestamp: &str,
        api_key: &str,
        recv_window: &str,
        params: &str,
        api_secret: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        // TODO: optimise signature generation
        let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(timestamp.as_bytes());
        mac.update(api_key.as_bytes());
        mac.update(recv_window.as_bytes());
        mac.update(params.as_bytes());

        let result = mac.finalize();
        let code_bytes = result.into_bytes();
        Ok(hex::encode(code_bytes))
    }
}
