//! `tome` — a central limit order book (CLOB) simulator.
//!
//! The goal is a fast, correct price-time-priority matching engine, built up
//! from a clean baseline. See [`book::OrderBook`] for the entry point.

mod bitset;
pub mod book;
pub mod ladder;
mod pool;
pub mod sim;
pub mod types;

pub use book::OrderBook;
pub use ladder::LadderBook;
pub use types::{OrderId, Price, Qty, Side, SubmitResult, Trade};

#[cfg(test)]
mod tests {
    use super::*;

    /// A sell resting above a buy that isn't willing to reach it: no trade,
    /// both rest, and there's a spread.
    #[test]
    fn no_cross_rests_both_sides() {
        let mut book = OrderBook::new();
        let ask = book.submit_limit(Side::Ask, 101, 5);
        assert!(ask.trades.is_empty());
        assert_eq!(ask.resting, 5);

        let bid = book.submit_limit(Side::Bid, 100, 5);
        assert!(bid.trades.is_empty());
        assert_eq!(bid.resting, 5);

        assert_eq!(book.best_bid(), Some(100));
        assert_eq!(book.best_ask(), Some(101));
        assert_eq!(book.spread(), Some(1));
    }

    /// A buy that exactly meets a resting ask fully trades; nothing rests.
    #[test]
    fn exact_cross_fully_fills() {
        let mut book = OrderBook::new();
        let maker = book.submit_limit(Side::Ask, 100, 5).id;
        let taker = book.submit_limit(Side::Bid, 100, 5);

        assert_eq!(taker.resting, 0);
        assert_eq!(taker.trades, vec![Trade { taker: taker.id, maker, price: 100, qty: 5 }]);
        // Book is now empty on both sides.
        assert_eq!(book.best_bid(), None);
        assert_eq!(book.best_ask(), None);
    }

    /// An aggressive buy walks up multiple ask levels, cheapest first, and
    /// rests the leftover as a bid.
    #[test]
    fn walks_levels_cheapest_first_then_rests() {
        let mut book = OrderBook::new();
        let a1 = book.submit_limit(Side::Ask, 100, 2).id;
        let a2 = book.submit_limit(Side::Ask, 101, 2).id;

        // Wants 5 @ up to 101; takes 2@100 then 2@101, 1 rests as a bid.
        let res = book.submit_limit(Side::Bid, 101, 5);
        assert_eq!(
            res.trades,
            vec![
                Trade { taker: res.id, maker: a1, price: 100, qty: 2 },
                Trade { taker: res.id, maker: a2, price: 101, qty: 2 },
            ]
        );
        assert_eq!(res.resting, 1);
        assert_eq!(book.best_bid(), Some(101));
        assert_eq!(book.best_ask(), None);
        assert_eq!(book.depth_at(Side::Bid, 101), 1);
    }

    /// Two makers at the same price fill in arrival order (time priority).
    #[test]
    fn fifo_within_a_price_level() {
        let mut book = OrderBook::new();
        let first = book.submit_limit(Side::Bid, 100, 3).id;
        let second = book.submit_limit(Side::Bid, 100, 3).id;

        // A sell of 4 should exhaust `first` (3) then take 1 from `second`.
        let res = book.submit_limit(Side::Ask, 100, 4);
        assert_eq!(
            res.trades,
            vec![
                Trade { taker: res.id, maker: first, price: 100, qty: 3 },
                Trade { taker: res.id, maker: second, price: 100, qty: 1 },
            ]
        );
        assert_eq!(res.resting, 0);
        assert_eq!(book.depth_at(Side::Bid, 100), 2); // 2 left on `second`
    }

    /// A sell sweeps the bids from the highest price down.
    #[test]
    fn sell_hits_highest_bid_first() {
        let mut book = OrderBook::new();
        let low = book.submit_limit(Side::Bid, 99, 2).id;
        let high = book.submit_limit(Side::Bid, 100, 2).id;

        let res = book.submit_limit(Side::Ask, 99, 3);
        assert_eq!(
            res.trades,
            vec![
                Trade { taker: res.id, maker: high, price: 100, qty: 2 },
                Trade { taker: res.id, maker: low, price: 99, qty: 1 },
            ]
        );
        assert_eq!(res.resting, 0);
        assert_eq!(book.depth_at(Side::Bid, 99), 1);
    }

    /// Cancelling a resting order removes it and empties the (now-bare) level.
    #[test]
    fn cancel_removes_resting_order() {
        let mut book = OrderBook::new();
        let id = book.submit_limit(Side::Bid, 100, 5).id;
        assert_eq!(book.depth_at(Side::Bid, 100), 5);

        assert!(book.cancel(id));
        assert_eq!(book.depth_at(Side::Bid, 100), 0);
        assert_eq!(book.best_bid(), None);

        // A second cancel is a no-op.
        assert!(!book.cancel(id));
    }

    /// Cancelling the *middle* of a FIFO level preserves the order of the rest,
    /// so the survivors still fill in arrival order.
    #[test]
    fn cancel_middle_preserves_fifo() {
        let mut book = OrderBook::new();
        let a = book.submit_limit(Side::Bid, 100, 1).id;
        let b = book.submit_limit(Side::Bid, 100, 1).id;
        let c = book.submit_limit(Side::Bid, 100, 1).id;

        assert!(book.cancel(b)); // pull the middle order
        assert_eq!(book.depth_at(Side::Bid, 100), 2);

        // A sell of 2 should hit `a` then `c`, skipping cancelled `b`.
        let res = book.submit_limit(Side::Ask, 100, 2);
        assert_eq!(
            res.trades,
            vec![
                Trade { taker: res.id, maker: a, price: 100, qty: 1 },
                Trade { taker: res.id, maker: c, price: 100, qty: 1 },
            ]
        );
    }

    /// A fully-filled order can't be cancelled — its id is no longer resting.
    #[test]
    fn cannot_cancel_filled_order() {
        let mut book = OrderBook::new();
        let maker = book.submit_limit(Side::Ask, 100, 2).id;
        book.submit_limit(Side::Bid, 100, 2); // fully consumes the maker
        assert!(!book.cancel(maker));
    }

    /// Freed arena slots are recycled: churn many orders, then confirm the book
    /// still matches correctly (regression guard for the free list).
    #[test]
    fn arena_slots_recycle() {
        let mut book = OrderBook::new();
        for _ in 0..1000 {
            let id = book.submit_limit(Side::Bid, 50, 1).id;
            assert!(book.cancel(id));
        }
        assert_eq!(book.best_bid(), None);

        // Book is usable after all that churn.
        let maker = book.submit_limit(Side::Ask, 100, 3).id;
        let res = book.submit_limit(Side::Bid, 100, 3);
        assert_eq!(res.trades, vec![Trade { taker: res.id, maker, price: 100, qty: 3 }]);
    }
}
