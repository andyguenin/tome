//! Per-operation latency distribution for both engines.
//!
//! Throughput (orders/sec) is an *average* — it hides the tail. This example
//! times each `submit_limit` individually and reports percentiles, so you can
//! see p50 vs p99 vs the worst case. Run it in release for meaningful numbers:
//!
//! ```text
//! cargo run --release --example latency
//! ```
//!
//! Caveat: timing a single ~tens-of-nanoseconds operation includes the clock's
//! own read overhead, so treat the absolute low-percentile values as an upper
//! bound. The *shape* (how far p99/max sit above p50) is the interesting part —
//! that's where reallocations and tree rebalancing show up.

use std::time::Instant;

use tome::sim::{random_limit_flow, LimitOrder};
use tome::{Engine, LadderBook, OrderBook};

const MID: u64 = 100_000;
const BAND: u64 = 1_000;
const N: usize = 200_000;

/// Time each order, returning per-op latencies in nanoseconds.
fn latencies(engine: &mut dyn Engine, flow: &[LimitOrder]) -> Vec<u64> {
    let mut samples = Vec::with_capacity(flow.len());
    for o in flow {
        let start = Instant::now();
        engine.submit_limit(o.side, o.price, o.qty);
        samples.push(start.elapsed().as_nanos() as u64);
    }
    samples
}

/// The value at percentile `p` (0..=100) of an already-sorted slice.
fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank]
}

fn report(name: &str, mut samples: Vec<u64>) {
    samples.sort_unstable();
    let sum: u128 = samples.iter().map(|&x| x as u128).sum();
    let mean = sum / samples.len() as u128;
    println!(
        "{name:8}  mean {mean:>4}ns   p50 {:>4}ns   p90 {:>4}ns   p99 {:>5}ns   p99.9 {:>6}ns   max {:>7}ns",
        pct(&samples, 50.0),
        pct(&samples, 90.0),
        pct(&samples, 99.0),
        pct(&samples, 99.9),
        pct(&samples, 100.0),
    );
}

fn main() {
    let flow = random_limit_flow(0xC10B, N, MID, BAND, 20);
    println!("per-op submit_limit latency over {N} orders (band +/-{BAND}):\n");

    report("btree", latencies(&mut OrderBook::new(), &flow));
    report("ladder", latencies(&mut LadderBook::new(MID - BAND, MID + BAND), &flow));
}
