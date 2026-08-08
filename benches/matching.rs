//! Throughput benchmarks for the matching engine.
//!
//! Run with `cargo bench`. Each benchmark pre-generates a fixed, deterministic
//! order flow *outside* the timed loop, then measures only the cost of feeding
//! it through a fresh [`OrderBook`].

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

use tome::sim::{random_limit_flow, LimitOrder};
use tome::OrderBook;

/// Replay a pre-built flow through a fresh book. Returns the book so the
/// optimiser can't discard the work.
fn replay(flow: &[LimitOrder]) -> OrderBook {
    let mut book = OrderBook::new();
    for o in flow {
        book.submit_limit(o.side, o.price, o.qty);
    }
    book
}

fn bench_submit(c: &mut Criterion) {
    let mut group = c.benchmark_group("submit_limit");

    // A tight band => lots of crossing (deep matching); a wide band => more
    // resting orders and a bigger book. Both are worth watching.
    for &band in &[10_u64, 1_000] {
        const N: usize = 100_000;
        let flow = random_limit_flow(0xC10B, N, 100_000, band, 20);

        group.throughput(Throughput::Elements(N as u64));
        group.bench_function(format!("n{N}_band{band}"), |b| {
            b.iter_batched(|| &flow, |f| replay(f), BatchSize::SmallInput)
        });
    }

    group.finish();
}

criterion_group!(benches, bench_submit);
criterion_main!(benches);
