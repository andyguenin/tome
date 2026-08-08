//! Differential fuzz test: the two engines must be indistinguishable.
//!
//! We drive a [`BTreeMap`](tome::OrderBook) book and a
//! [`LadderBook`](tome::LadderBook) through the *same* pseudo-random stream of
//! operations and assert they agree at every step — identical trades, identical
//! return values, identical book state. Because the two engines share no code
//! above [`tome::pool`], any divergence is a real bug in one of them.
//!
//! Order ids stay in lockstep for free: both engines start at 0 and bump the
//! same counter on every submit (limit *and* market), so a cancel/amend target
//! computed once is valid for both.

use tome::sim::Rng;
use tome::{Engine, LadderBook, OrderBook, Side};

const MIN: u64 = 1_000;
const MAX: u64 = 1_199; // 200 ticks — wide enough to exercise the bitmap summary
const SPAN: u64 = MAX - MIN + 1;
const MAX_QTY: u64 = 10;
const SNAPSHOT_DEPTH: usize = SPAN as usize; // compare the *entire* book each step

fn side(rng: &mut Rng) -> Side {
    if rng.next_u64() & 1 == 0 { Side::Bid } else { Side::Ask }
}

fn price(rng: &mut Rng) -> u64 {
    MIN + rng.below(SPAN)
}

fn qty(rng: &mut Rng) -> u64 {
    1 + rng.below(MAX_QTY)
}

/// Apply one random op to both engines and assert they still agree.
fn step(rng: &mut Rng, a: &mut dyn Engine, b: &mut dyn Engine, next_id: &mut u64) {
    match rng.below(100) {
        // ~55%: limit order
        0..=54 => {
            let (s, p, q) = (side(rng), price(rng), qty(rng));
            let ra = a.submit_limit(s, p, q);
            let rb = b.submit_limit(s, p, q);
            assert_eq!(ra, rb, "limit {s:?} {q}@{p} diverged");
            assert_eq!(ra.id, *next_id, "id counters drifted");
            *next_id += 1;
        }
        // ~20%: market order
        55..=74 => {
            let (s, q) = (side(rng), qty(rng));
            let ra = a.submit_market(s, q);
            let rb = b.submit_market(s, q);
            assert_eq!(ra, rb, "market {s:?} {q} diverged");
            *next_id += 1;
        }
        // ~13%: cancel a (possibly stale) id
        75..=87 if *next_id > 0 => {
            let id = rng.below(*next_id);
            assert_eq!(a.cancel(id), b.cancel(id), "cancel #{id} diverged");
        }
        // ~12%: amend a (possibly stale) id
        _ if *next_id > 0 => {
            let (id, q) = (rng.below(*next_id), qty(rng));
            assert_eq!(a.amend(id, q), b.amend(id, q), "amend #{id}->{q} diverged");
        }
        // Nothing submitted yet: seed the book with a limit order.
        _ => {
            let (s, p, q) = (side(rng), price(rng), qty(rng));
            assert_eq!(a.submit_limit(s, p, q), b.submit_limit(s, p, q));
            *next_id += 1;
        }
    }

    // Full structural agreement after every single operation.
    assert_eq!(a.best_bid(), b.best_bid(), "best_bid diverged");
    assert_eq!(a.best_ask(), b.best_ask(), "best_ask diverged");
    assert_eq!(a.spread(), b.spread(), "spread diverged");
    assert_eq!(a.l2(SNAPSHOT_DEPTH), b.l2(SNAPSHOT_DEPTH), "book state diverged");
}

/// Run one seed for `ops` operations.
fn run(seed: u64, ops: usize) {
    let mut rng = Rng::new(seed);
    let mut btree = OrderBook::new();
    let mut ladder = LadderBook::new(MIN, MAX);
    let mut next_id = 0;

    for _ in 0..ops {
        step(&mut rng, &mut btree, &mut ladder, &mut next_id);
    }

    // The run should have actually done something interesting.
    assert!(next_id > 0);
}

#[test]
fn engines_agree_across_random_flows() {
    // A spread of seeds, each a long op stream. Deterministic, so any failure
    // reproduces exactly.
    for seed in [1u64, 7, 42, 1_000, 0xDEAD_BEEF, 0xC10B_C10B] {
        run(seed, 20_000);
    }
}

#[test]
fn engines_agree_on_amend_heavy_flow() {
    // Amends and cancels churn resting orders and exercise priority handling;
    // a dedicated long run over a single seed hammers that path.
    run(0xA5A5_A5A5, 50_000);
}
