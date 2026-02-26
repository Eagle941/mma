use std::str::FromStr;

use exchange::{Order, OrderAmendedBuilder, OrderBuilder, OrderSide};
use slab::Slab;

pub enum Outcome {
    NewOrder(OrderBuilder),
    AmendOrder(OrderAmendedBuilder),
    Nothing,
}

// TODO: This file may be moved to a dedicated library
pub struct RiskManager();
impl RiskManager {
    fn get_existing_order(orders: &Slab<Order>, side: OrderSide) -> Option<(usize, &Order)> {
        orders
            .iter()
            .find(|(_, o)| o.order_status.is_open() && side == o.side)
    }

    // fn get_opposite_order(
    //     orders: &HashMap<String, Order>,
    //     side: OrderSide,
    // ) -> Option<(&String, &Order)> {
    //     orders
    //         .iter()
    //         .filter(|(_, o)| {
    //             o.order_status.is_open() && side != o.side &&
    // OrderSide::NotAvailable != o.side         })
    //         .next()
    // }

    pub fn submit_order(
        orders: &Slab<Order>,
        new_order: OrderBuilder,
        inventory: f64,
        last_buy: Option<&Order>,
        last_sell: Option<&Order>,
    ) -> Outcome {
        // NOTE: could be dynamic
        const MAKER_FEE: f64 = 0.0676; // %
        const MAX_INVENTORY: f64 = 500.0;
        const MIN_INVENTORY: f64 = -500.0;

        if inventory >= MAX_INVENTORY && new_order.side == OrderSide::Buy {
            return Outcome::Nothing;
        }

        if inventory <= MIN_INVENTORY && new_order.side == OrderSide::Sell {
            return Outcome::Nothing;
        }

        // This is a very simple risk management. Don't have more than two orders
        // running at the same time.
        // NOTE: Assumption is that there is only one active order per side at a time!
        let Some((_, existing_order)) = RiskManager::get_existing_order(orders, new_order.side)
        else {
            return Outcome::NewOrder(new_order);
        };

        let new_order_price = f64::from_str(new_order.price.as_str()).unwrap();
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
            return Outcome::Nothing;
        }

        match new_order.side {
            OrderSide::Buy => {
                let Some(last_sell) = last_sell else {
                    return Outcome::AmendOrder(amended_order);
                };
                if new_order_price >= -last_sell.filled_price * (MAKER_FEE * 2.0 / 100.0 - 1.0) {
                    return Outcome::Nothing;
                }
            }
            OrderSide::Sell => {
                let Some(last_buy) = last_buy else {
                    return Outcome::AmendOrder(amended_order);
                };
                if new_order_price <= last_buy.filled_price * (MAKER_FEE * 2.0 / 100.0 + 1.0) {
                    return Outcome::Nothing;
                }
            }
            OrderSide::NotAvailable => return Outcome::Nothing,
        };
        Outcome::AmendOrder(amended_order)
    }
}
