use attohttpc::Session;
use chrono::Utc;
use hex;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Serialize, Deserialize)]
pub enum OrderSide {
    BUY,
    SELL,
}

#[derive(Serialize, Deserialize)]
pub enum OrderType {
    MARKET,
    LIMIT,
}

#[derive(Serialize, Deserialize)]
pub struct Order {
    symbol: String,
    side: OrderSide,
    order_type: OrderType,
    qty: f64,
    price: f64,
}

pub struct OrderManagementSystem {
    base_url: String,
    api_key: String,
    api_secret: String,
    recv_window: u32,
    session: Session,
}
impl OrderManagementSystem {
    pub fn new(base_url: String, api_key: String, api_secret: String) -> Self {
        // how long an HTTP request is valid. It is also used to prevent replay
        // attacks.
        // A smaller X-BAPI-RECV-WINDOW is more secure, but your request may
        // fail if the transmission time is greater than your X-BAPI-RECV-WINDOW.
        let recv_window = 1000;
        let mut session = Session::new();
        session.header("X-BAPI-SIGN", &api_secret);
        session.header("X-BAPI-API-KEY", &api_key);
        session.header("X-BAPI-RECV-WINDOW", recv_window);
        OrderManagementSystem {
            base_url,
            api_key,
            api_secret,
            recv_window,
            session,
        }
    }

    pub fn submit_order(&self, order: Order) -> attohttpc::Result {
        // TODO: identify more efficient methods than `serde`
        // TODO: add additional exchange mandatory parameters
        let url = format!("{}/v5/order/create", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        let resp = self
            .session
            .post(url)
            .header("X-BAPI-SIGN", time_ms.to_string())
            .header("X-BAPI-TIMESTAMP", time_ms.to_string())
            .json(&order)?
            .send()?;
        Ok(())
    }

    fn generate_post_signature(
        timestamp: &str,
        api_key: &str,
        recv_window: &str,
        params: &serde_json::Map<String, Value>,
        api_secret: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        // TODO: optimise signature generation
        let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(timestamp.as_bytes());
        mac.update(api_key.as_bytes());
        mac.update(recv_window.as_bytes());
        mac.update(serde_json::to_string(&params)?.as_bytes());

        let result = mac.finalize();
        let code_bytes = result.into_bytes();
        Ok(hex::encode(code_bytes))
    }
}
