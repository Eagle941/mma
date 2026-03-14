use std::env;
use std::str::FromStr;

use chrono::Utc;
use log::{info, warn};
use log_execution_time::log_execution_time;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::json;
use serde_json::value::RawValue;

use crate::bybit::utils::generate_signature;
use crate::{OrderAmendedBuilder, OrderBuilder};

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

#[derive(Clone, Debug)]
pub struct OrderHandler {
    base_url: String,
    api_key: String,
    api_secret: String,
    recv_window: String,
    session: Client,
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
        let recv_window = 1000.to_string();

        let mut headers = HeaderMap::new();
        headers.insert("X-BAPI-API-KEY", HeaderValue::from_str(&api_key).unwrap());
        headers.insert(
            "X-BAPI-RECV-WINDOW",
            HeaderValue::from_str(&recv_window.to_string()).unwrap(),
        );
        let session = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("Failed to build reqwest client");

        OrderHandler {
            base_url,
            api_key,
            api_secret,
            recv_window,
            session,
        }
    }

    // TODO: introduce kill-switch when bot crashes or it's killed with ^c
    #[log_execution_time]
    pub fn cancel_all(&self) {
        // TODO: identify more efficient methods than `serde`
        // TODO: add support for all additional exchange non-mandatory parameters
        let url = format!("{}/v5/order/cancel-all", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        let body = json!({
            "category": "spot",
        });
        let signature = generate_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window,
            &body.to_string(),
            &self.api_secret,
        )
        .unwrap();
        let request = self
            .session
            .post(url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-TIMESTAMP", time_ms)
            .json(&body);
        // NOTE: it is assumed this request won't fail
        tokio::spawn(async move {
            let start = std::time::Instant::now();

            // NOTE: error 999 is used because an order id is required, but there is no
            // order id for cancel-all. I am not using an Option type to reduce the
            // overhead.
            Self::send_request(request, 999).await;

            let duration = start.elapsed();
            log::info!("Execution time of `cancel_all`: {:.2?}", duration);
        });
    }

    #[log_execution_time]
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
        let signature = generate_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window,
            &body.to_string(),
            &self.api_secret,
        )
        .unwrap();
        let request = self
            .session
            .post(url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-TIMESTAMP", time_ms)
            .json(&body);
        let order_link_id = order_builder.order_link_id;
        // TODO: move from HTTP request to WebSocket
        // TODO: find a proper way to deal with failed orders
        tokio::spawn(async move {
            let start = std::time::Instant::now();

            Self::send_request(request, order_link_id).await;

            let duration = start.elapsed();
            log::info!("Execution time of `send_request`: {:.2?}", duration);
        });
    }

    #[log_execution_time]
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
            "price": order_builder.price,
            "timeInForce": "FOK" // Fill or Kill
        });
        let signature = generate_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window,
            &body.to_string(),
            &self.api_secret,
        )
        .unwrap();
        let request = self
            .session
            .post(url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-TIMESTAMP", time_ms)
            .json(&body);
        // TODO: move from HTTP request to WebSocket
        // TODO: find a proper way to deal with failed orders
        tokio::spawn(async move {
            let start = std::time::Instant::now();

            Self::send_request(request, order_link_id).await;

            let duration = start.elapsed();
            log::info!("Execution time of `send_request`: {:.2?}", duration);
        });
    }

    async fn send_request(request: RequestBuilder, order_link_id: u64) {
        let res = request.send().await;
        match res {
            Ok(x) => {
                if !x.status().is_success() {
                    panic!("Failed order response. Status code {}", x.status());
                }
                let url = x.url().clone();
                // NOTE: The current handling of zero requests left is very simple because HTTP
                // requests will be replaced by WebSocket orders and the test strategy will run
                // at low iteration rate to guarantee safety.
                let api_limit_status: u8 = (u8::from_str(
                    x.headers()
                        .get("x-bapi-limit-status")
                        .unwrap_or(&HeaderValue::from_str("10").unwrap())
                        .to_str()
                        .unwrap(),
                ))
                .unwrap();
                if api_limit_status == 0 {
                    panic!("Zero requests left for {url}");
                } else if api_limit_status <= 2 {
                    warn!("Remaining {api_limit_status} requests for {url}");
                }
                let raw_text = x.text().await.expect("Failed to read response text");
                let content = serde_json::from_str::<CommonResponse>(&raw_text).unwrap();
                match content.ret_code {
                    0 => (),
                    10001 | 10002 | 170194 | 170193 | 170213 => {
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
                        // same time due to the latency to receive the HTTP response.
                        // Order does not exist.
                        // NOTE: This error occurs when an order is filled
                        // during the amend
                        // request.
                        info!(
                            "{url} error. {} Code: {}. Msg: {}",
                            order_link_id, content.ret_code, content.ret_msg
                        );
                    }
                    10016 => {
                        // internal server error
                        // This is triggered when the request rate limit is
                        // exceeded
                        panic!("{url} Internal server error.")
                    }
                    _ => panic!(
                        "Failed {url} request. Code: {}. Msg: {}",
                        content.ret_code, content.ret_msg
                    ),
                }
            }
            Err(x) => {
                panic!("Failed to send order request {x}");
            }
        }
    }
}
