mod runner;
mod simulated_exchange;

use assert_approx_eq::assert_approx_eq;
use exchange::{
    Level,
    Order,
    OrderBook,
    OrderBuilder,
    OrderEvent,
    OrderExecution,
    OrderSide,
    OrderStatus,
    OrderType,
};
use recorder::{CompletedMarkout, DataPoint};
use runner::OfflineRunner;

#[derive(Debug, PartialEq)]
struct OrderSnapshot {
    order_link_id: u64,
    order_status: OrderStatus,
    symbol: String,
    side: OrderSide,
    order_type: OrderType,
    qty: f64,
    price: f64,
    filled_qty: f64,
    filled_price: Option<f64>,
    updated_time: u64,
}

impl From<&Order> for OrderSnapshot {
    fn from(order: &Order) -> Self {
        Self {
            order_link_id: order.order_link_id,
            order_status: order.order_status,
            symbol: order.symbol.clone(),
            side: order.side,
            order_type: order.order_type,
            qty: order.qty,
            price: order.price,
            filled_qty: order.filled_qty,
            filled_price: (!order.filled_price.is_nan()).then_some(order.filled_price),
            updated_time: order.updated_time,
        }
    }
}

fn order_book(cts: u64, bid: (f64, f64), ask: (f64, f64)) -> OrderBook {
    OrderBook {
        bids: vec![Level {
            price: bid.0,
            size: bid.1,
        }],
        asks: vec![Level {
            price: ask.0,
            size: ask.1,
        }],
        ts: cts,
        cts,
    }
}

fn expected_order(side: OrderSide, price: &str) -> OrderBuilder {
    OrderBuilder {
        symbol: "ADAUSDT".to_string(),
        side,
        order_type: OrderType::Limit,
        qty: 100.0,
        price: price.to_string(),
    }
}

#[test]
fn predefined_market_sequence_produces_expected_trading_state() {
    let mut runner = OfflineRunner::new(vec![
        order_book(0, (0.6000, 12_000.0), (0.6004, 8000.0)), // Initial quotes
        order_book(500, (0.5997, 9000.0), (0.6000, 11_000.0)), // Buy fill
        order_book(1000, (0.5998, 11_000.0), (0.6002, 9000.0)),
        order_book(1500, (0.5999, 15_000.0), (0.6003, 5000.0)), // 1s markout
        order_book(2000, (0.5997, 8500.0), (0.6002, 11_500.0)),
        order_book(2500, (0.5998, 13_000.0), (0.6003, 7000.0)),
        order_book(3000, (0.5996, 7000.0), (0.6001, 13_000.0)),
        order_book(3500, (0.5997, 10_000.0), (0.6002, 10_000.0)),
        order_book(4000, (0.5999, 6000.0), (0.6003, 14_000.0)),
        order_book(4500, (0.6000, 9000.0), (0.6004, 11_000.0)),
        order_book(5000, (0.5998, 12_500.0), (0.6002, 7500.0)),
        order_book(5500, (0.5997, 5000.0), (0.6001, 15_000.0)), // 5s markout
        order_book(6000, (0.5998, 14_000.0), (0.6003, 6000.0)),
        order_book(6500, (0.5996, 8000.0), (0.6000, 12_000.0)),
        order_book(7000, (0.5997, 11_500.0), (0.6001, 8500.0)),
        order_book(7500, (0.5999, 10_000.0), (0.6004, 10_000.0)),
        order_book(8000, (0.5999, 7000.0), (0.6004, 13_000.0)),
        order_book(8500, (0.5998, 13_500.0), (0.6002, 6500.0)),
        order_book(9000, (0.5997, 9500.0), (0.6001, 10_500.0)),
        order_book(9500, (0.5999, 12_000.0), (0.6003, 8000.0)),
        order_book(10_000, (0.5998, 10_500.0), (0.6002, 9500.0)),
        order_book(10_500, (0.6000, 12_000.0), (0.6004, 8000.0)), // 10s markout
    ]);

    runner.run();

    // Verify that strategy execution follows the production 1 Hz cadence and uses
    // the inventory resulting from the fill.
    assert_eq!(
        runner
            .strategy_decisions
            .iter()
            .map(|decision| decision.cts)
            .collect::<Vec<_>>(),
        vec![
            0, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10_000
        ]
    );
    let post_fill_decision = runner
        .strategy_decisions
        .iter()
        .find(|decision| decision.cts == 1000)
        .expect("strategy must run after the fill");
    assert_approx_eq!(post_fill_decision.inventory, 99.9);
    assert_eq!(
        post_fill_decision.orders,
        vec![
            expected_order(OrderSide::Buy, "0.5997"),
            expected_order(OrderSide::Sell, "0.6001"),
        ]
    );

    // Verify the complete final OMS state after processing the market replay.
    assert_approx_eq!(runner.oms.inventory(), 99.9);
    assert_approx_eq!(runner.oms.average_entry_price(), 0.6000);
    assert_eq!(
        runner
            .oms
            .orders()
            .map(OrderSnapshot::from)
            .collect::<Vec<_>>(),
        vec![
            OrderSnapshot {
                order_link_id: 90,
                order_status: OrderStatus::Filled,
                symbol: "ADAUSDT".to_string(),
                side: OrderSide::Buy,
                order_type: OrderType::Limit,
                qty: 100.0,
                price: 0.6000,
                filled_qty: 100.0,
                filled_price: Some(0.6000),
                updated_time: 500,
            },
            OrderSnapshot {
                order_link_id: 91,
                order_status: OrderStatus::New,
                symbol: "ADAUSDT".to_string(),
                side: OrderSide::Sell,
                order_type: OrderType::Limit,
                qty: 100.0,
                price: 0.6001,
                filled_qty: 0.0,
                filled_price: None,
                updated_time: 10_000,
            },
            OrderSnapshot {
                order_link_id: 92,
                order_status: OrderStatus::New,
                symbol: "ADAUSDT".to_string(),
                side: OrderSide::Buy,
                order_type: OrderType::Limit,
                qty: 100.0,
                price: 0.5997,
                filled_qty: 0.0,
                filled_price: None,
                updated_time: 10_000,
            },
        ]
    );

    // Verify the complete sequence of exchange submissions and amendments, with
    // no rejected commands, liability repayments, or cancellations.
    let exchange_state = runner.simulated_exchange.state();
    assert_eq!(
        exchange_state
            .submitted_orders
            .iter()
            .map(|submission| (
                submission.cts,
                submission.order_link_id,
                submission.order.clone(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (0, 90, expected_order(OrderSide::Buy, "0.6000")),
            (0, 91, expected_order(OrderSide::Sell, "0.6004")),
            (1000, 92, expected_order(OrderSide::Buy, "0.5997")),
        ]
    );
    assert_eq!(
        exchange_state
            .amendment_requests
            .iter()
            .map(|request| (
                request.cts,
                request.amendment.order_link_id,
                request.amendment.price.as_str(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (1000, 91, "0.6001"),
            (3000, 92, "0.5996"),
            (3000, 91, "0.6000"),
            (4000, 92, "0.5999"),
            (4000, 91, "0.6003"),
            (5000, 92, "0.5997"),
            (5000, 91, "0.6001"),
            (7000, 92, "0.5996"),
            (7000, 91, "0.6000"),
            (8000, 92, "0.5999"),
            (8000, 91, "0.6003"),
            (9000, 92, "0.5996"),
            (9000, 91, "0.6000"),
            (10_000, 92, "0.5997"),
            (10_000, 91, "0.6001"),
        ]
    );
    assert!(
        exchange_state
            .amendment_requests
            .iter()
            .all(|request| request.amendment.new_price && !request.amendment.new_qty)
    );
    assert!(exchange_state.rejected_amendments.is_empty());
    assert!(exchange_state.repaid_coins.is_empty());
    assert_eq!(exchange_state.cancel_all_calls, 0);

    // Verify that the recorder emits the execution once all three markout horizons
    // have been populated from their respective books.
    assert_eq!(
        runner.completed_markouts,
        vec![(
            10_500,
            CompletedMarkout {
                exec_id: "sim-execution-1".to_string(),
                order_link_id: 90,
                fill_ts: 500,
                side: OrderSide::Buy,
                limit_price: 0.6000,
                exec_price: 0.6000,
                exec_qty: 100.0,
                mid_1s: DataPoint {
                    mid_price: 0.6001,
                    imbalance: 0.5,
                },
                mid_5s: DataPoint {
                    mid_price: 0.5999,
                    imbalance: -0.5,
                },
                mid_10s: DataPoint {
                    mid_price: (0.6000 + 0.6004) / 2.0,
                    imbalance: 0.2,
                },
            },
        )]
    );

    // Verify the execution payload, event ordering, and absence of submission
    // failures across the complete exchange-to-OMS trace.
    let executions: Vec<_> = runner
        .processed_events
        .iter()
        .filter_map(|event| match event {
            OrderEvent::ExecutionUpdate(execution) => Some(execution.clone()),
            OrderEvent::OrderUpdate(_) | OrderEvent::SubmissionFailed(_) => None,
        })
        .collect();
    assert_eq!(
        executions,
        vec![OrderExecution {
            order_link_id: 90,
            order_id: "sim-order-90".to_string(),
            order_price: 0.6000,
            order_side: OrderSide::Buy,
            exec_id: "sim-execution-1".to_string(),
            exec_ts: 500,
            exec_price: 0.6000,
            exec_fee: 100.0 * 0.001,
            exec_qty: 100.0,
            remaining_qty: 0.0,
        }]
    );
    assert!(matches!(
        &runner.processed_events[2],
        OrderEvent::ExecutionUpdate(_)
    ));
    assert!(matches!(
        &runner.processed_events[3],
        OrderEvent::OrderUpdate(update) if update.order_status == OrderStatus::Filled
    ));
    assert_eq!(runner.processed_events.len(), 20);
    assert!(
        runner
            .processed_events
            .iter()
            .all(|event| !matches!(event, OrderEvent::SubmissionFailed(_)))
    );
}
