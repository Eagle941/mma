use bybit::WebSocketApiClient;
use bybit::ws::private::PrivateWebsocketApiClient;
use bybit::ws::response::PrivateResponse;
use configuration::AppConfigProvider;
use crossbeam_channel::Sender;
use log::warn;

use crate::OrderEvent;

#[derive(Debug)]
pub struct PrivateWebSocket {
    testnet: bool,
    api_key: String,
    api_secret: String,
    to_oms: Sender<OrderEvent>,
    to_recorder: Sender<OrderEvent>,
}
impl PrivateWebSocket {
    pub fn new(
        to_oms: Sender<OrderEvent>,
        to_recorder: Sender<OrderEvent>,
        config: &dyn AppConfigProvider,
    ) -> Self {
        PrivateWebSocket {
            testnet: config.testnet(),
            to_oms,
            to_recorder,
            api_key: config.api_key().to_string(),
            api_secret: config.api_secret().to_string(),
        }
    }

    fn get_ws_client(&self) -> PrivateWebsocketApiClient {
        if self.testnet {
            return WebSocketApiClient::private()
                .testnet()
                .build_with_credentials(&self.api_key, &self.api_secret);
        }
        WebSocketApiClient::private().build_with_credentials(&self.api_key, &self.api_secret)
    }

    fn process_response(&self, response: PrivateResponse) {
        match response {
            PrivateResponse::Order(res) => {
                let data = res.data;
                for order in data {
                    if order.order_link_id.is_empty() {
                        continue;
                    }
                    self.to_oms.send((&order).into()).unwrap();
                }
            }
            PrivateResponse::Execution(res) => {
                let data = res.data;
                for order in data {
                    if order.order_link_id.is_empty() {
                        continue;
                    }
                    self.to_oms.send((&order).into()).unwrap();
                    self.to_recorder.send((&order).into()).unwrap();
                }
            }
            PrivateResponse::Op(res) => {
                if !res.success {
                    warn!("{res:?}");
                }
            }
            PrivateResponse::Pong(_) => (),
            response => warn!("PrivateResponse::{response:?} not implemented"),
        }
    }

    pub fn subscribe(&self) {
        let mut client = self.get_ws_client();
        client.subscribe_order();
        client.subscribe_execution();

        // TODO: Add subscription to Wallet stream.
        let callback = |response: PrivateResponse| self.process_response(response);

        match client.run(callback) {
            Ok(()) => {}
            Err(e) => eprintln!("{e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use bybit::ws::response::{
        BasePrivateResponse,
        Execution,
        OpResponse,
        Order as BybitOrder,
        PrivatePongResponse,
    };
    use crossbeam_channel::{Receiver, unbounded};
    use rstest::rstest;

    use super::*;

    struct PrivateWebSocketTestBench {
        websocket: PrivateWebSocket,
        from_oms: Receiver<OrderEvent>,
        from_recorder: Receiver<OrderEvent>,
    }
    impl PrivateWebSocketTestBench {
        fn new() -> Self {
            let (to_oms, from_oms) = unbounded();
            let (to_recorder, from_recorder) = unbounded();
            let websocket = PrivateWebSocket {
                testnet: true,
                api_key: "api-key".to_string(),
                api_secret: "api-secret".to_string(),
                to_oms,
                to_recorder,
            };

            Self {
                websocket,
                from_oms,
                from_recorder,
            }
        }

        fn assert_channels_are_empty(&self) {
            assert!(self.from_oms.is_empty());
            assert!(self.from_recorder.is_empty());
        }
    }

    fn create_order(order_link_id: &str) -> BybitOrder<'_> {
        BybitOrder {
            category: "spot",
            order_id: "exchange-order-id",
            order_link_id,
            is_leverage: "0",
            block_trade_id: "",
            symbol: "ADAUSDT",
            price: "0.567",
            qty: "25.0",
            side: "Buy",
            position_idx: 0,
            order_status: "New",
            cancel_type: "",
            reject_reason: "",
            avg_price: "0.566",
            leaves_qty: "15.0",
            leaves_value: "",
            cum_exec_qty: "10.0",
            cum_exec_value: "",
            cum_exec_fee: "",
            time_in_force: "PostOnly",
            order_type: "Limit",
            stop_order_type: "",
            order_iv: "",
            trigger_price: "",
            take_profit: "",
            stop_loss: "",
            tp_trigger_by: "",
            sl_trigger_by: "",
            trigger_direction: 0,
            trigger_by: "",
            last_price_on_created: "",
            reduce_only: false,
            close_on_trigger: false,
            created_time: "1773956505000",
            updated_time: "1773956505537",
        }
    }

    fn create_execution(order_link_id: &str) -> Execution<'_> {
        Execution {
            category: "spot",
            symbol: "ADAUSDT",
            is_leverage: "0",
            order_id: "exchange-order-id",
            order_link_id,
            side: "Sell",
            order_price: "0.567",
            order_qty: "25.0",
            leaves_qty: "15.0",
            order_type: "Limit",
            stop_order_type: "",
            exec_fee: "0.01",
            exec_id: "execution-id",
            exec_price: "0.566",
            exec_qty: "10.0",
            exec_type: "Trade",
            exec_value: "5.66",
            exec_time: "1773956505537",
            is_maker: true,
            fee_rate: "0.001",
            trade_iv: "",
            mark_iv: "",
            mark_price: "",
            index_price: "",
            underlying_price: "",
            block_trade_id: "",
        }
    }

    #[test]
    fn order_response_forwards_linked_orders_only_to_oms() {
        let test_bench = PrivateWebSocketTestBench::new();
        let order = create_order("1234");
        let empty_order_id = create_order("");
        let expected_event = OrderEvent::from(&order);
        let response = PrivateResponse::Order(BasePrivateResponse {
            id: "message-id",
            topic: "order",
            creation_time: 1_773_956_505_537,
            data: vec![empty_order_id, order],
        });

        test_bench.websocket.process_response(response);

        assert_eq!(test_bench.from_oms.try_recv().unwrap(), expected_event);
        test_bench.assert_channels_are_empty();
    }

    #[test]
    fn execution_response_forwards_linked_executions_to_oms_and_recorder() {
        let test_bench = PrivateWebSocketTestBench::new();
        let execution = create_execution("1234");
        let empty_execution = create_execution("");
        let expected_event = OrderEvent::from(&execution);
        let response = PrivateResponse::Execution(BasePrivateResponse {
            id: "message-id",
            topic: "execution",
            creation_time: 1_773_956_505_537,
            data: vec![empty_execution, execution],
        });

        test_bench.websocket.process_response(response);

        assert_eq!(
            test_bench.from_oms.try_recv().unwrap(),
            expected_event.clone()
        );
        assert_eq!(test_bench.from_recorder.try_recv().unwrap(), expected_event);
        test_bench.assert_channels_are_empty();
    }

    #[rstest]
    #[case(true)]
    #[case(false)]
    fn operation_response_does_not_emit_order_events(#[case] success: bool) {
        let test_bench = PrivateWebSocketTestBench::new();
        let response = PrivateResponse::Op(OpResponse {
            success,
            ret_msg: "",
            conn_id: "connection-id",
            req_id: Some("request-id"),
            op: "subscribe",
        });

        test_bench.websocket.process_response(response);

        test_bench.assert_channels_are_empty();
    }

    #[test]
    fn pong_response_does_not_emit_order_events() {
        let test_bench = PrivateWebSocketTestBench::new();
        let response = PrivateResponse::Pong(PrivatePongResponse {
            req_id: Some("request-id"),
            op: "pong",
            args: ["1773956505537"],
            conn_id: "connection-id",
        });

        test_bench.websocket.process_response(response);

        test_bench.assert_channels_are_empty();
    }
}
