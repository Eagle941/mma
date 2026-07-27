use std::str::FromStr;

use exchange::{Order, OrderAmendedBuilder, OrderBuilder, OrderSide};
use slab::Slab;

#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
        // NOTE: These limits use current inventory only. They do not account for the
        // quantity of this order or other open orders that may still execute.
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
    use exchange::{OrderStatus, OrderType};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(MAX_INVENTORY, OrderSide::Buy)]
    #[case(MAX_INVENTORY + 1.0, OrderSide::Buy)]
    #[case(MIN_INVENTORY, OrderSide::Sell)]
    #[case(MIN_INVENTORY - 1.0, OrderSide::Sell)]
    fn inventory_limit_blocks_orders_that_increase_exposure(
        #[case] inventory: f64,
        #[case] side: OrderSide,
    ) {
        let orders = Slab::new();
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };

        let outcome = RiskManager.evaluate_order(&orders, order_builder, inventory, 0.0);

        assert_eq!(outcome, Outcome::DoNothing);
    }

    #[rstest]
    #[case(MAX_INVENTORY, OrderSide::Sell)]
    #[case(MIN_INVENTORY, OrderSide::Buy)]
    fn inventory_limit_allows_orders_that_reduce_exposure(
        #[case] inventory: f64,
        #[case] side: OrderSide,
    ) {
        let orders = Slab::new();
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };

        let outcome = RiskManager.evaluate_order(&orders, order_builder.clone(), inventory, 0.0);

        assert_eq!(outcome, Outcome::NewOrder(order_builder));
    }

    #[test]
    fn empty_orders_returns_new_order() {
        let orders = Slab::new();
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };

        let outcome = RiskManager.evaluate_order(&orders, order_builder.clone(), 0.0, 0.0);

        assert_eq!(outcome, Outcome::NewOrder(order_builder));
    }

    #[test]
    fn opposite_side_open_order_does_not_block_new_order() {
        let existing_order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let mut existing_order = existing_order_builder.build(1000);
        existing_order.order_status = OrderStatus::New;
        let mut orders = Slab::new();
        orders.insert(existing_order);
        let new_order = OrderBuilder {
            side: OrderSide::Buy,
            ..existing_order_builder
        };

        let outcome = RiskManager.evaluate_order(&orders, new_order.clone(), 0.0, 0.0);

        assert_eq!(outcome, Outcome::NewOrder(new_order));
    }

    #[test]
    fn closed_same_side_order_does_not_block_new_order() {
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let mut closed_order = order_builder.build(1000);
        closed_order.order_status = OrderStatus::Filled;
        let mut orders = Slab::new();
        orders.insert(closed_order);

        let outcome = RiskManager.evaluate_order(&orders, order_builder.clone(), 0.0, 0.0);

        assert_eq!(outcome, Outcome::NewOrder(order_builder));
    }

    #[test]
    fn unchanged_open_order_returns_do_nothing() {
        let order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let mut existing_order = order_builder.build(1000);
        existing_order.order_status = OrderStatus::New;
        let mut orders = Slab::new();
        orders.insert(existing_order);

        let outcome = RiskManager.evaluate_order(&orders, order_builder, 0.0, 0.0);

        assert_eq!(outcome, Outcome::DoNothing);
    }

    #[rstest]
    #[case("0.568", 25.0, true, false)]
    #[case("0.567", 30.0, false, true)]
    #[case("0.568", 30.0, true, true)]
    fn changed_open_order_returns_amendment(
        #[case] price: &str,
        #[case] qty: f64,
        #[case] expected_new_price: bool,
        #[case] expected_new_qty: bool,
    ) {
        let existing_order_builder = OrderBuilder {
            symbol: "ADAUSDT".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            qty: 25.0,
            price: "0.567".to_string(),
        };
        let mut existing_order = existing_order_builder.build(1000);
        existing_order.order_status = OrderStatus::New;
        let mut orders = Slab::new();
        orders.insert(existing_order);
        let changed_order = OrderBuilder {
            qty,
            price: price.to_string(),
            ..existing_order_builder
        };

        let outcome = RiskManager.evaluate_order(&orders, changed_order, 0.0, 0.0);

        assert_eq!(
            outcome,
            Outcome::AmendOrder(OrderAmendedBuilder {
                symbol: "ADAUSDT".to_string(),
                order_link_id: 1000,
                qty,
                price: price.to_string(),
                new_price: expected_new_price,
                new_qty: expected_new_qty,
            })
        );
    }

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
