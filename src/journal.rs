//! An event log for the order book: every mutating operation as a plain,
//! copyable [`Event`], plus [`apply`]/[`replay`] to feed events into any
//! [`Engine`].
//!
//! # Why
//!
//! The engines are deterministic: the same operations in the same order always
//! produce the same trades and the same book. An event log turns that property
//! into something useful — record the stream once and you can reconstruct the
//! exact book state on a fresh engine (or a *different* engine implementation),
//! which is the backbone of crash recovery, audit, and the eventual `~/morse`
//! plugin. Order ids are implicit: they fall out of the submit order, so events
//! never carry them for submits, only reference them for cancel/amend.

use crate::engine::Engine;
use crate::sim::Rng;
use crate::types::{OrderId, Price, Qty, Side, SubmitResult};

/// A single mutating operation against a book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Submit a limit order.
    Limit { side: Side, price: Price, qty: Qty },
    /// Submit a market order.
    Market { side: Side, qty: Qty },
    /// Cancel a resting order by id.
    Cancel { id: OrderId },
    /// Change a resting order's quantity.
    Amend { id: OrderId, new_qty: Qty },
}

/// The result of applying one [`Event`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A submit (limit or market) and its fills/remainder.
    Submitted(SubmitResult),
    /// Whether a cancel found a resting order.
    Cancelled(bool),
    /// Whether an amend found a resting order.
    Amended(bool),
}

/// Apply one event to `engine`, returning its outcome.
pub fn apply(engine: &mut dyn Engine, event: &Event) -> Outcome {
    match *event {
        Event::Limit { side, price, qty } => Outcome::Submitted(engine.submit_limit(side, price, qty)),
        Event::Market { side, qty } => Outcome::Submitted(engine.submit_market(side, qty)),
        Event::Cancel { id } => Outcome::Cancelled(engine.cancel(id)),
        Event::Amend { id, new_qty } => Outcome::Amended(engine.amend(id, new_qty)),
    }
}

/// Replay a whole event stream into `engine`, collecting every outcome.
pub fn replay(engine: &mut dyn Engine, events: &[Event]) -> Vec<Outcome> {
    events.iter().map(|e| apply(engine, e)).collect()
}

/// Generate a deterministic, realistic mix of events for tests and demos.
///
/// Prices fall in `[min_price, max_price]` and quantities in `1..=max_qty`.
/// Cancels and amends target ids drawn from the submits generated so far
/// (some will already be filled — exercising the "not resting" paths too).
pub fn random_events(
    seed: u64,
    n: usize,
    min_price: Price,
    max_price: Price,
    max_qty: Qty,
) -> Vec<Event> {
    let mut rng = Rng::new(seed);
    let span = max_price - min_price + 1;
    let mut submitted: OrderId = 0;
    let mut out = Vec::with_capacity(n);

    for _ in 0..n {
        let side = if rng.next_u64() & 1 == 0 { Side::Bid } else { Side::Ask };
        let event = match rng.below(100) {
            0..=54 => {
                submitted += 1;
                Event::Limit { side, price: min_price + rng.below(span), qty: 1 + rng.below(max_qty) }
            }
            55..=74 => {
                submitted += 1;
                Event::Market { side, qty: 1 + rng.below(max_qty) }
            }
            75..=87 if submitted > 0 => Event::Cancel { id: rng.below(submitted) },
            _ if submitted > 0 => Event::Amend { id: rng.below(submitted), new_qty: 1 + rng.below(max_qty) },
            // Nothing to cancel/amend yet — emit a limit instead.
            _ => {
                submitted += 1;
                Event::Limit { side, price: min_price + rng.below(span), qty: 1 + rng.below(max_qty) }
            }
        };
        out.push(event);
    }
    out
}
