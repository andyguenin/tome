//! An arena of resting-order nodes, with each price level threaded through it
//! as an intrusive doubly-linked FIFO list.
//!
//! # Why this shape
//!
//! The baseline used a `VecDeque` per level. That's fine for matching (we only
//! ever touch the front) but cancellation can hit *any* order, and removing
//! from the middle of a `VecDeque` is `O(n)`. An intrusive linked list removes
//! a known node in `O(1)` — we just splice its neighbours together.
//!
//! "Intrusive" means the `prev`/`next` links live *inside* the order node
//! rather than in separate list-cell allocations. All nodes live in one `Vec`
//! (the arena / slab); links are `u32` indices into it, not pointers. That
//! keeps nodes dense and cache-friendly and sidesteps the borrow-checker pain
//! of a pointer-linked list. Freed slots are recycled through a free list.

use crate::types::{OrderId, Price, Qty, Side};

/// Sentinel index meaning "no node" (list end, or empty list).
const NONE: u32 = u32::MAX;

/// One resting order, plus its links within its price level's list.
struct Node {
    id: OrderId,
    qty: Qty,
    /// The order's price and side — enough to locate its level on cancel.
    price: Price,
    side: Side,
    /// The participant that owns this order, for self-trade prevention.
    owner: OrderId,
    prev: u32,
    next: u32,
}

/// The FIFO list of resting orders at a single price: head/tail indices into
/// the [`Pool`], plus a cached total so depth queries are `O(1)`.
///
/// Not `Default`-derived: an empty level's links must be [`NONE`], not `0`.
#[derive(Clone, Copy)]
pub(crate) struct Level {
    head: u32,
    tail: u32,
    pub total_qty: Qty,
}

impl Level {
    /// A fresh, empty level.
    pub fn empty() -> Self {
        Self { head: NONE, tail: NONE, total_qty: 0 }
    }
}

/// The arena of order nodes with a free list for slot reuse.
pub(crate) struct Pool {
    nodes: Vec<Node>,
    free: Vec<u32>,
}

impl Pool {
    /// An empty pool.
    pub fn new() -> Self {
        Self { nodes: Vec::new(), free: Vec::new() }
    }

    /// Claim a slot for `node`, reusing a freed one if available.
    fn alloc(&mut self, node: Node) -> u32 {
        if let Some(idx) = self.free.pop() {
            self.nodes[idx as usize] = node;
            idx
        } else {
            let idx = self.nodes.len() as u32;
            self.nodes.push(node);
            idx
        }
    }

    /// The id of the order in slot `idx`.
    pub fn id_of(&self, idx: u32) -> OrderId {
        self.nodes[idx as usize].id
    }

    /// The remaining quantity of the order in slot `idx`.
    pub fn qty_of(&self, idx: u32) -> Qty {
        self.nodes[idx as usize].qty
    }

    /// The owning participant of the order in slot `idx`.
    pub fn owner_of(&self, idx: u32) -> OrderId {
        self.nodes[idx as usize].owner
    }

    /// Where the order in slot `idx` rests: its price and side.
    pub fn location(&self, idx: u32) -> (Price, Side) {
        let n = &self.nodes[idx as usize];
        (n.price, n.side)
    }

    /// The slot at the front of `level` (the next to be filled), if any.
    pub fn head(&self, level: &Level) -> Option<u32> {
        (level.head != NONE).then_some(level.head)
    }

    /// Whether `level` has no resting orders left.
    pub fn is_empty(&self, level: &Level) -> bool {
        level.head == NONE
    }

    /// Append a new resting order at the tail of `level`. Returns its slot.
    #[allow(clippy::too_many_arguments)]
    pub fn push_back(
        &mut self,
        level: &mut Level,
        id: OrderId,
        owner: OrderId,
        price: Price,
        side: Side,
        qty: Qty,
    ) -> u32 {
        let idx = self.alloc(Node { id, qty, price, side, owner, prev: level.tail, next: NONE });
        if level.tail != NONE {
            self.nodes[level.tail as usize].next = idx;
        } else {
            level.head = idx;
        }
        level.tail = idx;
        level.total_qty += qty;
        idx
    }

    /// Reduce slot `idx`'s quantity by `by`, keeping the node in place. Updates
    /// the level's cached total. Used for partial fills.
    pub fn reduce(&mut self, level: &mut Level, idx: u32, by: Qty) {
        self.nodes[idx as usize].qty -= by;
        level.total_qty -= by;
    }

    /// Splice slot `idx` out of `level`'s list and return it to the free list.
    /// The level's cached total is decremented by whatever quantity remained.
    /// `O(1)` regardless of where in the list the node sits.
    pub fn unlink(&mut self, level: &mut Level, idx: u32) {
        let Node { prev, next, qty, .. } = self.nodes[idx as usize];

        if prev != NONE {
            self.nodes[prev as usize].next = next;
        } else {
            level.head = next;
        }
        if next != NONE {
            self.nodes[next as usize].prev = prev;
        } else {
            level.tail = prev;
        }

        level.total_qty -= qty;
        self.free.push(idx);
    }
}
