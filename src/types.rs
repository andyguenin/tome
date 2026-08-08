//! Core value types for the order book.
//!
//! Prices and quantities are integers on purpose — floating point has no place
//! in a matching engine. `Price` is a count of ticks (the exchange's minimum
//! price increment), so equality and ordering are exact and cheap.

/// A resting order's unique, monotonically-increasing identifier.
pub type OrderId = u64;

/// A price expressed as an integer number of ticks. Higher = more expensive.
pub type Price = u64;

/// A quantity of the instrument, in whole lots.
pub type Qty = u64;

/// Which side of the book an order sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// A buy order. Rests among the bids; matches against asks.
    Bid,
    /// A sell order. Rests among the asks; matches against bids.
    Ask,
}

impl Side {
    /// The opposite side — the book an aggressive order of this side matches into.
    #[inline]
    pub fn opposite(self) -> Side {
        match self {
            Side::Bid => Side::Ask,
            Side::Ask => Side::Bid,
        }
    }
}

/// A single execution: one taker order crossing against one resting maker order.
///
/// Trades always print at the *maker's* price (price-time priority means the
/// resting order set the price; the aggressor accepts it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trade {
    /// The incoming aggressive order that caused the match.
    pub taker: OrderId,
    /// The resting order that was hit.
    pub maker: OrderId,
    /// Execution price, in ticks (the maker's resting price).
    pub price: Price,
    /// Quantity executed in this fill.
    pub qty: Qty,
}

/// The outcome of submitting an order: what it traded, and what (if anything)
/// remains resting on the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitResult {
    /// The id assigned to the submitted order.
    pub id: OrderId,
    /// Executions generated, in the order they occurred.
    pub trades: Vec<Trade>,
    /// For a limit order, quantity left resting on the book (0 if fully filled).
    /// For a market order, quantity left *unfilled* — a market order never
    /// rests, so any remainder after exhausting the book is simply cancelled.
    pub resting: Qty,
}

/// One aggregated price level in an L2 (market-by-price) snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level2 {
    /// The level's price, in ticks.
    pub price: Price,
    /// Total resting quantity across all orders at this price.
    pub qty: Qty,
}

/// A depth-limited snapshot of the book: the top price levels per side.
///
/// `bids` run from best (highest) downward; `asks` from best (lowest) upward.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct L2Snapshot {
    pub bids: Vec<Level2>,
    pub asks: Vec<Level2>,
}
