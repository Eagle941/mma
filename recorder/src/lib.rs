use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::{f64, fmt};

use crossbeam_channel::{Receiver, RecvTimeoutError};
use crossbeam_queue::ArrayQueue;
use exchange::{OrderBook, OrderEvent, OrderExecution, OrderSide};
use log::info;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DataPoint {
    pub mid_price: f64,
    pub imbalance: f64,
}
impl fmt::Display for DataPoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:.5} {:.5}", self.mid_price, self.imbalance)
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PendingMarkout {
    pub order_link_id: u64,
    pub fill_ts: u64, // ms
    pub side: OrderSide,
    pub limit_price: f64,
    pub exec_price: f64,
    pub exec_qty: f64,
    pub mid_1s: Option<DataPoint>,
    pub mid_5s: Option<DataPoint>,
    pub mid_10s: Option<DataPoint>,
}
impl fmt::Display for PendingMarkout {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {:.5} {:.5} {:.5}",
            self.order_link_id,
            self.fill_ts,
            self.side,
            self.limit_price,
            self.exec_price,
            self.exec_qty,
        )?;

        for markout in [self.mid_1s, self.mid_5s, self.mid_10s] {
            match markout {
                Some(data_point) => write!(f, " {data_point}")?,
                None => write!(f, " NA")?,
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompletedMarkout {
    pub exec_id: String,
    pub order_link_id: u64,
    pub fill_ts: u64,
    pub side: OrderSide,
    pub limit_price: f64,
    pub exec_price: f64,
    pub exec_qty: f64,
    pub mid_1s: DataPoint,
    pub mid_5s: DataPoint,
    pub mid_10s: DataPoint,
}
impl From<(String, PendingMarkout)> for CompletedMarkout {
    fn from((exec_id, pending): (String, PendingMarkout)) -> Self {
        Self {
            exec_id,
            order_link_id: pending.order_link_id,
            fill_ts: pending.fill_ts,
            side: pending.side,
            limit_price: pending.limit_price,
            exec_price: pending.exec_price,
            exec_qty: pending.exec_qty,
            mid_1s: pending
                .mid_1s
                .expect("completed markout must have a 1s value"),
            mid_5s: pending
                .mid_5s
                .expect("completed markout must have a 5s value"),
            mid_10s: pending
                .mid_10s
                .expect("completed markout must have a 10s value"),
        }
    }
}
impl fmt::Display for CompletedMarkout {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {:.5} {:.5} {:.5} {} {} {}",
            self.order_link_id,
            self.fill_ts,
            self.side,
            self.limit_price,
            self.exec_price,
            self.exec_qty,
            self.mid_1s,
            self.mid_5s,
            self.mid_10s,
        )
    }
}

#[derive(Debug)]
pub struct MarkoutEngine {
    // TODO: Replace `ArrayQueue` with a latest-value data structure that notifies
    // the consumer when a new order book is available, eliminating polling.
    from_book: Arc<ArrayQueue<OrderBook>>,
    order_events_rx: Receiver<OrderEvent>,
    trades: HashMap<String, PendingMarkout>, // key is execId
}
impl MarkoutEngine {
    const ORDER_BOOK_POLL_INTERVAL: Duration = Duration::from_micros(500);

    pub fn new(
        from_book: Arc<ArrayQueue<OrderBook>>,
        order_events_rx: Receiver<OrderEvent>,
    ) -> Self {
        MarkoutEngine {
            from_book,
            order_events_rx,
            trades: HashMap::new(),
        }
    }

    pub fn cycle(&mut self) {
        loop {
            if let Some(order_book) = self.from_book.pop() {
                for markout in self.process_order_book(order_book) {
                    info!("ExecId {} | {markout}", markout.exec_id);
                }
            }

            match self
                .order_events_rx
                .recv_timeout(Self::ORDER_BOOK_POLL_INTERVAL)
            {
                Ok(OrderEvent::ExecutionUpdate(execution)) => self.update_trades(execution),
                Ok(OrderEvent::OrderUpdate(_) | OrderEvent::SubmissionFailed(_)) => (),
                Err(RecvTimeoutError::Timeout) => (),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    pub fn update_trades(&mut self, execution: OrderExecution) {
        let markout = PendingMarkout {
            order_link_id: execution.order_link_id,
            fill_ts: execution.exec_ts,
            side: execution.order_side,
            limit_price: execution.order_price,
            exec_price: execution.exec_price,
            exec_qty: execution.exec_qty,
            mid_1s: None,
            mid_5s: None,
            mid_10s: None,
        };
        self.trades.insert(execution.exec_id, markout);
    }

    pub fn process_order_book(&mut self, order_book: OrderBook) -> Vec<CompletedMarkout> {
        self.update_prices(order_book);
        self.take_completed_markouts()
    }

    pub fn update_prices(&mut self, order_book: OrderBook) {
        let first_bid = order_book.bids.first().unwrap();
        let first_ask = order_book.asks.first().unwrap();

        let mid_price = (first_bid.price + first_ask.price) / 2.0;
        let imbalance = (first_bid.size - first_ask.size) / (first_bid.size + first_ask.size);
        let data_point = DataPoint {
            mid_price,
            imbalance,
        };

        for (_, t) in self.trades.iter_mut() {
            match t.mid_1s {
                None if t.fill_ts + 1000 <= order_book.cts => {
                    t.mid_1s = Some(data_point);
                }
                _ => (),
            }
            match t.mid_5s {
                None if t.fill_ts + 5000 <= order_book.cts => {
                    t.mid_5s = Some(data_point);
                }
                _ => (),
            }
            match t.mid_10s {
                None if t.fill_ts + 10000 <= order_book.cts => {
                    t.mid_10s = Some(data_point);
                }
                _ => (),
            }
        }
    }

    fn take_completed_markouts(&mut self) -> Vec<CompletedMarkout> {
        let mut completed: Vec<_> = self
            .trades
            .extract_if(|_, markout| {
                markout.mid_1s.is_some() && markout.mid_5s.is_some() && markout.mid_10s.is_some()
            })
            .collect();
        completed.sort_by(|(left_exec_id, left), (right_exec_id, right)| {
            left.fill_ts
                .cmp(&right.fill_ts)
                .then_with(|| left.order_link_id.cmp(&right.order_link_id))
                .then_with(|| left_exec_id.cmp(right_exec_id))
        });
        completed.into_iter().map(CompletedMarkout::from).collect()
    }
}

#[cfg(test)]
mod tests {

    use crossbeam_channel::unbounded;
    use exchange::Level;

    use super::*;

    #[test]
    fn data_point_display_use_log_placeholders_and_precision() {
        let data_point = DataPoint {
            mid_price: 100.123_456,
            imbalance: 0.123_456,
        };
        assert_eq!(data_point.to_string(), "100.12346 0.12346");
    }

    #[test]
    fn pending_markout_display_formats_trade_and_markout_data() {
        let pending_markout = PendingMarkout {
            order_link_id: 1000,
            fill_ts: 10_000,
            side: OrderSide::Buy,
            limit_price: 100.0,
            exec_price: 99.5,
            exec_qty: 2.0,
            mid_1s: Some(DataPoint {
                mid_price: 100.25,
                imbalance: 0.5,
            }),
            mid_5s: Some(DataPoint {
                mid_price: 100.26,
                imbalance: 0.6,
            }),
            mid_10s: Some(DataPoint {
                mid_price: 100.27,
                imbalance: 0.7,
            }),
        };

        assert_eq!(
            pending_markout.to_string(),
            "1000 10000 Buy 100.00000 99.50000 2.00000 100.25000 0.50000 100.26000 0.60000 \
             100.27000 0.70000"
        );
    }

    #[test]
    fn update_trades_stores_execution_as_pending_markout() {
        let order_books = Arc::new(ArrayQueue::new(1));
        let (_events_tx, events_rx) = unbounded();
        let mut recorder = MarkoutEngine::new(order_books, events_rx);
        let execution = OrderExecution {
            order_link_id: 1000,
            order_id: "exchange-order-id".to_string(),
            order_price: 0.567,
            order_side: OrderSide::Buy,
            exec_id: "execution-id".to_string(),
            exec_ts: 10_000,
            exec_price: 0.566,
            exec_fee: 0.01,
            exec_qty: 25.0,
            remaining_qty: 5.0,
        };

        recorder.update_trades(execution.clone());

        let expected_pending_markout = PendingMarkout {
            order_link_id: execution.order_link_id,
            fill_ts: execution.exec_ts,
            side: execution.order_side,
            limit_price: execution.order_price,
            exec_price: execution.exec_price,
            exec_qty: execution.exec_qty,
            mid_1s: None,
            mid_5s: None,
            mid_10s: None,
        };
        assert_eq!(
            recorder.trades.get("execution-id").unwrap(),
            &expected_pending_markout
        );
    }

    #[test]
    fn update_prices_records_each_markout_at_its_time_threshold() {
        let order_books = Arc::new(ArrayQueue::new(1));
        let (_events_tx, events_rx) = unbounded();
        let mut recorder = MarkoutEngine::new(order_books, events_rx);
        recorder.trades.insert(
            "execution-id".to_string(),
            PendingMarkout {
                order_link_id: 1000,
                fill_ts: 10_000,
                side: OrderSide::Buy,
                limit_price: 100.0,
                exec_price: 100.0,
                exec_qty: 1.0,
                mid_1s: None,
                mid_5s: None,
                mid_10s: None,
            },
        );

        recorder.update_prices(OrderBook {
            bids: vec![Level {
                price: 99.0,
                size: 3.0,
            }],
            asks: vec![Level {
                price: 101.0,
                size: 1.0,
            }],
            cts: 10_999,
            ..OrderBook::default()
        });
        assert_eq!(recorder.trades.get("execution-id").unwrap().mid_1s, None);

        recorder.update_prices(OrderBook {
            bids: vec![Level {
                price: 99.0,
                size: 3.0,
            }],
            asks: vec![Level {
                price: 101.0,
                size: 1.0,
            }],
            cts: 11_000,
            ..OrderBook::default()
        });
        assert_eq!(
            recorder.trades.get("execution-id").unwrap().mid_1s,
            Some(DataPoint {
                mid_price: 100.0,
                imbalance: 0.5,
            })
        );

        recorder.update_prices(OrderBook {
            bids: vec![Level {
                price: 101.0,
                size: 1.0,
            }],
            asks: vec![Level {
                price: 103.0,
                size: 3.0,
            }],
            cts: 15_000,
            ..OrderBook::default()
        });
        let pending_markout = recorder.trades.get("execution-id").unwrap();
        assert_eq!(
            pending_markout.mid_1s,
            Some(DataPoint {
                mid_price: 100.0,
                imbalance: 0.5,
            })
        );
        assert_eq!(
            pending_markout.mid_5s,
            Some(DataPoint {
                mid_price: 102.0,
                imbalance: -0.5,
            })
        );

        recorder.update_prices(OrderBook {
            bids: vec![Level {
                price: 103.0,
                size: 2.0,
            }],
            asks: vec![Level {
                price: 105.0,
                size: 2.0,
            }],
            cts: 20_000,
            ..OrderBook::default()
        });
        assert_eq!(
            recorder.trades.get("execution-id"),
            Some(&PendingMarkout {
                order_link_id: 1000,
                fill_ts: 10_000,
                side: OrderSide::Buy,
                limit_price: 100.0,
                exec_price: 100.0,
                exec_qty: 1.0,
                mid_1s: Some(DataPoint {
                    mid_price: 100.0,
                    imbalance: 0.5,
                }),
                mid_5s: Some(DataPoint {
                    mid_price: 102.0,
                    imbalance: -0.5,
                }),
                mid_10s: Some(DataPoint {
                    mid_price: 104.0,
                    imbalance: 0.0,
                }),
            })
        );
    }

    #[test]
    fn late_order_book_populates_all_elapsed_markouts() {
        let order_books = Arc::new(ArrayQueue::new(1));
        let (_events_tx, events_rx) = unbounded();
        let mut recorder = MarkoutEngine::new(order_books, events_rx);
        recorder.trades.insert(
            "execution-id".to_string(),
            PendingMarkout {
                order_link_id: 1000,
                fill_ts: 10_000,
                side: OrderSide::Sell,
                limit_price: 100.0,
                exec_price: 100.0,
                exec_qty: 1.0,
                mid_1s: None,
                mid_5s: None,
                mid_10s: None,
            },
        );
        let expected_data_point = DataPoint {
            mid_price: 100.0,
            imbalance: 0.5,
        };

        recorder.update_prices(OrderBook {
            bids: vec![Level {
                price: 99.0,
                size: 3.0,
            }],
            asks: vec![Level {
                price: 101.0,
                size: 1.0,
            }],
            cts: 20_000,
            ..OrderBook::default()
        });

        let pending_markout = recorder.trades.get("execution-id").unwrap();
        assert_eq!(pending_markout.mid_1s, Some(expected_data_point));
        assert_eq!(pending_markout.mid_5s, Some(expected_data_point));
        assert_eq!(pending_markout.mid_10s, Some(expected_data_point));
    }

    #[test]
    fn take_completed_markouts_returns_once_in_deterministic_order() {
        let order_books = Arc::new(ArrayQueue::new(1));
        let (_events_tx, events_rx) = unbounded();
        let mut recorder = MarkoutEngine::new(order_books, events_rx);
        let data_point = DataPoint {
            mid_price: 100.0,
            imbalance: 0.5,
        };
        recorder.trades.insert(
            "earlier-execution".to_string(),
            PendingMarkout {
                order_link_id: 999,
                fill_ts: 9_000,
                side: OrderSide::Sell,
                limit_price: 99.0,
                exec_price: 99.0,
                exec_qty: 0.5,
                mid_1s: Some(data_point),
                mid_5s: Some(data_point),
                mid_10s: Some(data_point),
            },
        );
        recorder.trades.insert(
            "complete-execution".to_string(),
            PendingMarkout {
                order_link_id: 1000,
                fill_ts: 10_000,
                side: OrderSide::Buy,
                limit_price: 100.0,
                exec_price: 100.0,
                exec_qty: 1.0,
                mid_1s: Some(data_point),
                mid_5s: Some(data_point),
                mid_10s: Some(data_point),
            },
        );
        recorder.trades.insert(
            "pending-execution".to_string(),
            PendingMarkout {
                order_link_id: 1001,
                fill_ts: 11_000,
                side: OrderSide::Sell,
                limit_price: 101.0,
                exec_price: 101.0,
                exec_qty: 2.0,
                mid_1s: Some(data_point),
                mid_5s: Some(data_point),
                mid_10s: None,
            },
        );

        let completed = recorder.take_completed_markouts();

        assert!(recorder.take_completed_markouts().is_empty());
        assert_eq!(recorder.trades.len(), 1);
        assert!(recorder.trades.contains_key("pending-execution"));
        assert_eq!(
            completed,
            vec![
                CompletedMarkout {
                    exec_id: "earlier-execution".to_string(),
                    order_link_id: 999,
                    fill_ts: 9_000,
                    side: OrderSide::Sell,
                    limit_price: 99.0,
                    exec_price: 99.0,
                    exec_qty: 0.5,
                    mid_1s: data_point,
                    mid_5s: data_point,
                    mid_10s: data_point,
                },
                CompletedMarkout {
                    exec_id: "complete-execution".to_string(),
                    order_link_id: 1000,
                    fill_ts: 10_000,
                    side: OrderSide::Buy,
                    limit_price: 100.0,
                    exec_price: 100.0,
                    exec_qty: 1.0,
                    mid_1s: data_point,
                    mid_5s: data_point,
                    mid_10s: data_point,
                },
            ]
        );
    }
}
