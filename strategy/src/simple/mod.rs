use exchange::{InstrumentInfo, OrderBook, OrderBuilder, OrderSide, OrderType};
use log::{info, warn};

use crate::Strategy;

#[derive(Debug)]
pub struct SimpleStrategy {
    size: f64,
    instrument_info: InstrumentInfo,
}
impl SimpleStrategy {
    pub fn new(size: f64, instrument_info: InstrumentInfo) -> SimpleStrategy {
        assert!(
            size.is_finite() && size > 0.0,
            "Order size must be finite and greater than zero."
        );
        SimpleStrategy {
            size,
            instrument_info,
        }
    }

    fn compute_orders(&self, order_book: &OrderBook, inventory: f64) -> Vec<OrderBuilder> {
        const BASE_SPREAD: f64 = 2.0;
        const SKEW_FACTOR: f64 = 0.01;

        let first_bid = order_book.bids.first().unwrap();
        // let last_bid = order_book.bids.last().unwrap();
        let first_ask = order_book.asks.first().unwrap();
        // let last_ask = order_book.asks.last().unwrap();

        let decimal_digits = self.instrument_info.decimal_places();
        info!(
            "B {:.*} | A {:.*} | S {:.*}",
            decimal_digits,
            first_bid.price,
            decimal_digits,
            first_ask.price,
            decimal_digits,
            if first_bid.price != 0.0 && first_ask.price != 0.0 {
                first_ask.price - first_bid.price
            } else {
                0.0
            }
        );

        let precision = self.instrument_info.tick_size();
        // TODO: check if I should go to different levels for the mid price depending on
        // the volume available in the top level.
        let micro_price = (first_bid.price * first_bid.size + first_ask.price * first_ask.size)
            / (first_bid.size + first_ask.size);

        let price_shift = inventory * SKEW_FACTOR * precision;
        let reservation_price = micro_price - price_shift;

        let mut bid_price = reservation_price - (BASE_SPREAD * precision);
        let mut ask_price = reservation_price + (BASE_SPREAD * precision);

        if bid_price >= first_ask.price {
            bid_price = first_ask.price - precision;
        }

        if ask_price <= first_bid.price {
            ask_price = first_bid.price + precision;
        }

        // TODO: Optimise String cloning
        // TODO: Make batch order submission
        // TODO: Deal with channel send errors
        // TODO: Optimise use of vector to return orders to submit to oms
        let bid_order = OrderBuilder {
            symbol: self.instrument_info.symbol().to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: self.size,
            price: format!("{bid_price:.*}", decimal_digits),
        };

        let ask_order = OrderBuilder {
            symbol: self.instrument_info.symbol().to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            qty: self.size,
            price: format!("{ask_price:.*}", decimal_digits),
        };

        vec![bid_order, ask_order]
    }
}

impl Strategy for SimpleStrategy {
    fn execute(&mut self, order_book: &OrderBook, inventory: f64) -> Vec<OrderBuilder> {
        assert!(inventory.is_finite(), "Inventory must be finite.");

        let (Some(first_bid), Some(first_ask)) = (order_book.bids.first(), order_book.asks.first())
        else {
            warn!("Empty book");
            return Vec::new();
        };

        if !first_bid.price.is_finite()
            || first_bid.price <= 0.0
            || !first_bid.size.is_finite()
            || first_bid.size <= 0.0
            || !first_ask.price.is_finite()
            || first_ask.price <= 0.0
            || !first_ask.size.is_finite()
            || first_ask.size <= 0.0
        {
            warn!("Invalid top-of-book values");
            return Vec::new();
        }

        self.compute_orders(order_book, inventory)
    }
}

#[cfg(test)]
mod tests {
    use exchange::Level;

    use super::*;

    fn instrument_info() -> InstrumentInfo {
        InstrumentInfo::new(
            "ADAUSDT".to_string(),
            "ADA".to_string(),
            "USDT".to_string(),
            0.01,
            0.000001,
            0.001,
            3,
        )
    }

    #[test]
    fn execute_returns_orders_using_current_inventory() {
        let mut strategy = SimpleStrategy::new(25.0, instrument_info());
        let order_book = OrderBook {
            bids: vec![Level {
                price: 0.499,
                size: 1000.0,
            }],
            asks: vec![Level {
                price: 0.501,
                size: 1000.0,
            }],
            ..OrderBook::default()
        };

        let orders = strategy.execute(&order_book, 100.0);

        assert_eq!(
            orders,
            vec![
                OrderBuilder {
                    symbol: "ADAUSDT".to_string(),
                    side: OrderSide::Buy,
                    order_type: OrderType::Limit,
                    qty: 25.0,
                    price: "0.497".to_string(),
                },
                OrderBuilder {
                    symbol: "ADAUSDT".to_string(),
                    side: OrderSide::Sell,
                    order_type: OrderType::Limit,
                    qty: 25.0,
                    price: "0.501".to_string(),
                },
            ]
        );
    }

    #[test]
    fn execute_returns_no_orders_for_empty_book() {
        let mut strategy = SimpleStrategy::new(25.0, instrument_info());

        assert!(strategy.execute(&OrderBook::default(), 0.0).is_empty());
    }

    #[test]
    fn execute_returns_no_orders_for_invalid_top_of_book() {
        let mut strategy = SimpleStrategy::new(25.0, instrument_info());
        let order_book = OrderBook {
            bids: vec![Level {
                price: 0.499,
                size: 0.0,
            }],
            asks: vec![Level {
                price: 0.501,
                size: 1000.0,
            }],
            ..OrderBook::default()
        };

        assert!(strategy.execute(&order_book, 0.0).is_empty());
    }

    #[test]
    #[should_panic(expected = "Inventory must be finite.")]
    fn execute_rejects_invalid_inventory() {
        let mut strategy = SimpleStrategy::new(25.0, instrument_info());

        strategy.execute(&OrderBook::default(), f64::NAN);
    }

    #[test]
    #[should_panic(expected = "Order size must be finite and greater than zero.")]
    fn new_rejects_invalid_order_size() {
        SimpleStrategy::new(f64::NAN, instrument_info());
    }
}
