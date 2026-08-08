//! A central limit order book with a price-time-priority matching engine.
//!
//! # Design
//!
//! Each side of the book is a [`BTreeMap`] keyed by [`Price`]. This gives us
//! ordered iteration for free: the best ask is the *lowest* key, the best bid
//! is the *highest* key, both reachable in `O(log n)`.
//!
//! At every price sits a [`Level`] — a FIFO intrusive linked list of resting
//! orders living in a shared [`Pool`] arena (see [`crate::pool`]). FIFO is
//! exactly price-*time* priority. A separate `OrderId -> slot` index lets us
//! cancel any order in `O(1)`.
//!
//! The price container is still a `BTreeMap`; swapping it for a flat array
//! "price ladder" and benchmarking the difference is the intended next step,
//! and this public API is meant to survive that change.

use std::collections::{BTreeMap, HashMap};

use crate::engine::{Engine, SelfTrade, ANONYMOUS};
use crate::pool::{Level, Pool};
use crate::types::{L2Snapshot, Level2, OrderId, Price, Qty, Side, SubmitResult, Trade};

/// A price-time-priority central limit order book.
pub struct OrderBook {
    /// Buy orders, keyed by price. Best bid = highest key.
    bids: BTreeMap<Price, Level>,
    /// Sell orders, keyed by price. Best ask = lowest key.
    asks: BTreeMap<Price, Level>,
    /// The arena holding every resting order node.
    pool: Pool,
    /// `OrderId -> arena slot`, for `O(1)` cancellation.
    locations: HashMap<OrderId, u32>,
    /// Self-trade-prevention policy applied to aggressor orders.
    stp: SelfTrade,
    /// Source of monotonically-increasing order ids.
    next_id: OrderId,
}

impl Default for OrderBook {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderBook {
    /// Create an empty book (self-trade prevention off).
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            pool: Pool::new(),
            locations: HashMap::new(),
            stp: SelfTrade::Allow,
            next_id: 0,
        }
    }

    /// Set the self-trade-prevention policy for subsequent aggressor orders.
    pub fn set_self_trade(&mut self, policy: SelfTrade) {
        self.stp = policy;
    }

    /// Submit an [`ANONYMOUS`] limit order (no self-trade prevention).
    ///
    /// The order first matches as much as it can against the opposite side
    /// (best price first, FIFO within a price). Any unfilled remainder rests on
    /// its own side of the book at `price`.
    pub fn submit_limit(&mut self, side: Side, price: Price, qty: Qty) -> SubmitResult {
        self.submit_limit_as(ANONYMOUS, side, price, qty)
    }

    /// Submit a limit order attributed to `owner`.
    pub fn submit_limit_as(&mut self, owner: OrderId, side: Side, price: Price, qty: Qty) -> SubmitResult {
        assert!(qty > 0, "cannot submit a zero-quantity order");

        let id = self.next_id;
        self.next_id += 1;

        let mut trades = Vec::new();
        // A buy matches into the asks taking the lowest price first; a sell
        // matches into the bids taking the highest price first.
        let outcome = match side {
            Side::Bid => match_into(
                &mut self.asks, &mut self.pool, &mut self.locations,
                id, owner, self.stp, qty, &mut trades, Best::Lowest, |ask| ask <= price,
            ),
            Side::Ask => match_into(
                &mut self.bids, &mut self.pool, &mut self.locations,
                id, owner, self.stp, qty, &mut trades, Best::Highest, |bid| bid >= price,
            ),
        };

        // A remainder only rests if it wasn't cancelled by self-trade prevention.
        let remaining = if outcome.cancelled { 0 } else { outcome.remaining };
        if remaining > 0 {
            let book = match side {
                Side::Bid => &mut self.bids,
                Side::Ask => &mut self.asks,
            };
            let level = book.entry(price).or_insert_with(Level::empty);
            let slot = self.pool.push_back(level, id, owner, price, side, remaining);
            self.locations.insert(id, slot);
        }

        SubmitResult { id, trades, resting: remaining }
    }

    /// Submit an [`ANONYMOUS`] market order.
    pub fn submit_market(&mut self, side: Side, qty: Qty) -> SubmitResult {
        self.submit_market_as(ANONYMOUS, side, qty)
    }

    /// Submit a market order attributed to `owner` — match against the best
    /// prices with no limit. Never rests; the unfilled remainder is reported in
    /// `resting`.
    pub fn submit_market_as(&mut self, owner: OrderId, side: Side, qty: Qty) -> SubmitResult {
        assert!(qty > 0, "cannot submit a zero-quantity order");

        let id = self.next_id;
        self.next_id += 1;

        let mut trades = Vec::new();
        let outcome = match side {
            Side::Bid => match_into(
                &mut self.asks, &mut self.pool, &mut self.locations,
                id, owner, self.stp, qty, &mut trades, Best::Lowest, |_| true,
            ),
            Side::Ask => match_into(
                &mut self.bids, &mut self.pool, &mut self.locations,
                id, owner, self.stp, qty, &mut trades, Best::Highest, |_| true,
            ),
        };

        SubmitResult { id, trades, resting: outcome.remaining }
    }

    /// Change a resting order's quantity (see [`Engine::amend`]).
    pub fn amend(&mut self, id: OrderId, new_qty: Qty) -> bool {
        assert!(new_qty > 0, "amend to zero; use cancel instead");

        let Some(&slot) = self.locations.get(&id) else {
            return false;
        };
        let current = self.pool.qty_of(slot);
        if new_qty == current {
            return true;
        }
        let (price, side) = self.pool.location(slot);
        let book = match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };
        let level = book.get_mut(&price).expect("resting order must have a level");

        if new_qty < current {
            // Shrink in place — time priority preserved.
            self.pool.reduce(level, slot, current - new_qty);
        } else {
            // Grow — priority is forfeited: move to the back of the level,
            // keeping the same id and owner.
            let owner = self.pool.owner_of(slot);
            self.pool.unlink(level, slot);
            let new_slot = self.pool.push_back(level, id, owner, price, side, new_qty);
            self.locations.insert(id, new_slot);
        }
        true
    }

    /// A snapshot of the top `depth` price levels per side.
    pub fn l2(&self, depth: usize) -> L2Snapshot {
        let bids = self
            .bids
            .iter()
            .rev()
            .take(depth)
            .map(|(&price, lvl)| Level2 { price, qty: lvl.total_qty })
            .collect();
        let asks = self
            .asks
            .iter()
            .take(depth)
            .map(|(&price, lvl)| Level2 { price, qty: lvl.total_qty })
            .collect();
        L2Snapshot { bids, asks }
    }

    /// Cancel a resting order by id.
    ///
    /// Returns `true` if the order was resting and is now removed, `false` if
    /// no such order is on the book (already filled, already cancelled, or an
    /// id that never rested). `O(1)` plus the `O(log n)` level lookup.
    pub fn cancel(&mut self, id: OrderId) -> bool {
        let Some(slot) = self.locations.remove(&id) else {
            return false;
        };
        let (price, side) = self.pool.location(slot);
        let book = match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };
        let level = book.get_mut(&price).expect("resting order must have a level");
        self.pool.unlink(level, slot);
        if self.pool.is_empty(level) {
            book.remove(&price);
        }
        true
    }

    /// The highest resting bid price, if any.
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.keys().next_back().copied()
    }

    /// The lowest resting ask price, if any.
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.keys().next().copied()
    }

    /// The gap between best ask and best bid, if both sides are populated.
    pub fn spread(&self) -> Option<Price> {
        Some(self.best_ask()? - self.best_bid()?)
    }

    /// Total resting quantity at `price` on the given side (0 if none).
    pub fn depth_at(&self, side: Side, price: Price) -> Qty {
        let book = match side {
            Side::Bid => &self.bids,
            Side::Ask => &self.asks,
        };
        book.get(&price).map_or(0, |lvl| lvl.total_qty)
    }
}

impl Engine for OrderBook {
    fn submit_limit_as(&mut self, owner: OrderId, side: Side, price: Price, qty: Qty) -> SubmitResult {
        OrderBook::submit_limit_as(self, owner, side, price, qty)
    }
    fn submit_market_as(&mut self, owner: OrderId, side: Side, qty: Qty) -> SubmitResult {
        OrderBook::submit_market_as(self, owner, side, qty)
    }
    fn set_self_trade(&mut self, policy: SelfTrade) {
        OrderBook::set_self_trade(self, policy)
    }
    fn cancel(&mut self, id: OrderId) -> bool {
        OrderBook::cancel(self, id)
    }
    fn amend(&mut self, id: OrderId, new_qty: Qty) -> bool {
        OrderBook::amend(self, id, new_qty)
    }
    fn best_bid(&self) -> Option<Price> {
        OrderBook::best_bid(self)
    }
    fn best_ask(&self) -> Option<Price> {
        OrderBook::best_ask(self)
    }
    fn spread(&self) -> Option<Price> {
        OrderBook::spread(self)
    }
    fn depth_at(&self, side: Side, price: Price) -> Qty {
        OrderBook::depth_at(self, side, price)
    }
    fn l2(&self, depth: usize) -> L2Snapshot {
        OrderBook::l2(self, depth)
    }
}

/// Which end of the price-ordered book is the "best" (most aggressive) level to
/// fill first for the incoming order.
enum Best {
    /// Lowest price first — used when matching a buy into the asks.
    Lowest,
    /// Highest price first — used when matching a sell into the bids.
    Highest,
}

/// The result of matching an aggressor against the book.
struct MatchOutcome {
    /// Quantity the aggressor couldn't fill.
    remaining: Qty,
    /// Whether self-trade prevention cancelled the aggressor's remainder (so it
    /// must not rest, even if `remaining > 0`).
    cancelled: bool,
}

/// Match an incoming order of `qty` against `map`, taking the best-priced level
/// first, until it is exhausted or no more levels cross.
///
/// `crosses(level_price)` decides whether the aggressor is still willing to
/// trade at that level's price. Once the best level no longer crosses, no worse
/// level can either, so we stop. Fully-filled maker orders are unlinked and
/// dropped from `locations`. When the aggressor would match an order it owns,
/// `stp` decides what happens instead of trading (see [`SelfTrade`]).
#[allow(clippy::too_many_arguments)]
fn match_into(
    map: &mut BTreeMap<Price, Level>,
    pool: &mut Pool,
    locations: &mut HashMap<OrderId, u32>,
    taker: OrderId,
    taker_owner: OrderId,
    stp: SelfTrade,
    mut qty: Qty,
    trades: &mut Vec<Trade>,
    best: Best,
    crosses: impl Fn(Price) -> bool,
) -> MatchOutcome {
    let mut cancelled = false;
    'outer: while qty > 0 {
        // Peek the best price without holding a borrow across the mutation.
        let peek = match best {
            Best::Lowest => map.keys().next(),
            Best::Highest => map.keys().next_back(),
        };
        let Some(&best_price) = peek.filter(|&&p| crosses(p)) else {
            break;
        };

        let level = map.get_mut(&best_price).expect("peeked price must exist");
        while qty > 0 {
            let Some(slot) = pool.head(level) else { break };
            let maker = pool.id_of(slot);

            // Self-trade prevention: the aggressor owns this resting order.
            if stp != SelfTrade::Allow
                && taker_owner != ANONYMOUS
                && pool.owner_of(slot) == taker_owner
            {
                let cancel_maker = matches!(stp, SelfTrade::CancelResting | SelfTrade::CancelBoth);
                if cancel_maker {
                    pool.unlink(level, slot);
                    locations.remove(&maker);
                }
                if matches!(stp, SelfTrade::CancelAggressor | SelfTrade::CancelBoth) {
                    cancelled = true;
                    if pool.is_empty(level) {
                        map.remove(&best_price);
                    }
                    break 'outer;
                }
                continue; // CancelResting: skip this maker, keep matching
            }

            let available = pool.qty_of(slot);
            let fill = qty.min(available);
            trades.push(Trade { taker, maker, price: best_price, qty: fill });
            qty -= fill;

            if fill == available {
                // Maker fully consumed: unlink and forget it.
                pool.unlink(level, slot);
                locations.remove(&maker);
            } else {
                pool.reduce(level, slot, fill);
            }
        }
        if pool.is_empty(level) {
            map.remove(&best_price);
        }
    }
    MatchOutcome { remaining: qty, cancelled }
}
