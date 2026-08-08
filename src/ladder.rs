//! A flat "price ladder" order book: the same matching engine as
//! [`crate::book::OrderBook`], but with the per-side `BTreeMap` replaced by a
//! single contiguous array indexed directly by price tick.
//!
//! # Why
//!
//! In the `BTreeMap` book, reaching a price level costs `O(log n)` and chases
//! pointers all over the heap. A real instrument trades in a *bounded* band of
//! prices, so we can preallocate one [`Level`] slot per tick and index it in
//! `O(1)` with cache-friendly, sequential memory. Finding the best price is
//! then just following a cursor.
//!
//! Bids and asks share one array. That's sound because resting bids are always
//! strictly below resting asks (anything crossing would have matched), so a
//! given tick is only ever one side at a time. Two cursors — [`best_bid`] and
//! [`best_ask`] — track the extremes; scanning to the next non-empty level when
//! one empties is the only linear cost, and it walks *dense* memory.
//!
//! Everything below the price index (the arena, the intrusive FIFO lists,
//! `O(1)` cancel) is shared verbatim with the `BTreeMap` engine via
//! [`crate::pool`] — the whole point of the exercise is that only the price
//! container differs.
//!
//! [`best_bid`]: LadderBook::best_bid
//! [`best_ask`]: LadderBook::best_ask

use std::collections::HashMap;

use crate::pool::{Level, Pool};
use crate::types::{OrderId, Price, Qty, Side, SubmitResult, Trade};

/// Cursor sentinel meaning "no populated level on this side".
const NONE: u32 = u32::MAX;

/// A price-time-priority order book backed by a flat array of price levels.
pub struct LadderBook {
    /// Lowest price the ladder can represent; `levels[0]` is this tick.
    min_price: Price,
    /// One [`Level`] per tick in `[min_price, max_price]`.
    levels: Box<[Level]>,
    /// Shared arena of resting-order nodes.
    pool: Pool,
    /// `OrderId -> arena slot`, for `O(1)` cancellation.
    locations: HashMap<OrderId, u32>,
    /// Index of the highest populated bid level, or [`NONE`].
    best_bid: u32,
    /// Index of the lowest populated ask level, or [`NONE`].
    best_ask: u32,
    /// Source of monotonically-increasing order ids.
    next_id: OrderId,
}

impl LadderBook {
    /// Create an empty book covering the inclusive tick range
    /// `[min_price, max_price]`. Orders outside this band will panic.
    pub fn new(min_price: Price, max_price: Price) -> Self {
        assert!(max_price >= min_price, "empty price range");
        let ticks = (max_price - min_price + 1) as usize;
        Self {
            min_price,
            levels: vec![Level::empty(); ticks].into_boxed_slice(),
            pool: Pool::new(),
            locations: HashMap::new(),
            best_bid: NONE,
            best_ask: NONE,
            next_id: 0,
        }
    }

    /// Convert a price to its array index, panicking if out of the ladder's band.
    #[inline]
    fn index(&self, price: Price) -> u32 {
        assert!(
            price >= self.min_price && (price - self.min_price) < self.levels.len() as Price,
            "price {price} outside ladder range",
        );
        (price - self.min_price) as u32
    }

    /// Submit a limit order (see [`crate::book::OrderBook::submit_limit`]).
    pub fn submit_limit(&mut self, side: Side, price: Price, qty: Qty) -> SubmitResult {
        assert!(qty > 0, "cannot submit a zero-quantity order");
        let _ = self.index(price); // bounds-check up front

        let id = self.next_id;
        self.next_id += 1;

        let mut trades = Vec::new();
        let buy = matches!(side, Side::Bid);
        let remaining = self.take_liquidity(id, price, qty, buy, &mut trades);

        if remaining > 0 {
            let idx = self.index(price);
            let slot = self.pool.push_back(&mut self.levels[idx as usize], id, price, side, remaining);
            self.locations.insert(id, slot);
            match side {
                Side::Bid if self.best_bid == NONE || idx > self.best_bid => self.best_bid = idx,
                Side::Ask if self.best_ask == NONE || idx < self.best_ask => self.best_ask = idx,
                _ => {}
            }
        }

        SubmitResult { id, trades, resting: remaining }
    }

    /// Match `qty` against the opposite side, walking from the best cursor
    /// toward `limit`. Returns the unfilled remainder and advances the cursor
    /// past any levels it empties.
    fn take_liquidity(
        &mut self,
        taker: OrderId,
        limit: Price,
        mut qty: Qty,
        buy: bool,
        trades: &mut Vec<Trade>,
    ) -> Qty {
        while qty > 0 {
            let i = if buy { self.best_ask } else { self.best_bid };
            if i == NONE {
                break;
            }
            let price_i = self.min_price + i as Price;
            let crosses = if buy { price_i <= limit } else { price_i >= limit };
            if !crosses {
                break;
            }

            // Fill this level FIFO until it's empty or the taker is done.
            loop {
                let level = &mut self.levels[i as usize];
                let Some(slot) = self.pool.head(level) else { break };
                let maker = self.pool.id_of(slot);
                let available = self.pool.qty_of(slot);
                let fill = qty.min(available);

                trades.push(Trade { taker, maker, price: price_i, qty: fill });
                qty -= fill;

                if fill == available {
                    self.pool.unlink(level, slot);
                    self.locations.remove(&maker);
                } else {
                    self.pool.reduce(level, slot, fill);
                }
                if qty == 0 {
                    break;
                }
            }

            if self.pool.is_empty(&self.levels[i as usize]) {
                let next = if buy {
                    next_up(&self.levels, &self.pool, i)
                } else {
                    next_down(&self.levels, &self.pool, i)
                };
                if buy {
                    self.best_ask = next;
                } else {
                    self.best_bid = next;
                }
            } else {
                break; // taker exhausted, level still has depth
            }
        }
        qty
    }

    /// Cancel a resting order by id (see [`crate::book::OrderBook::cancel`]).
    pub fn cancel(&mut self, id: OrderId) -> bool {
        let Some(slot) = self.locations.remove(&id) else {
            return false;
        };
        let (price, side) = self.pool.location(slot);
        let idx = self.index(price);
        self.pool.unlink(&mut self.levels[idx as usize], slot);

        // If we just emptied the best level on its side, walk the cursor inward.
        if self.pool.is_empty(&self.levels[idx as usize]) {
            match side {
                Side::Bid if idx == self.best_bid => {
                    self.best_bid = next_down(&self.levels, &self.pool, idx);
                }
                Side::Ask if idx == self.best_ask => {
                    self.best_ask = next_up(&self.levels, &self.pool, idx);
                }
                _ => {}
            }
        }
        true
    }

    /// The highest resting bid price, if any.
    pub fn best_bid(&self) -> Option<Price> {
        (self.best_bid != NONE).then(|| self.min_price + self.best_bid as Price)
    }

    /// The lowest resting ask price, if any.
    pub fn best_ask(&self) -> Option<Price> {
        (self.best_ask != NONE).then(|| self.min_price + self.best_ask as Price)
    }

    /// The gap between best ask and best bid, if both sides are populated.
    pub fn spread(&self) -> Option<Price> {
        Some(self.best_ask()? - self.best_bid()?)
    }

    /// Total resting quantity at `price` on the given side (0 if none, or if
    /// the price is outside the ladder's range).
    pub fn depth_at(&self, _side: Side, price: Price) -> Qty {
        if price < self.min_price {
            return 0;
        }
        self.levels
            .get((price - self.min_price) as usize)
            .map_or(0, |lvl| lvl.total_qty)
    }
}

/// First non-empty level strictly above `from`, or [`NONE`].
fn next_up(levels: &[Level], pool: &Pool, from: u32) -> u32 {
    ((from as usize + 1)..levels.len())
        .find(|&j| !pool.is_empty(&levels[j]))
        .map_or(NONE, |j| j as u32)
}

/// First non-empty level strictly below `from`, or [`NONE`].
fn next_down(levels: &[Level], pool: &Pool, from: u32) -> u32 {
    (0..from as usize)
        .rev()
        .find(|&j| !pool.is_empty(&levels[j]))
        .map_or(NONE, |j| j as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> LadderBook {
        LadderBook::new(90, 110)
    }

    #[test]
    fn walks_levels_cheapest_first_then_rests() {
        let mut b = book();
        let a1 = b.submit_limit(Side::Ask, 100, 2).id;
        let a2 = b.submit_limit(Side::Ask, 101, 2).id;

        let res = b.submit_limit(Side::Bid, 101, 5);
        assert_eq!(
            res.trades,
            vec![
                Trade { taker: res.id, maker: a1, price: 100, qty: 2 },
                Trade { taker: res.id, maker: a2, price: 101, qty: 2 },
            ]
        );
        assert_eq!(res.resting, 1);
        assert_eq!(b.best_bid(), Some(101));
        assert_eq!(b.best_ask(), None);
    }

    #[test]
    fn sell_hits_highest_bid_first() {
        let mut b = book();
        let low = b.submit_limit(Side::Bid, 99, 2).id;
        let high = b.submit_limit(Side::Bid, 100, 2).id;

        let res = b.submit_limit(Side::Ask, 99, 3);
        assert_eq!(
            res.trades,
            vec![
                Trade { taker: res.id, maker: high, price: 100, qty: 2 },
                Trade { taker: res.id, maker: low, price: 99, qty: 1 },
            ]
        );
        assert_eq!(b.depth_at(Side::Bid, 99), 1);
    }

    #[test]
    fn cancel_advances_cursor() {
        let mut b = book();
        let top = b.submit_limit(Side::Bid, 100, 1).id;
        b.submit_limit(Side::Bid, 99, 1);
        assert_eq!(b.best_bid(), Some(100));

        assert!(b.cancel(top));
        assert_eq!(b.best_bid(), Some(99)); // cursor stepped down
    }

    #[test]
    fn cancel_middle_preserves_fifo() {
        let mut b = book();
        let a = b.submit_limit(Side::Bid, 100, 1).id;
        let mid = b.submit_limit(Side::Bid, 100, 1).id;
        let c = b.submit_limit(Side::Bid, 100, 1).id;
        assert!(b.cancel(mid));

        let res = b.submit_limit(Side::Ask, 100, 2);
        assert_eq!(
            res.trades,
            vec![
                Trade { taker: res.id, maker: a, price: 100, qty: 1 },
                Trade { taker: res.id, maker: c, price: 100, qty: 1 },
            ]
        );
    }
}
