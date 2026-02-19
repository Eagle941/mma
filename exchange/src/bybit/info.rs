use std::str::FromStr;
use std::{env, f64};

use serde_json::Value;
// TODO: struct `info` may need to become a shared struct common across
// Exchanges.
#[derive(Clone, Debug)]
pub struct Info {
    base_url: String,
    symbol: String,
    base_coin: String,
    quote_coin: String,
    base_precision: f64,
    quote_precision: f64,
}
impl Info {
    pub fn new(symbol: String) -> Self {
        // TODO: add option to switch between testnet and production.
        let base_url = "https://api-testnet.bybit.com".to_string();
        let mut info = Info {
            base_url,
            symbol,
            base_coin: String::default(),
            quote_coin: String::default(),
            base_precision: f64::NAN,
            quote_precision: f64::NAN,
        };
        info.get_info();
        info
    }

    pub fn factory() -> Self {
        let symbol = env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");
        Self::new(symbol)
    }

    pub fn get_info(&mut self) {
        let url = format!(
            "{}/v5/market/instruments-info?category=spot&symbol={}",
            self.base_url, self.symbol
        );
        let res = attohttpc::get(url).send();
        match res {
            Ok(x) => {
                if !x.is_success() {
                    panic!(
                        "Failed instruments-info response for {}. Status code {}",
                        self.symbol,
                        x.status()
                    );
                } else {
                    let content = x.text().unwrap();
                    let content: Value = serde_json::from_str(&content).unwrap();
                    // NOTE: I am not deserialising the result in a struct because it's not time
                    // critical and I don't need all the parameters.
                    // NOTE: despite using the parameter `symbol` in the request, Bybit returns all
                    // the symbols.
                    if content["retCode"].as_i64().unwrap() == 0 {
                        for s in content["result"]["list"].as_array().unwrap() {
                            if s["symbol"] == self.symbol {
                                self.base_coin = s["baseCoin"].as_str().unwrap().to_string();
                                self.quote_coin = s["quoteCoin"].as_str().unwrap().to_string();
                                self.base_precision = f64::from_str(
                                    s["lotSizeFilter"]["basePrecision"].as_str().unwrap(),
                                )
                                .unwrap();
                                self.quote_precision = f64::from_str(
                                    s["lotSizeFilter"]["quotePrecision"].as_str().unwrap(),
                                )
                                .unwrap();
                                return;
                            }
                        }
                        panic!(
                            "Symbol {} not found in instruments-info response.",
                            self.symbol
                        );
                    } else {
                        panic!(
                            "Failed instruments-info request. Code: {}. Msg: {}",
                            content["retCode"], content["retMsg"]
                        );
                    }
                }
            }
            Err(x) => {
                panic!(
                    "Failed to receive instrument info for {}. Error {x}.",
                    self.symbol
                );
            }
        }
    }
}
