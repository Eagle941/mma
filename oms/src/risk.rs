use std::collections::HashMap;

use exchange::{Order, OrderBuilder};

// TODO: This file may be moved to a dedicated library
pub struct RiskManager();
impl RiskManager {
    pub fn submit_order(orders_history: &HashMap<String, Order>, new_order: &OrderBuilder) -> bool {
        // This is a very simple risk management. Don't have more than two orders
        // running at the same time.
        let num_active_orders = orders_history
            .iter()
            .filter(|(_, o)| o.order_status.is_open() && new_order.side == o.side)
            .count();

        if num_active_orders == 0 {
            return true;
        }
        return false;
    }
}
