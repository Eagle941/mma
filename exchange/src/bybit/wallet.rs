use std::collections::HashMap;
use std::env;
use std::str::FromStr;

use attohttpc::Session;
use chrono::Utc;
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
    pub fn new() -> Self {
        let base_url = get_base_url();
        let api_key = env::var("API_KEY").expect("API_KEY env variable must not be blank.");
        let api_secret =
            env::var("API_SECRET").expect("API_SECRET env variable must not be blank.");
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
        )
        .unwrap();

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
                    // NOTE: I am not deserialising the result in a struct because it's not time
                    // critical and I don't need all the parameters.
                    if content["retCode"].as_i64().unwrap() == 0 {
                        // NOTE: there should be only one object under list because there is only
                        // one UNIFIED account.
                        // TODO: should I add a check that length of list is only 1?
                        for s in content["result"]["list"].as_array().unwrap() {
                            for coin in s["coin"].as_array().unwrap() {
                                let name = coin["coin"].as_str().unwrap().to_string();
                                let balance =
                                    f64::from_str(coin["equity"].as_str().unwrap()).unwrap();
                                self.coins.insert(name, balance);
                            }
                        }
                    } else {
                        panic!(
                            "Failed wallet-balance request. Code: {}. Msg: {}",
                            content["retCode"], content["retMsg"]
                        );
                    }
                }
            }
            Err(x) => {
                panic!("Failed to receive wallet-balance. Error {x}.");
            }
        }
    }
}
