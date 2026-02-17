use std::thread;

use attohttpc::Session;
use chrono::Utc;
use hex;
use hmac::{Hmac, Mac};
use http::StatusCode;
use serde_json::json;
use sha2::Sha256;

use crate::Order;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Default)]
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
        let base_url = "https://api-testnet.bybit.com".to_string();
        let api_key = "xxxxxxxx".to_string();
        let api_secret = "xxxxxxxxxxx".to_string();
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

    pub fn submit_order(&self, order: Order) {
        // TODO: identify more efficient methods than `serde`
        // TODO: add additional exchange mandatory parameters
        let url = format!("{}/v5/order/create", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        // TODO: add timeInForce parameter
        let body = json!({
            "category": "spot",
            "symbol": order.symbol,
            "side": order.side,
            "orderType": order.order_type,
            "qty": order.qty,
            "price": order.price
        });
        let signature = Self::generate_post_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window.to_string(),
            &body.to_string(),
            &self.api_secret,
        )
        .unwrap();

        // TODO: use non-blocking HTTP request and avoid creating a new thread.
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
                    if x.status() != StatusCode::OK {
                        panic!("Failed order response {x:?}\n{body:#?}");
                    }
                }
                Err(x) => {
                    panic!("Failed to send order {x}\n{body:#?}");
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
