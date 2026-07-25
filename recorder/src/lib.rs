use std::collections::HashMap;
use std::sync::Arc;
use std::{f64, fmt};

use crossbeam_channel::Receiver;
use crossbeam_queue::ArrayQueue;
use exchange::{OrderBook, OrderExecution, OrderMessages, OrderSide};
use log::info;

#[derive(Copy, Clone, Debug)]
pub struct DataPoint {
    mid_price: f64,
    imbalance: f64,
}
impl Default for DataPoint {
    fn default() -> Self {
        DataPoint {
            mid_price: f64::NAN,
            imbalance: f64::NAN,
        }
    }
}
impl fmt::Display for DataPoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:.5} {:.5}", self.mid_price, self.imbalance)
    }
}

#[derive(Copy, Clone, Debug)]
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
            "{} {} {} {:.5} {:.5} {:.5} {} {} {}",
            self.order_link_id,
            self.fill_ts,
            self.side,
            self.limit_price,
            self.exec_price,
            self.exec_qty,
            self.mid_1s.unwrap_or_default(),
            self.mid_5s.unwrap_or_default(),
            self.mid_10s.unwrap_or_default()
        )
    }
}

#[derive(Clone)]
pub struct MarkoutEngine {
    from_book: Arc<ArrayQueue<OrderBook>>,
    from_execution: Receiver<OrderMessages>,
    trades: HashMap<String, PendingMarkout>, // key is execId
}
impl MarkoutEngine {
    pub fn new(
        from_book: Arc<ArrayQueue<OrderBook>>,
        from_execution: Receiver<OrderMessages>,
    ) -> Self {
        MarkoutEngine {
            from_book,
            from_execution,
            trades: HashMap::new(),
        }
    }

    pub fn cycle(&mut self) {
        loop {
            if let Some(order_book) = self.from_book.pop() {
                self.update_prices(order_book);
                self.log_and_remove();
            }
            // TODO: remove select! because there is only one channel.
            crossbeam_channel::select! {
                recv(self.from_execution) -> msg => {
                    if let Ok(new_execution) = msg {
                        match new_execution {
                            OrderMessages::ExecutionUpdate(order_execution) => self.update_trades(order_execution),
                            OrderMessages::OrderUpdate(_) => (),
                        }
                    }
                }
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

    pub fn log_and_remove(&mut self) {
        for (id, t) in self
            .trades
            .extract_if(|_, t| t.mid_1s.and(t.mid_5s).and(t.mid_10s).is_some())
        {
            info!("ExecId {id} | {t}");
        }
    }
}
