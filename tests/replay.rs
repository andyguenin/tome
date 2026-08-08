//! Replay tests: an event log faithfully reconstructs book state.
//!
//! Two properties, both flowing from engine determinism:
//! 1. Replaying the same log twice into the same engine type yields identical
//!    outcomes and identical final state.
//! 2. Replaying the same log into the *other* engine type also agrees — the log
//!    is engine-independent, so a `BTreeMap` book and a ladder book reconstruct
//!    the same market from it.

use tome::journal::{random_events, replay};
use tome::{LadderBook, OrderBook};

const MIN: u64 = 1_000;
const MAX: u64 = 1_199;
const DEPTH: usize = (MAX - MIN + 1) as usize;

#[test]
fn replay_is_deterministic_and_engine_independent() {
    for seed in [3u64, 17, 99, 0xFEED_FACE] {
        let events = random_events(seed, 20_000, MIN, MAX, 10);

        // Same engine type, replayed twice.
        let mut a = OrderBook::new();
        let mut b = OrderBook::new();
        let out_a = replay(&mut a, &events);
        let out_b = replay(&mut b, &events);
        assert_eq!(out_a, out_b, "seed {seed}: non-deterministic outcomes");
        assert_eq!(a.l2(DEPTH), b.l2(DEPTH), "seed {seed}: non-deterministic state");

        // A different engine implementation reconstructs the same market.
        let mut ladder = LadderBook::new(MIN, MAX);
        let out_ladder = replay(&mut ladder, &events);
        assert_eq!(out_ladder, out_a, "seed {seed}: ladder replay diverged");
        assert_eq!(ladder.l2(DEPTH), a.l2(DEPTH), "seed {seed}: ladder state diverged");
    }
}
