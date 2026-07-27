use std::str::FromStr;

use chrono::Utc;
use configuration::AppConfigProvider;
use crossbeam_channel::Sender;
use log::{error, info, warn};
use log_execution_time::log_execution_time;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::bybit::utils::{generate_signature, get_base_url};
use crate::{OrderAmendedBuilder, OrderBuilder, OrderEvent, OrderGateway};

// TODO: Add automatic casting of `result` to various struct types like in bybit
// library.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonResponse<'a> {
    pub ret_code: u32,
    pub ret_msg: &'a str,
    pub result: Box<RawValue>,
    pub ret_ext_info: Box<RawValue>,
    pub time: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse<'a> {
    pub order_id: &'a str,
    pub order_link_id: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestOutcome {
    Accepted,
    /// return code
    Rejected(u32),
    Unknown,
}

#[derive(Clone, Debug)]
pub struct OrderHandler {
    base_url: String,
    api_key: String,
    api_secret: String,
    recv_window: String,
    session: Client,
    to_oms: Sender<OrderEvent>,
}
impl OrderHandler {
    #[allow(clippy::new_without_default)]
    pub fn new(to_oms: Sender<OrderEvent>, config: &dyn AppConfigProvider) -> Self {
        let base_url = get_base_url(config.testnet());
        let api_key = config.api_key().to_string();
        let api_secret = config.api_secret().to_string();
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
            to_oms,
        }
    }

    fn serialize_quantity(qty: f64) -> String {
        assert!(
            qty.is_finite() && qty > 0.0,
            "Order quantity must be finite and greater than zero."
        );
        qty.to_string()
    }

    fn serialize_price(price: &str) -> String {
        let price_value =
            f64::from_str(price).expect("Order price must be a finite number greater than zero.");
        assert!(
            price_value.is_finite() && price_value > 0.0,
            "Order price must be a finite number greater than zero."
        );
        price.to_string()
    }

    fn submit_order_body(order_builder: &OrderBuilder, order_link_id: u64) -> Value {
        let qty = Self::serialize_quantity(order_builder.qty);
        let price = Self::serialize_price(&order_builder.price);
        json!({
            "orderLinkId": order_link_id.to_string(),
            "category": "spot",
            "isLeverage": 1,
            "symbol": order_builder.symbol,
            "side": order_builder.side,
            "orderType": order_builder.order_type,
            "qty": qty,
            "price": price,
            "timeInForce": "PostOnly",
            "smpType": "CancelBoth",
            "marketUnit": "baseCoin"
        })
    }

    fn amend_order_body(order_builder: &OrderAmendedBuilder) -> Value {
        // NOTE: always populate price and qty even if they don't change to allow the
        // OMS to be synced up correctly.
        let mut body = json!({
            "category": "spot",
            "symbol": order_builder.symbol,
            "orderLinkId": order_builder.order_link_id.to_string(),
        });
        if order_builder.new_qty {
            body["qty"] = json!(Self::serialize_quantity(order_builder.qty));
        }
        if order_builder.new_price {
            body["price"] = json!(Self::serialize_price(&order_builder.price));
        }
        body
    }

    fn cancel_all_body() -> Value {
        json!({ "category": "spot" })
    }

    fn classify_response(
        content: &CommonResponse,
        url: &str,
        order_link_id: u64,
    ) -> RequestOutcome {
        // TODO: replace ret_code matching with enum
        match content.ret_code {
            0 => RequestOutcome::Accepted,
            // Timestamp for this request is outside of the
            // recvWindow.
            // NOTE: if the order request took too long to
            // arrive, just skip the order and let the strategy send a new one in the
            // next cycle with updated values.
            // Sell order price cannot be lower than %s.
            // Buy order price cannot be higher than %s.
            // NOTE: This error occurs when order book changed
            // while submitting the order. Wait for the next cycle to submit another
            // order at a different price.
            // The order remains unchanged as the parameters
            // entered match the existing ones.
            // NOTE: This error occurs
            // when two identical amend orders are issued at the
            // same time due to the latency to receive the HTTP response.
            // Order does not exist.
            // NOTE: This error occurs when an order is filled
            // during the amend
            // request.
            10001 | 10002 | 170194 | 170193 | 170213 => {
                info!(
                    "{url} error. {} Code: {}. Msg: {}",
                    order_link_id, content.ret_code, content.ret_msg
                );
                RequestOutcome::Rejected(content.ret_code)
            }
            // Server Timeout
            // internal server error
            // For orders, this is triggered when the request rate limit is
            // exceeded.
            // The call to quick-repayment also returns 10016 even when the repayment
            // was executed successfully, therefore I don't want to panic.
            // NOTE: this was changed from panic! to error! for quick-repayment not to
            // crash the bot.
            10000 | 10016 => {
                error!("{url} Internal server error.");
                RequestOutcome::Rejected(content.ret_code)
            }
            _ => {
                // Panic in case of unknown code to catch bugs and undefined behaviour.
                panic!(
                    "Failed {url} request. Code: {}. Msg: {}",
                    content.ret_code, content.ret_msg
                )
            }
        }
    }

    fn report_submission_outcome(
        outcome: RequestOutcome,
        order_link_id: u64,
        to_oms: &Sender<OrderEvent>,
    ) {
        if outcome != RequestOutcome::Accepted {
            // TODO: Reconcile ambiguous submissions by querying Bybit with order_link_id.
            to_oms
                .send(OrderEvent::SubmissionFailed(order_link_id))
                .unwrap();
        }
    }

    async fn send_request(request: RequestBuilder, order_link_id: u64) -> RequestOutcome {
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => {
                return RequestOutcome::Unknown;
            }
        };

        if !response.status().is_success() {
            return RequestOutcome::Unknown;
        }

        let url = response.url().to_string();
        // NOTE: The current handling of zero requests left is very simple because HTTP
        // requests will be replaced by WebSocket orders and the test strategy will run
        // at low iteration rate to guarantee safety.
        if let Some(api_limit_status) = response
            .headers()
            .get("x-bapi-limit-status")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| u8::from_str(value).ok())
        {
            if api_limit_status == 0 {
                error!("Zero requests left for {url}");
            } else if api_limit_status <= 2 {
                warn!("Remaining {api_limit_status} requests for {url}");
            }
        }

        let raw_text = match response.text().await {
            Ok(raw_text) => raw_text,
            Err(_) => {
                return RequestOutcome::Unknown;
            }
        };

        let content = match serde_json::from_str::<CommonResponse>(&raw_text) {
            Ok(content) => content,
            Err(_) => {
                return RequestOutcome::Unknown;
            }
        };

        Self::classify_response(&content, &url, order_link_id)
    }
}

impl OrderGateway for OrderHandler {
    #[log_execution_time]
    fn submit_order(&self, order_builder: &OrderBuilder, order_link_id: u64) {
        // TODO: identify more efficient methods than `serde`
        // TODO: add support for all additional exchange non-mandatory parameters
        let url = format!("{}/v5/order/create", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        // TODO: add timeInForce parameter
        let body = Self::submit_order_body(order_builder, order_link_id);
        let signature = generate_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window,
            &body.to_string(),
            &self.api_secret,
        );
        let request = self
            .session
            .post(url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-TIMESTAMP", time_ms)
            .json(&body);
        let to_oms = self.to_oms.clone();
        tokio::spawn(async move {
            let start = std::time::Instant::now();

            let outcome = Self::send_request(request, order_link_id).await;
            Self::report_submission_outcome(outcome, order_link_id, &to_oms);

            let duration = start.elapsed();
            log::info!("Execution time of `send_request`: {:.2?}", duration);
        });
    }

    #[log_execution_time]
    fn amend_order(&self, order_builder: &OrderAmendedBuilder) {
        // TODO: identify more efficient methods than `serde`
        // TODO: add support for all additional exchange non-mandatory parameters
        let url = format!("{}/v5/order/amend", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        let body = Self::amend_order_body(order_builder);
        let signature = generate_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window,
            &body.to_string(),
            &self.api_secret,
        );
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

            // NOTE: `RequestOutcome` is not fed back to the OMS because if the
            // order change wasn't successful, it will be re-updated with the
            // next refresh of the order book.
            Self::send_request(request, order_link_id).await;

            let duration = start.elapsed();
            log::info!("Execution time of `send_request`: {:.2?}", duration);
        });
    }

    #[log_execution_time]
    fn repay_liability(&self, _coin: &str) {
        let url = format!("{}/v5/account/quick-repayment", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        let signature = generate_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window,
            &String::default(),
            &self.api_secret,
        );
        let request = self
            .session
            .post(url)
            .header("X-BAPI-SIGN", signature)
            .header("X-BAPI-TIMESTAMP", time_ms);
        // TODO: move from HTTP request to WebSocket
        // TODO: find a proper way to deal with failed orders
        tokio::spawn(async move {
            let start = std::time::Instant::now();

            // NOTE: error 999 is used because an order id is required, but there is no
            // order id for cancel-all. I am not using an Option type to reduce the
            // overhead.
            // NOTE: `RequestOutcome` is not fed back to the OMS
            Self::send_request(request, 999).await;

            let duration = start.elapsed();
            log::info!("Execution time of `send_request`: {:.2?}", duration);
        });
    }

    // TODO: introduce kill-switch when bot crashes or it's killed with ^c
    #[log_execution_time]
    fn cancel_all(&self) {
        // TODO: identify more efficient methods than `serde`
        // TODO: add support for all additional exchange non-mandatory parameters
        let url = format!("{}/v5/order/cancel-all", self.base_url);
        let time_ms = Utc::now().timestamp_millis().to_string();

        let body = Self::cancel_all_body();
        let signature = generate_signature(
            &time_ms,
            &self.api_key,
            &self.recv_window,
            &body.to_string(),
            &self.api_secret,
        );
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
            // NOTE: `RequestOutcome` is not fed back to the OMS
            Self::send_request(request, 999).await;

            let duration = start.elapsed();
            log::info!("Execution time of `cancel_all`: {:.2?}", duration);
        });
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::unbounded;
    use rstest::rstest;

    use super::*;
    use crate::{OrderSide, OrderType};

    fn create_common_response(ret_code: u32) -> CommonResponse<'static> {
        CommonResponse {
            ret_code,
            ret_msg: "response message",
            result: RawValue::from_string("{}".to_string()).unwrap(),
            ret_ext_info: RawValue::from_string("{}".to_string()).unwrap(),
            time: 1773956505537,
        }
    }

    fn create_order_builder() -> OrderBuilder {
        OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        }
    }

    fn create_amended_order_builder(new_qty: bool, new_price: bool) -> OrderAmendedBuilder {
        OrderAmendedBuilder {
            symbol: "ADAUSDT".to_string(),
            order_link_id: 1234,
            qty: 25.0,
            price: "0.567".to_string(),
            new_price,
            new_qty,
        }
    }

    #[test]
    fn order_response_deserializes_bybit_fields() {
        let response: OrderResponse = serde_json::from_str(
            r#"{
                "orderId": "exchange-order-id",
                "orderLinkId": "1234"
            }"#,
        )
        .unwrap();

        assert_eq!(
            response,
            OrderResponse {
                order_id: "exchange-order-id",
                order_link_id: "1234",
            }
        );
    }

    #[test]
    fn classify_response_accepts_success() {
        let response = create_common_response(0);

        let outcome = OrderHandler::classify_response(
            &response,
            "https://api.example.com/v5/order/create",
            1234,
        );

        assert_eq!(outcome, RequestOutcome::Accepted);
    }

    #[rstest]
    #[case(10001)]
    #[case(10002)]
    #[case(10000)]
    #[case(10016)]
    #[case(170193)]
    #[case(170194)]
    #[case(170213)]
    fn classify_response_rejects_known_error_codes(#[case] ret_code: u32) {
        let response = create_common_response(ret_code);

        let outcome = OrderHandler::classify_response(
            &response,
            "https://api.example.com/v5/order/create",
            1234,
        );

        assert_eq!(outcome, RequestOutcome::Rejected(ret_code));
    }

    #[test]
    #[should_panic(
        expected = "Failed https://api.example.com/v5/order/create request. Code: 99999"
    )]
    fn classify_response_panics_for_unknown_error_code() {
        let response = create_common_response(99999);

        OrderHandler::classify_response(&response, "https://api.example.com/v5/order/create", 1234);
    }

    #[test]
    fn accepted_submission_does_not_notify_oms() {
        let (to_oms, from_gateway) = unbounded();

        OrderHandler::report_submission_outcome(RequestOutcome::Accepted, 1234, &to_oms);

        assert!(from_gateway.is_empty());
    }

    #[rstest]
    #[case(RequestOutcome::Rejected(10001))]
    #[case(RequestOutcome::Unknown)]
    fn failed_submission_notifies_oms(#[case] outcome: RequestOutcome) {
        let (to_oms, from_gateway) = unbounded();

        OrderHandler::report_submission_outcome(outcome, 1234, &to_oms);

        assert_eq!(
            from_gateway.try_recv().unwrap(),
            OrderEvent::SubmissionFailed(1234)
        );
        assert!(from_gateway.is_empty());
    }

    #[test]
    fn submit_order_body_contains_bybit_parameters() {
        let order_builder = create_order_builder();

        let body = OrderHandler::submit_order_body(&order_builder, 1234);

        assert_eq!(
            body,
            json!({
                "orderLinkId": "1234",
                "category": "spot",
                "isLeverage": 1,
                "symbol": "ADAUSDT",
                "side": "Buy",
                "orderType": "Limit",
                "qty": "25",
                "price": "0.567",
                "timeInForce": "PostOnly",
                "smpType": "CancelBoth",
                "marketUnit": "baseCoin"
            })
        );
    }

    #[rstest]
    #[case(0.0)]
    #[case(-1.0)]
    #[case(f64::NAN)]
    #[case(f64::INFINITY)]
    #[case(f64::NEG_INFINITY)]
    #[should_panic(expected = "Order quantity must be finite and greater than zero.")]
    fn quantity_serialization_rejects_invalid_value(#[case] qty: f64) {
        OrderHandler::serialize_quantity(qty);
    }

    #[rstest]
    #[case("")]
    #[case("not-a-number")]
    #[case("0")]
    #[case("-0.1")]
    #[case("NaN")]
    #[case("inf")]
    #[should_panic(expected = "Order price must be a finite number greater than zero.")]
    fn price_serialization_rejects_invalid_value(#[case] price: &str) {
        OrderHandler::serialize_price(price);
    }

    #[rstest]
    #[case(false, false, None, None)]
    #[case(true, false, Some("25"), None)]
    #[case(false, true, None, Some("0.567"))]
    #[case(true, true, Some("25"), Some("0.567"))]
    fn amend_order_body_contains_only_changed_values(
        #[case] new_qty: bool,
        #[case] new_price: bool,
        #[case] expected_qty: Option<&str>,
        #[case] expected_price: Option<&str>,
    ) {
        let order_builder = create_amended_order_builder(new_qty, new_price);
        let mut expected_body = json!({
            "category": "spot",
            "symbol": "ADAUSDT",
            "orderLinkId": "1234",
        });
        if let Some(qty) = expected_qty {
            expected_body["qty"] = json!(qty);
        }
        if let Some(price) = expected_price {
            expected_body["price"] = json!(price);
        }

        let body = OrderHandler::amend_order_body(&order_builder);

        assert_eq!(body, expected_body);
    }

    #[test]
    fn cancel_all_body_contains_spot_category() {
        assert_eq!(
            OrderHandler::cancel_all_body(),
            json!({ "category": "spot" })
        );
    }
}
