use std::collections::HashMap;
use std::str::FromStr;

use attohttpc::Session;
use chrono::Utc;
use configuration::AppConfigProvider;
use serde_json::Value;

use crate::bybit::utils::{generate_signature, get_base_url};

#[derive(Clone, Debug)]
pub struct Wallet {
    base_url: String,
    api_key: String,
    api_secret: String,
    recv_window: String,
    session: Session,
    pub coins: HashMap<String, f64>,
}
impl Wallet {
    // NOTE: The default implementation doesn't have any sense for this struct.
    #[allow(clippy::new_without_default)]
    pub fn new(config: &dyn AppConfigProvider) -> Self {
        let base_url = get_base_url(config.testnet());
        let api_key = config.api_key().to_string();
        let api_secret = config.api_secret().to_string();
        // how long an HTTP request is valid. It is also used to prevent replay
        // attacks.
        // A smaller X-BAPI-RECV-WINDOW is more secure, but your request may
        // fail if the transmission time is greater than your X-BAPI-RECV-WINDOW.
        let recv_window = 1000.to_string();
        let mut session = Session::new();
        session.header("X-BAPI-API-KEY", &api_key);
        session.header("X-BAPI-RECV-WINDOW", &recv_window);
        let mut wallet = Wallet {
            base_url,
            api_key,
            api_secret,
            recv_window,
            session,
            coins: HashMap::default(),
        };
        wallet.get_wallet();
        log::info!("{:#?}", wallet.coins);
        wallet
    }

    fn get_wallet(&mut self) {
        let query = "accountType=UNIFIED";
        let url = format!("{}/v5/account/wallet-balance?{query}", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();
        let signature = generate_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window,
            query,
            &self.api_secret,
        );

        let res = self
            .session
            .get(url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-TIMESTAMP", time_ms)
            .send();
        match res {
            Ok(x) => {
                if !x.is_success() {
                    panic!("Failed wallet-balance response. Status code {}", x.status());
                } else {
                    let content = x.text().unwrap();
                    let content: Value = serde_json::from_str(&content).unwrap();
                    self.process_response(&content);
                }
            }
            Err(x) => {
                panic!("Failed to receive wallet-balance. Error {x}.");
            }
        }
    }

    fn process_response(&mut self, content: &Value) {
        if content["retCode"].as_i64().unwrap() == 0 {
            let accounts = content["result"]["list"].as_array().unwrap();
            let [account] = accounts.as_slice() else {
                panic!(
                    "Expected exactly one UNIFIED account in wallet-balance response, found {}.",
                    accounts.len()
                );
            };
            for coin in account["coin"].as_array().unwrap() {
                let name = coin["coin"].as_str().unwrap().to_string();
                let balance = f64::from_str(coin["equity"].as_str().unwrap()).unwrap();
                assert!(
                    !self.coins.contains_key(&name),
                    "Duplicate coin {name} in wallet-balance response."
                );
                self.coins.insert(name, balance);
            }
            return;
        }

        panic!(
            "Failed wallet-balance request. Code: {}. Msg: {}",
            content["retCode"], content["retMsg"]
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn create_wallet() -> Wallet {
        Wallet {
            base_url: "https://api.example.com".to_string(),
            api_key: "api-key".to_string(),
            api_secret: "api-secret".to_string(),
            recv_window: "1000".to_string(),
            session: Session::new(),
            coins: HashMap::default(),
        }
    }

    #[test]
    fn wallet_response_maps_coin_equities() {
        let mut wallet = create_wallet();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [{
                    "coin": [
                        { "coin": "ADA", "equity": "100.5" },
                        { "coin": "USDT", "equity": "250.25" },
                        { "coin": "BTC", "equity": "0" },
                        { "coin": "ETH", "equity": "-0.125" }
                    ]
                }]
            }
        });

        wallet.process_response(&response);

        assert_eq!(
            wallet.coins,
            HashMap::from([
                ("ADA".to_string(), 100.5),
                ("USDT".to_string(), 250.25),
                ("BTC".to_string(), 0.0),
                ("ETH".to_string(), -0.125),
            ])
        );
    }

    #[test]
    #[should_panic(
        expected = "Expected exactly one UNIFIED account in wallet-balance response, found 0."
    )]
    fn wallet_response_panics_when_account_is_missing() {
        let mut wallet = create_wallet();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": { "list": [] }
        });

        wallet.process_response(&response);
    }

    #[test]
    #[should_panic(
        expected = "Expected exactly one UNIFIED account in wallet-balance response, found 2."
    )]
    fn wallet_response_panics_for_multiple_accounts() {
        let mut wallet = create_wallet();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [
                    { "coin": [{ "coin": "ADA", "equity": "100.5" }] },
                    { "coin": [{ "coin": "USDT", "equity": "250.25" }] }
                ]
            }
        });

        wallet.process_response(&response);
    }

    #[test]
    fn wallet_response_with_no_coins_leaves_wallet_empty() {
        let mut wallet = create_wallet();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [{ "coin": [] }]
            }
        });

        wallet.process_response(&response);

        assert!(wallet.coins.is_empty());
    }

    #[test]
    #[should_panic(expected = "Duplicate coin ADA in wallet-balance response.")]
    fn wallet_response_panics_for_duplicate_coin() {
        let mut wallet = create_wallet();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [{
                    "coin": [
                        { "coin": "ADA", "equity": "100.5" },
                        { "coin": "ADA", "equity": "120.75" }
                    ]
                }]
            }
        });

        wallet.process_response(&response);
    }

    #[test]
    #[should_panic(expected = "Failed wallet-balance request.")]
    fn wallet_response_panics_when_request_is_rejected() {
        let mut wallet = create_wallet();
        let response = json!({
            "retCode": 10001,
            "retMsg": "Invalid request",
            "result": { "list": [] }
        });

        wallet.process_response(&response);
    }
}
