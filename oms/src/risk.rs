use std::collections::HashMap;
use std::str::FromStr;

use exchange::{Order, OrderAmendedBuilder, OrderBuilder, OrderSide};

pub enum Outcome {
    NewOrder(OrderBuilder),
    AmendOrder(OrderAmendedBuilder),
    Nothing,
}

// TODO: This file may be moved to a dedicated library
pub struct RiskManager();
impl RiskManager {
    pub fn submit_order(
        orders_history: &HashMap<String, Order>,
        new_order: OrderBuilder,
        inventory: f64,
    ) -> Outcome {
        const MAX_INVENTORY: f64 = 3000.0;
        const MIN_INVENTORY: f64 = -1800.0;

        if inventory > MAX_INVENTORY && new_order.side == OrderSide::Buy {
            return Outcome::Nothing;
        }

        if inventory < MIN_INVENTORY && new_order.side == OrderSide::Sell {
            return Outcome::Nothing;
        }

        // This is a very simple risk management. Don't have more than two orders
        // running at the same time.
        // NOTE: Assumption is that there is only one active order per side at a time!
        let Some((_, existing_order)) = orders_history
            .iter()
            .filter(|(_, o)| o.order_status.is_open() && new_order.side == o.side)
            .next()
        else {
            return Outcome::NewOrder(new_order);
        };

        let amended_order = OrderAmendedBuilder {
            symbol: new_order.symbol,
            order_id: existing_order.order_id.clone(),
            qty: new_order.qty,
            price: new_order.price.clone(),
            // TODO: is it more efficient to compare String instead of f64?
            new_price: f64::from_str(new_order.price.as_str()).unwrap() != existing_order.price,
            new_qty: new_order.qty != existing_order.qty,
        };
        if !amended_order.new_price && !amended_order.new_qty {
            return Outcome::Nothing;
        }
        return Outcome::AmendOrder(amended_order);
    }
}
