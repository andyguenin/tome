//! Deterministic synthetic order flow, for demos and benchmarks.
//!
//! No external RNG crate: [`Rng`] is a SplitMix64, which is tiny, fast, and
//! fully reproducible from a seed — exactly what you want for benchmarks you
//! can compare run to run.

use crate::types::{Price, Qty, Side};

/// A minimal SplitMix64 pseudo-random generator.
pub struct Rng(u64);

impl Rng {
    /// Seed the generator. The same seed always yields the same sequence.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (n must be non-zero).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// A single limit order to feed into the book.
#[derive(Debug, Clone, Copy)]
pub struct LimitOrder {
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
}

/// Generate `n` random limit orders clustered around `mid`.
///
/// Prices fall in `mid ± band` and quantities in `1..=max_qty`, with side a
/// coin flip. Because buys and sells overlap in price, this flow produces a
/// realistic mix of crossing (matching) and resting orders — good exercise for
/// the matching engine.
pub fn random_limit_flow(seed: u64, n: usize, mid: Price, band: Price, max_qty: Qty) -> Vec<LimitOrder> {
    let mut rng = Rng::new(seed);
    let span = band * 2 + 1;
    (0..n)
        .map(|_| {
            let side = if rng.next_u64() & 1 == 0 { Side::Bid } else { Side::Ask };
            let price = mid - band + rng.below(span);
            let qty = 1 + rng.below(max_qty);
            LimitOrder { side, price, qty }
        })
        .collect()
}
