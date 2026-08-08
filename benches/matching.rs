//! Throughput benchmarks: `BTreeMap` engine vs. flat array price ladder.
//!
//! Run with `cargo bench`. Each benchmark pre-generates a fixed, deterministic
//! order flow *outside* the timed loop, then measures only the cost of feeding
//! it through a fresh book. Both engines see the identical flow, so the only
//! variable is the price-index data structure.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

use tome::sim::{random_limit_flow, LimitOrder};
use tome::{LadderBook, OrderBook};

const MID: u64 = 100_000;

/// Replay a flow through a fresh `BTreeMap` book.
fn replay_btree(flow: &[LimitOrder]) -> OrderBook {
    let mut book = OrderBook::new();
    for o in flow {
        book.submit_limit(o.side, o.price, o.qty);
    }
    book
}

/// Replay a flow through a fresh array-ladder book sized to `band`.
fn replay_ladder(flow: &[LimitOrder], band: u64) -> LadderBook {
    let mut book = LadderBook::new(MID - band, MID + band);
    for o in flow {
        book.submit_limit(o.side, o.price, o.qty);
    }
    book
}

fn bench_submit(c: &mut Criterion) {
    let mut group = c.benchmark_group("submit_limit");

    // A tight band => lots of crossing (shallow book); a wide band => more
    // resting orders and a deeper book, where the BTreeMap's O(log n) bites.
    for &band in &[10_u64, 1_000] {
        const N: usize = 100_000;
        let flow = random_limit_flow(0xC10B, N, MID, band, 20);
        group.throughput(Throughput::Elements(N as u64));

        group.bench_function(format!("btree/band{band}"), |b| {
            b.iter_batched(|| &flow, |f| replay_btree(f), BatchSize::SmallInput)
        });
        group.bench_function(format!("ladder/band{band}"), |b| {
            b.iter_batched(|| &flow, |f| replay_ladder(f, band), BatchSize::SmallInput)
        });
    }

    group.finish();
}

criterion_group!(benches, bench_submit);
criterion_main!(benches);
