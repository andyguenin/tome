//! The [`Engine`] trait: the operations every matching engine implements.
//!
//! Both [`crate::book::OrderBook`] (BTreeMap) and [`crate::ladder::LadderBook`]
//! (array + bitmap) implement this. Pinning the surface down as a trait lets
//! callers be generic over the price-index strategy, and — more usefully — lets
//! the differential test drive *both* engines through the same sequence of
//! operations and assert they agree (see `tests/differential.rs`).
//!
//! Construction is intentionally *not* part of the trait: the ladder needs a
//! bounded price range up front while the BTreeMap does not, so each engine
//! keeps its own constructor.

use crate::types::{L2Snapshot, OrderId, Price, Qty, Side, SubmitResult};

/// A price-time-priority matching engine.
pub trait Engine {
    /// Submit a limit order; matches then rests any remainder at `price`.
    fn submit_limit(&mut self, side: Side, price: Price, qty: Qty) -> SubmitResult;

    /// Submit a market order: match against the best available prices with no
    /// price limit. Never rests — any unfilled remainder is cancelled and
    /// reported in [`SubmitResult::resting`].
    fn submit_market(&mut self, side: Side, qty: Qty) -> SubmitResult;

    /// Cancel a resting order. Returns whether it was on the book.
    fn cancel(&mut self, id: OrderId) -> bool;

    /// Change a resting order's quantity.
    ///
    /// Reducing keeps the order's time priority (an `O(1)` in-place shrink).
    /// Increasing forfeits priority: the order moves to the back of its price
    /// level, keeping its id. Returns `false` if the order isn't resting.
    /// `new_qty` must be positive; use [`Engine::cancel`] to remove an order.
    fn amend(&mut self, id: OrderId, new_qty: Qty) -> bool;

    /// The highest resting bid price, if any.
    fn best_bid(&self) -> Option<Price>;

    /// The lowest resting ask price, if any.
    fn best_ask(&self) -> Option<Price>;

    /// The gap between best ask and best bid, if both sides are populated.
    fn spread(&self) -> Option<Price>;

    /// Total resting quantity at `price` on the given side (0 if none).
    fn depth_at(&self, side: Side, price: Price) -> Qty;

    /// A snapshot of the top `depth` price levels per side.
    fn l2(&self, depth: usize) -> L2Snapshot;
}
