use std::f64;
use std::str::FromStr;

use configuration::AppConfigProvider;
use serde_json::Value;

use crate::bybit::utils::get_base_url;

// TODO: struct `Info` may need to become a shared struct common across
// Exchanges.
#[derive(Clone, Debug, PartialEq)]
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
    pub fn new(config: &dyn AppConfigProvider) -> Self {
        let base_url = get_base_url(config.testnet());
        let mut info = Info {
            base_url,
            symbol: config.symbol().to_string(),
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
                    self.process_response(&content);
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

    fn process_response(&mut self, content: &Value) {
        // NOTE: despite using the parameter `symbol` in the request, Bybit returns all
        // the symbols.
        if content["retCode"].as_i64().unwrap() == 0 {
            for instrument in content["result"]["list"].as_array().unwrap() {
                if instrument["symbol"] == self.symbol {
                    self.base_coin = instrument["baseCoin"].as_str().unwrap().to_string();
                    self.quote_coin = instrument["quoteCoin"].as_str().unwrap().to_string();
                    self.base_precision = f64::from_str(
                        instrument["lotSizeFilter"]["basePrecision"]
                            .as_str()
                            .unwrap(),
                    )
                    .unwrap();
                    self.quote_precision = f64::from_str(
                        instrument["lotSizeFilter"]["quotePrecision"]
                            .as_str()
                            .unwrap(),
                    )
                    .unwrap();
                    let tick_size = instrument["priceFilter"]["tickSize"].as_str().unwrap();
                    self.tick_size = f64::from_str(tick_size).unwrap();
                    // 0.001 --> 3
                    self.decimal_places =
                        tick_size.len() - tick_size.find('.').unwrap_or_default() - 1;
                    return;
                }
            }
            panic!(
                "Symbol {} not found in instruments-info response.",
                self.symbol
            );
        }

        panic!(
            "Failed instruments-info request. Code: {}. Msg: {}",
            content["retCode"], content["retMsg"]
        );
    }
}

// TODO: struct `Trades` may need to become a shared struct common across
// Exchanges.
#[derive(Clone, Debug, PartialEq)]
pub struct Trades {
    base_url: String,
    pub symbol: String,
    pub last_price: f64,
}
impl Trades {
    pub fn new(config: &dyn AppConfigProvider) -> Self {
        let base_url = get_base_url(config.testnet());
        let mut trades = Trades {
            base_url,
            symbol: config.symbol().to_string(),
            last_price: 0.0,
        };
        trades.get_trades();
        log::info!("{trades:#?}");
        trades
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
                    self.process_response(&content);
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

    fn process_response(&mut self, content: &Value) {
        if content["retCode"].as_i64().unwrap() == 0 {
            if let Some(trade) = content["result"]["list"].as_array().unwrap().first() {
                self.last_price = f64::from_str(trade["price"].as_str().unwrap()).unwrap();
                return;
            }
            panic!("Symbol {} not found in recent-trade response.", self.symbol);
        }

        panic!(
            "Failed recent-trade request. Code: {}. Msg: {}",
            content["retCode"], content["retMsg"]
        );
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    fn create_info() -> Info {
        Info {
            base_url: "https://api.example.com".to_string(),
            symbol: "ADAUSDT".to_string(),
            base_coin: String::default(),
            quote_coin: String::default(),
            base_precision: f64::NAN,
            quote_precision: f64::NAN,
            tick_size: f64::NAN,
            decimal_places: 0,
        }
    }

    fn create_instrument(tick_size: &str) -> Value {
        json!({
            "symbol": "ADAUSDT",
            "baseCoin": "ADA",
            "quoteCoin": "USDT",
            "lotSizeFilter": {
                "basePrecision": "0.01",
                "quotePrecision": "0.000001"
            },
            "priceFilter": {
                "tickSize": tick_size
            }
        })
    }

    fn create_trades() -> Trades {
        Trades {
            base_url: "https://api.example.com".to_string(),
            symbol: "ADAUSDT".to_string(),
            last_price: 0.0,
        }
    }

    #[test]
    fn info_response_maps_instrument() {
        let mut info = create_info();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [create_instrument("0.001")]
            }
        });

        info.process_response(&response);

        assert_eq!(
            info,
            Info {
                base_url: "https://api.example.com".to_string(),
                symbol: "ADAUSDT".to_string(),
                base_coin: "ADA".to_string(),
                quote_coin: "USDT".to_string(),
                base_precision: 0.01,
                quote_precision: 0.000001,
                tick_size: 0.001,
                decimal_places: 3,
            }
        );
    }

    #[rstest]
    #[case("1", 0)]
    #[case("0.1", 1)]
    #[case("0.001", 3)]
    #[case("0.000100", 6)]
    fn info_response_calculates_tick_size_decimal_places(
        #[case] tick_size: &str,
        #[case] expected_decimal_places: usize,
    ) {
        let mut info = create_info();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [create_instrument(tick_size)]
            }
        });

        info.process_response(&response);

        assert_eq!(info.decimal_places, expected_decimal_places);
    }

    #[test]
    #[should_panic(expected = "Symbol ADAUSDT not found in instruments-info response.")]
    fn info_response_panics_when_symbol_is_missing() {
        let mut info = create_info();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": { "list": [] }
        });

        info.process_response(&response);
    }

    #[test]
    #[should_panic(expected = "Failed instruments-info request.")]
    fn info_response_panics_when_request_is_rejected() {
        let mut info = create_info();
        let response = json!({
            "retCode": 10001,
            "retMsg": "Invalid symbol",
            "result": { "list": [] }
        });

        info.process_response(&response);
    }

    #[test]
    fn trades_response_maps_first_trade_price() {
        let mut trades = create_trades();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [
                    { "price": "0.7123" },
                    { "price": "0.7000" }
                ]
            }
        });

        trades.process_response(&response);

        assert_eq!(trades.last_price, 0.7123);
    }

    #[test]
    #[should_panic(expected = "Symbol ADAUSDT not found in recent-trade response.")]
    fn trades_response_panics_when_trade_list_is_empty() {
        let mut trades = create_trades();
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": { "list": [] }
        });

        trades.process_response(&response);
    }

    #[test]
    #[should_panic(expected = "Failed recent-trade request.")]
    fn trades_response_panics_when_request_is_rejected() {
        let mut trades = create_trades();
        let response = json!({
            "retCode": 10001,
            "retMsg": "Invalid symbol",
            "result": { "list": [] }
        });

        trades.process_response(&response);
    }
}
