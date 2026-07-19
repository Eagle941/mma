use std::str::FromStr;

use exchange::{Order, OrderAmendedBuilder, OrderBuilder, OrderSide};
use slab::Slab;

#[derive(Debug, PartialEq)]
pub enum Outcome {
    NewOrder(OrderBuilder),
    AmendOrder(OrderAmendedBuilder),
    DoNothing,
}

pub trait RiskPolicy {
    fn evaluate_order(
        &self,
        orders: &Slab<Order>,
        new_order: OrderBuilder,
        inventory: f64,
        average_entry_price: f64,
    ) -> Outcome;
}

// NOTE: could be dynamic
const MAX_INVENTORY: f64 = 500.0; // Quantity
const MIN_INVENTORY: f64 = -500.0; // Quantity

// TODO: This file may be moved to a dedicated library
#[derive(Default)]
pub struct RiskManager;
impl RiskManager {
    fn get_existing_order(orders: &Slab<Order>, side: OrderSide) -> Option<(usize, &Order)> {
        orders
            .iter()
            .find(|(_, o)| o.order_status.is_open() && side == o.side)
    }
}
impl RiskPolicy for RiskManager {
    fn evaluate_order(
        &self,
        orders: &Slab<Order>,
        new_order: OrderBuilder,
        inventory: f64,
        _average_entry_price: f64,
    ) -> Outcome {
        if inventory >= MAX_INVENTORY && new_order.side == OrderSide::Buy {
            return Outcome::DoNothing;
        }

        if inventory <= MIN_INVENTORY && new_order.side == OrderSide::Sell {
            return Outcome::DoNothing;
        }

        let new_order_price = f64::from_str(new_order.price.as_str()).unwrap();

        // This is a very simple risk management. Don't have more than two orders
        // running at the same time.
        // NOTE: Assumption is that there is only one active order per side at a time!
        let Some((_, existing_order)) = RiskManager::get_existing_order(orders, new_order.side)
        else {
            return Outcome::NewOrder(new_order);
        };
        if existing_order.order_status == exchange::OrderStatus::Submitted {
            return Outcome::DoNothing;
        }

        let amended_order = OrderAmendedBuilder {
            symbol: new_order.symbol,
            order_link_id: existing_order.order_link_id,
            qty: new_order.qty,
            price: new_order.price.clone(),
            // TODO: is it more efficient to compare String instead of f64?
            new_price: new_order_price != existing_order.price,
            new_qty: new_order.qty != existing_order.qty,
        };

        if !amended_order.new_price && !amended_order.new_qty {
            return Outcome::DoNothing;
        }

        Outcome::AmendOrder(amended_order)
    }
}

#[cfg(test)]
mod tests {
    use exchange::OrderType;

    use super::*;

    #[test]
    fn submitted_order_blocks_another_same_side_submission() {
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let mut orders = Slab::new();
        orders.insert(order_builder.build(1000));

        let changed_order = OrderBuilder {
            price: "0.568".to_string(),
            ..order_builder
        };
        let outcome = RiskManager.evaluate_order(&orders, changed_order, 0.0, 0.0);

        assert_eq!(outcome, Outcome::DoNothing);
    }
}
