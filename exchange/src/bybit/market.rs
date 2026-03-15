use std::str::FromStr;
use std::{env, f64};

use serde_json::Value;

// TODO: struct `Info` may need to become a shared struct common across
// Exchanges.
#[derive(Clone, Debug)]
pub struct Info {
    base_url: String,
    pub symbol: String,
    pub base_coin: String,
    pub quote_coin: String,
    pub base_precision: f64,
    pub quote_precision: f64,
    pub tick_size: f64,
    pub decimal_places: usize,
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
            tick_size: f64::NAN,
            decimal_places: 0,
        };
        info.get_info();
        log::info!("{info:#?}");
        info
    }

    pub fn factory() -> Self {
        let symbol = env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");
        Self::new(symbol)
    }

    fn get_info(&mut self) {
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
                                let tick_size_str = s["priceFilter"]["tickSize"].as_str().unwrap();
                                self.tick_size = f64::from_str(tick_size_str).unwrap();
                                // 0.001 --> 3
                                self.decimal_places = tick_size_str.len()
                                    - tick_size_str.find(".").unwrap_or_default()
                                    - 1;
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

// TODO: struct `Trades` may need to become a shared struct common across
// Exchanges.
#[derive(Clone, Debug)]
pub struct Trades {
    base_url: String,
    pub symbol: String,
    pub price: f64,
}
impl Trades {
    pub fn new(symbol: String) -> Self {
        // TODO: add option to switch between testnet and production.
        let base_url = "https://api-testnet.bybit.com".to_string();
        let mut trades = Trades {
            base_url,
            symbol,
            price: 0.0,
        };
        trades.get_trades();
        log::info!("{trades:#?}");
        trades
    }

    pub fn factory() -> Self {
        let symbol = env::var("MMA_SYMBOL").expect("MMA_SYMBOL env variable must not be blank.");
        Self::new(symbol)
    }

    fn get_trades(&mut self) {
        let url = format!(
            "{}/v5/market/recent-trade?category=spot&symbol={}&limit=1",
            self.base_url, self.symbol
        );
        let res = attohttpc::get(url).send();
        match res {
            Ok(x) => {
                if !x.is_success() {
                    panic!(
                        "Failed recent-trade response for {}. Status code {}",
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
                        if let Some(s) = content["result"]["list"].as_array().unwrap().iter().next()
                        {
                            self.price = f64::from_str(s["price"].as_str().unwrap()).unwrap();
                            return;
                        }
                        panic!("Symbol {} not found in recent-trade response.", self.symbol);
                    } else {
                        panic!(
                            "Failed recent-trade request. Code: {}. Msg: {}",
                            content["retCode"], content["retMsg"]
                        );
                    }
                }
            }
            Err(x) => {
                panic!(
                    "Failed to receive recent-trade for {}. Error {x}.",
                    self.symbol
                );
            }
        }
    }
}
