use exchange::OrderSide;

#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    inventory: f64,
    average_entry_price: f64,
}

impl Metrics {
    pub fn new(inventory: f64, average_entry_price: f64) -> Self {
        Self {
            inventory,
            average_entry_price,
        }
    }

    pub fn inventory(&self) -> f64 {
        self.inventory
    }

    pub fn average_entry_price(&self) -> f64 {
        self.average_entry_price
    }

    /// Updates the inventory and average entry price with the latest execution.
    ///
    /// Inventory accounts for base-asset fees on buys. Average entry price
    /// excludes execution fees.
    pub fn update(&mut self, exec_price: f64, exec_qty: f64, exec_fee: f64, order_side: OrderSide) {
        let new_inventory = match order_side {
            // NOTE: On a Buy, the fee is paid in the base asset (e.g., ADA). We must subtract it.
            // On a Sell, the fee is paid in the quote asset (USDT), no additional fee to be
            // removed.
            OrderSide::Buy => self.inventory + exec_qty - exec_fee,
            OrderSide::Sell => self.inventory - exec_qty,
        };

        let new_average_entry_price = if self.inventory.abs() < 1e-8 {
            // New position is opened, take the exec_price as the average_entry_price
            exec_price
        } else if (self.inventory.is_sign_positive() && order_side == OrderSide::Buy)
            || (self.inventory.is_sign_negative() && order_side == OrderSide::Sell)
        {
            // If execution update is on the same side, re-calculate the average_entry_price
            let new_value = exec_qty * exec_price;
            let old_value = self.inventory.abs() * self.average_entry_price;
            let total_value = new_value + old_value;
            total_value / new_inventory.abs()
        } else if new_inventory.abs() < 1e-8 {
            // Existing position is closed, reset the average_entry_price to 0
            0.0
        } else {
            // NOTE: no need to worry about +/-0.0 because it is checked in the first case.
            let crossed_zero = self.inventory.signum() != new_inventory.signum();

            if crossed_zero {
                exec_price
            } else {
                // If we didn't cross zero average_entry_price stays the same!
                self.average_entry_price
            }
        };

        self.inventory = new_inventory;
        self.average_entry_price = new_average_entry_price;
    }
}

#[cfg(test)]
mod tests {
    use assert_approx_eq::assert_approx_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(0.0, 0.0, 2.0, 10.0, 0.0, OrderSide::Buy, 2.0)] // 1. Flat to long
    #[case(0.0, 0.0, 2.0, 10.0, 0.1, OrderSide::Buy, 2.0)] // 1. Flat to long w fee
    #[case(0.0, 0.0, 2.0, 10.0, 0.0, OrderSide::Sell, 2.0)] // 2. Flat to short
    #[case(0.0, 0.0, 2.0, 10.0, 0.1, OrderSide::Sell, 2.0)] // 2. Flat to short w fee
    #[case(1.0, 10.0, 2.0, 5.0, 0.0, OrderSide::Buy, 1.333333)] // 3. Increase a long
    #[case(1.0, 10.0, 2.0, 5.0, 0.1, OrderSide::Buy, 1.333333)] // 3. Increase a long w fee
    #[case(1.0, 10.0, 2.0, 4.0, 0.0, OrderSide::Sell, 1.0)] // 4. Reduce a long
    #[case(1.0, 10.0, 2.0, 4.0, 0.1, OrderSide::Sell, 1.0)] // 4. Reduce a long w fee
    #[case(1.0, 10.0, 2.0, 10.0, 0.0, OrderSide::Sell, 0.0)] // 5. Close a long
    #[case(1.0, 10.0, 2.0, 10.0, 0.1, OrderSide::Sell, 0.0)] // 5. Close a long w fee
    #[case(1.0, 10.0, 2.0, 15.0, 0.0, OrderSide::Sell, 2.0)] // 6. Cross from long to short
    #[case(1.0, 10.0, 2.0, 15.0, 0.1, OrderSide::Sell, 2.0)] // 6. Cross from long to short w fee
    #[case(2.0, -10.0, 1.0, 5.0, 0.0, OrderSide::Sell, 1.666666)] // 7. Increase a short
    #[case(2.0, -10.0, 1.0, 5.0, 0.1, OrderSide::Sell, 1.666666)] // 7. Increase a short w fee
    #[case(2.0, -10.0, 1.0, 4.0, 0.0, OrderSide::Buy, 2.0)] // 8. Reduce a short
    #[case(2.0, -10.0, 1.0, 4.0, 0.1, OrderSide::Buy, 2.0)] // 8. Reduce a short w fee
    #[case(2.0, -10.0, 1.0, 10.0, 0.0, OrderSide::Buy, 0.0)] // 9. Close a short
    #[case(2.0, -10.0, 1.0, 10.0, 0.1, OrderSide::Buy, 0.0)] // 9. Close a short w fee
    #[case(2.0, -10.0, 1.0, 15.0, 0.0, OrderSide::Buy, 1.0)] // 10. Cross from short to long
    #[case(2.0, -10.0, 1.0, 15.0, 0.1, OrderSide::Buy, 1.0)] // 10. Cross from short to long w fee
    fn test_average_entry_price(
        #[case] average_entry_price: f64,
        #[case] inventory: f64,
        #[case] exec_price: f64,
        #[case] exec_qty: f64,
        #[case] exec_fee: f64,
        #[case] order_side: OrderSide,
        #[case] expected_average_entry_price: f64,
    ) {
        let mut metrics = Metrics::new(inventory, average_entry_price);
        metrics.update(exec_price, exec_qty, exec_fee, order_side);

        assert_approx_eq!(metrics.average_entry_price(), expected_average_entry_price);
    }

    #[rstest]
    #[case(0.0, 22.0, 0.01, OrderSide::Buy, 21.99)]
    #[case(0.0, 22.0, 0.01, OrderSide::Sell, -22.0)]
    #[case(50.0, 22.0, 0.01, OrderSide::Buy, 71.99)]
    #[case(50.0, 22.0, 0.01, OrderSide::Sell, 28.0)]
    #[case(10.0, 22.0, 0.0, OrderSide::Sell, -12.0)]
    #[case(-50.0, 22.0, 0.01, OrderSide::Sell, -72.0)]
    #[case(-50.0, 22.0, 0.01, OrderSide::Buy, -28.01)]
    #[case(-10.0, 22.0, 0.01, OrderSide::Buy, 11.99)]
    #[case(22.0, 22.0, 0.0, OrderSide::Sell, 0.0)]
    #[case(-22.0, 22.0, 0.0, OrderSide::Buy, 0.0)]
    fn test_inventory(
        #[case] inventory: f64,
        #[case] exec_qty: f64,
        #[case] exec_fee: f64,
        #[case] order_side: OrderSide,
        #[case] expected_inventory: f64,
    ) {
        let mut metrics = Metrics::new(inventory, 1.0);
        metrics.update(0.567, exec_qty, exec_fee, order_side);

        assert_approx_eq!(metrics.inventory(), expected_inventory);
    }
}
