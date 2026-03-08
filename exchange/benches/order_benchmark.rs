use std::env;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use criterion::{criterion_group, criterion_main, Criterion};
use exchange::bybit::order::OrderHandler;
use exchange::{OrderBuilder, OrderSide, OrderType};

fn bench_order_handler(c: &mut Criterion) {
    unsafe {
        env::set_var("API_KEY", "dummy_benchmark_key");
        env::set_var("API_SECRET", "dummy_benchmark_secret");
    }

    let start_time_micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System clock went backwards!")
        .as_micros() as u64;
    let id_generator = AtomicU64::new(start_time_micros);

    let handler = OrderHandler::new();
    let submit_builder = OrderBuilder {
        symbol: "ADAUSDT".to_string(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        qty: 500.0,
        price: 0.278.to_string(),
    };

    // NOTE: amend_order isn't tested because it requires an existing order. We can
    // assume the benchmark results are similar to submit_order

    let mut group = c.benchmark_group("OrderHandler");
    group.warm_up_time(Duration::from_millis(10));
    group.sample_size(10);
    group.measurement_time(Duration::from_millis(100));

    group.bench_function("submit_order", |b| {
        b.iter(|| {
            // NOTE: fetch_add should be so small not to have a significant impact on the
            // benchmark results
            handler.submit_order(
                black_box(&submit_builder),
                black_box(id_generator.fetch_add(1, Ordering::Relaxed)),
            );
        })
    });

    group.finish();
}

criterion_group!(benches, bench_order_handler);
criterion_main!(benches);
