Confirmed bugs

### 1. Duplicate execution messages double-count inventory

`order_response` applies every `ExecutionUpdate` unconditionally:

```rust
self.metrics.update(
    order.exec_price,
    order.exec_qty,
    order.exec_fee,
    order.order_side,
);
```

Although `OrderExecution` contains `exec_id`, the OMS never records it.

If the same execution is received twice:

```text
Initial inventory: 0
Execution: buy 10, fee 0.01

After first message:  9.99
After duplicate:     19.98
```

The second message also potentially:

- Changes average entry price again
- Publishes incorrect inventory to the strategy
- Triggers liability repayment incorrectly

This is a financial-state corruption bug regardless of why a duplicate arrives. Whether Bybit can replay messages after reconnect should be verified, but the OMS is currently non-idempotent.

**Recommended fix:** retain processed execution IDs, at least for the lifetime of active/recent orders, and ignore duplicates.

---

### 2. Average entry price uses fee-adjusted inventory

This is already demonstrated by the two failing tests in `oms/src/metrics.rs`.

Confirmed failures:

- Increasing a long with a buy fee
- Closing a short with a buy fee

The root issue is using fee-adjusted inventory to determine gross position direction and average execution price.

This requires separate:

```rust
inventory
position_qty
average_entry_price
```

as previously discussed.

---

### 3. `Submitted` orders do not prevent duplicate same-side submissions

`forward_orders` inserts an order immediately:

```rust
let order_link_id = self.insert_new_order(&order);
self.order_gateway.submit_order(&order, order_link_id)
```

The inserted order has status `Submitted`.

However, `RiskManager::get_existing_order` only considers:

```rust
o.order_status.is_open()
```

`Submitted` is intentionally not considered open. Therefore, before Bybit confirms the first order, another strategy command can create another same-side order.

This violates the documented risk assumption:

```text
one active order per side at a time
```

Example:

1. Strategy sends buy order.
2. OMS stores it as `Submitted`.
3. HTTP request is still pending.
4. Strategy sends another buy order.
5. Risk manager does not consider the first order open.
6. OMS submits a second buy order.

**Recommended fix:** risk evaluation should separately consider locally pending submissions, without redefining `Submitted` as exchange-confirmed open.

---

### 4. Failed submissions remain permanently stored as `Submitted`

`OrderGateway::submit_order` returns `()` and the Bybit implementation starts an asynchronous task. The OMS cannot learn whether dispatch or exchange acceptance failed.

Consequences:

- The order remains in `orders`
- Its `id_map` entry remains
- It may never receive an exchange update
- Because `Submitted` is not open, it does not prevent additional orders
- The slab grows with phantom orders

The gateway documentation explicitly states that dispatch does not mean acceptance, but there is no failure path back into the OMS.

**Recommended fix:** represent command results explicitly, for example with a gateway-result channel:

```rust
enum GatewayResult {
    OrderDispatched { order_link_id: u64 },
    OrderDispatchFailed {
        order_link_id: u64,
        error: GatewayError,
    },
}
```

Exchange rejection should also have an explicit order status or response message.

---

### 5. Execution side is trusted instead of checked against the stored order

For a known order ID, the OMS calculates metrics using:

```rust
order.order_side
```

It only verifies that the order exists:

```rust
if self.orders.contains(*slab_id)
```

A malformed or inconsistent execution message can therefore update inventory in the wrong direction.

Because `order_link_id` identifies an internally stored order, the OMS should either:

- Use the stored order’s side, or
- Verify that `execution.order_side == stored_order.side` and reject mismatches

The second option is safer because it detects inconsistent exchange data.

The previous implementation used the stored order side; the current implementation lost that protection.

---

### 6. Closed orders are never removed

The code already contains the relevant TODO.

Every submitted order remains in:

```rust
orders: Slab<Order>
id_map: FxHashMap<u64, usize>
```

indefinitely.

This causes:

- Unbounded memory growth
- Increasing risk-policy scan cost
- Retention of obsolete IDs
- More complicated execution deduplication and recovery

When removing an order, both structures must be updated atomically:

```rust
if let Some(slab_index) = self.id_map.remove(&order_link_id) {
    self.orders.remove(slab_index);
}
```

Removal timing needs care because late order and execution messages may still arrive. A bounded recent-order archive may be safer than immediate deletion.

---

### 7. Order-link ID overflow corrupts the ID map

`insert_new_order` uses:

```rust
self.id_generator.fetch_add(1, Ordering::Relaxed)
```

Atomic integer addition wraps on overflow. Once it wraps to an existing ID:

```rust
self.id_map.insert(next_order_link_id, slab_index);
```

replaces the existing mapping while leaving the old order in the slab.

Consequences:

- The old order becomes unreachable by its link ID
- Later updates can mutate the wrong order
- `orders` and `id_map` cease to represent the same relationship

The existing TODO correctly identifies this.

Although unlikely with `u64`, the failure mode is severe and deterministic.

---

### 8. Disconnected channels make `cycle` busy-loop

`cycle` ignores receive errors:

```rust
recv(self.from_strategy) -> msg => {
    if let Ok(order_builder) = msg {
        // ...
    }
}
```

A disconnected crossbeam channel remains immediately ready and returns `Err`. The loop then selects again without blocking.

If both senders disconnect, `cycle` consumes CPU indefinitely. If one disconnects, the permanently ready channel can also cause unnecessary spinning and interfere with normal processing.

**Recommended fix:** track channel closure and exit when both are closed, or treat either closure as a shutdown/error condition.

For example:

```rust
let mut strategy_open = true;
let mut order_handler_open = true;

while strategy_open || order_handler_open {
    // Disable disconnected receivers from subsequent selection.
}
```

A dedicated shutdown channel would provide clearer lifecycle control.

## Message-ordering bugs

### 9. Older order updates can overwrite newer state

`OrderUpdate` contains `updated_time`, but the OMS never compares it with the stored order’s timestamp.

An older message can overwrite:

- `Filled` with `PartiallyFilled`
- Newer filled quantity with a smaller value
- Newer price or quantity
- Newer average fill price

Example:

```text
Stored update time: 200
Incoming update time: 100
```

The incoming update is applied unconditionally.

**Recommended fix:**

```rust
if order.updated_time <= old_order.updated_time {
    warn!("Discarding stale order update...");
    return;
}
```

Whether equal timestamps should be accepted depends on Bybit’s ordering guarantees.

---

### 10. Execution ordering can change average-price results

Even after deduplication, executions are applied in receive order without considering `exec_ts`.

For executions that only increase the same position, a correct weighted average is mathematically order-independent. For sequences involving reductions or direction changes, processing order matters.

If execution messages can arrive out of order, inventory totals may remain correct, but average entry price and repayment timing may not.

This risk depends on Bybit’s stream-order guarantees. The OMS currently does not enforce or document an assumption.

Possible approaches:

- Require monotonically increasing execution timestamps per order
- Buffer and reorder within a small window
- Trust stream ordering but document and test the assumption

## Temporary state inconsistencies

### 11. Execution updates do not update the stored order

An execution modifies only `Metrics`:

```rust
self.metrics.update(...);
```

It does not update the corresponding `Order`:

- `filled_qty`
- `filled_price`
- `order_status`
- `updated_time`

The OMS relies on a separate `OrderUpdate` arriving later.

Between the execution and order update:

- Inventory reflects the fill
- The stored order does not
- Risk management can amend an already fully or partially filled order
- The order may still appear open with stale quantity

Whether this becomes a user-visible bug depends on private-stream ordering, but the OMS state is temporarily internally inconsistent.

A robust design should either:

- Apply execution information to the stored order, then reconcile with order updates, or
- Explicitly model execution state separately and make risk evaluation aware of it

## Repayment risks

### 12. Repayment failure is neither observed nor retried

Repayment is triggered only during the negative-to-positive transition:

```rust
if old_inventory.is_sign_negative()
    && self.metrics.inventory().is_sign_positive()
{
    self.order_gateway.repay_liability(&self.coin);
}
```

The gateway returns `()` and performs the request asynchronously.

If repayment fails:

- Inventory is already positive
- Future executions no longer satisfy the negative-to-positive condition
- Repayment is never retried
- The OMS has no record that liability remains outstanding

This is a confirmed state-machine gap, though actual financial impact depends on Bybit’s automatic repayment behavior.

Track repayment as an explicit state:

```text
NotRequired
Pending
Succeeded
Failed
```

and retry or alert on failure.

---

### 13. Repayment and zero tolerance use inconsistent definitions

`Metrics` treats inventory magnitudes below `1e-8` as zero for average-price decisions.

Repayment uses IEEE sign checks:

```rust
is_sign_negative()
is_sign_positive()
```

A tiny positive or negative value can therefore be:

- Considered flat by metrics
- Considered long or short by repayment logic

Use one shared position classification function:

```rust
enum PositionSide {
    Short,
    Flat,
    Long,
}
```

based on the same tolerance or instrument quantity step.

## Input validation gaps

### 14. `OmsConfig` accepts invalid numeric state

`OmsConfig::new` accepts:

- `NaN`
- Infinite inventory
- Infinite average price
- Negative average price
- Empty coin

These values propagate into risk evaluation and metrics.

At minimum, constructor validation should reject non-finite values. Whether negative prices or empty coin should be rejected is domain-specific, but likely yes.

---

### 15. Execution values are not validated before mutating financial state

The OMS accepts execution updates containing:

- Negative quantity
- Negative fee
- Fee greater than quantity
- Zero or negative price
- `NaN`
- Infinity

Exchange conversion currently parses strings but does not enforce domain validity.

A malformed message can make inventory or average price `NaN` or infinite, after which comparisons and risk limits become unreliable.

Validation is best placed at the exchange-domain conversion boundary, with defensive checks in OMS for critical invariants.

## Lower-priority issues

### Documentation is stale

`order_response` says it populates an `active_orders` `HashMap`. The implementation uses:

- `Slab<Order>`
- `id_map`

This is not a runtime bug, but misleading documentation increases maintenance risk.

### Missing `id_map`/slab invariant recovery

If `id_map` contains an index no longer present in the slab:

- `OrderUpdate` silently does nothing
- `ExecutionUpdate` silently does nothing

This cannot occur with current removal-free code except through future bugs or ID corruption, but removal work should introduce explicit invariant checks.

## Recommended priority

1. Deduplicate executions using `exec_id`
2. Separate gross position quantity from fee-adjusted inventory
3. Prevent duplicate orders while a submission is pending
4. Validate execution side against the stored order
5. Reject stale order updates
6. Add gateway failure/result handling
7. Handle channel disconnection and shutdown
8. Reconcile execution updates with stored order state
9. Add repayment state and retries
10. Prune closed orders safely
11. Handle ID overflow
12. Add numeric validation

The most dangerous current issue is duplicate execution processing because it directly corrupts inventory and can trigger incorrect trading decisions.
