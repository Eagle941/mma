use std::str::FromStr;

use configuration::AppConfigProvider;
use serde_json::Value;

use crate::InstrumentInfo;
use crate::bybit::utils::get_base_url;

/// Fetches and parses instrument metadata from Bybit.
///
/// # Panics
///
/// Panics if the request fails, Bybit rejects it, or the response does not
/// contain valid metadata for the configured symbol.
pub fn get_instrument_info(config: &dyn AppConfigProvider) -> InstrumentInfo {
    let symbol = config.symbol();
    let url = format!(
        "{}/v5/market/instruments-info?category=spot&symbol={}",
        get_base_url(config.testnet()),
        symbol
    );
    let res = attohttpc::get(url).send();
    match res {
        Ok(response) => {
            assert!(
                response.is_success(),
                "Failed instruments-info response for {}. Status code {}",
                symbol,
                response.status()
            );

            let content = response.text().unwrap();
            let content: Value = serde_json::from_str(&content).unwrap();
            let info = process_instrument_info_response(symbol, &content);
            log::info!("{info:#?}");
            info
        }
        Err(error) => {
            panic!("Failed to receive instrument info for {symbol}. Error {error}.");
        }
    }
}

fn process_instrument_info_response(symbol: &str, content: &Value) -> InstrumentInfo {
    // NOTE: despite using the parameter `symbol` in the request, Bybit returns all
    // the symbols.
    if content["retCode"].as_i64().unwrap() == 0 {
        for instrument in content["result"]["list"].as_array().unwrap() {
            if instrument["symbol"] == symbol {
                let base_coin = instrument["baseCoin"].as_str().unwrap().to_string();
                let quote_coin = instrument["quoteCoin"].as_str().unwrap().to_string();
                let base_precision = f64::from_str(
                    instrument["lotSizeFilter"]["basePrecision"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                let quote_precision = f64::from_str(
                    instrument["lotSizeFilter"]["quotePrecision"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                let tick_size = instrument["priceFilter"]["tickSize"].as_str().unwrap();
                let decimal_places = tick_size.len() - tick_size.find('.').unwrap_or_default() - 1;
                return InstrumentInfo::new(
                    symbol.to_string(),
                    base_coin,
                    quote_coin,
                    base_precision,
                    quote_precision,
                    f64::from_str(tick_size).unwrap(),
                    decimal_places,
                );
            }
        }
        panic!("Symbol {symbol} not found in instruments-info response.");
    }

    panic!(
        "Failed instruments-info request. Code: {}. Msg: {}",
        content["retCode"], content["retMsg"]
    );
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
                if x.is_success() {
                    let content = x.text().unwrap();
                    let content: Value = serde_json::from_str(&content).unwrap();
                    self.process_response(&content);
                } else {
                    panic!(
                        "Failed recent-trade response for {}. Status code {}",
                        self.symbol,
                        x.status()
                    );
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
    fn instrument_info_response_maps_instrument() {
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [create_instrument("0.001")]
            }
        });

        let info = process_instrument_info_response("ADAUSDT", &response);

        assert_eq!(
            info,
            InstrumentInfo::new(
                "ADAUSDT".to_string(),
                "ADA".to_string(),
                "USDT".to_string(),
                0.01,
                0.000_001,
                0.001,
                3,
            )
        );
    }

    #[rstest]
    #[case("1", 0)]
    #[case("0.1", 1)]
    #[case("0.001", 3)]
    #[case("0.000100", 6)]
    fn instrument_info_response_calculates_tick_size_decimal_places(
        #[case] tick_size: &str,
        #[case] expected_decimal_places: usize,
    ) {
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [create_instrument(tick_size)]
            }
        });

        let info = process_instrument_info_response("ADAUSDT", &response);

        assert_eq!(info.decimal_places(), expected_decimal_places);
    }

    #[test]
    #[should_panic(expected = "Symbol ADAUSDT not found in instruments-info response.")]
    fn instrument_info_response_panics_when_symbol_is_missing() {
        let response = json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": { "list": [] }
        });

        process_instrument_info_response("ADAUSDT", &response);
    }

    #[test]
    #[should_panic(expected = "Failed instruments-info request.")]
    fn instrument_info_response_panics_when_request_is_rejected() {
        let response = json!({
            "retCode": 10001,
            "retMsg": "Invalid symbol",
            "result": { "list": [] }
        });

        process_instrument_info_response("ADAUSDT", &response);
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

        assert_eq!(trades.last_price.to_bits(), 0.7123_f64.to_bits());
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
