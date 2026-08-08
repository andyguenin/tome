//! A tiny demo that pushes a handful of orders through the book and prints the
//! resulting trades, an L2 snapshot, and a market-order sweep. Run with
//! `cargo run`.

use tome::{L2Snapshot, OrderBook, Side};

fn print_l2(book: &OrderBook) {
    let L2Snapshot { bids, asks } = book.l2(3);
    println!("  asks (low→high):");
    for lvl in asks.iter().rev() {
        println!("    {:>4} @ {}", lvl.qty, lvl.price);
    }
    println!("  bids (high→low):");
    for lvl in &bids {
        println!("    {:>4} @ {}", lvl.qty, lvl.price);
    }
}

fn main() {
    let mut book = OrderBook::new();

    // Build a resting book across a few levels on each side.
    book.submit_limit(Side::Ask, 102, 5);
    book.submit_limit(Side::Ask, 101, 3);
    book.submit_limit(Side::Bid, 99, 4);
    book.submit_limit(Side::Bid, 98, 6);

    println!("resting book (L2, depth 3):");
    print_l2(&book);
    println!("  best bid / ask: {:?} / {:?}", book.best_bid(), book.best_ask());
    println!("  spread: {:?}", book.spread());

    // A limit buy for 6 @ up to 102: sweeps 3@101 and 3@102, nothing rests.
    println!("\nLIMIT BUY 6 @ 102 ...");
    for t in &book.submit_limit(Side::Bid, 102, 6).trades {
        println!("  fill: {} @ {} (maker #{})", t.qty, t.price, t.maker);
    }

    // A market sell for 5: hits the bids from the top down, no price limit.
    println!("\nMARKET SELL 5 ...");
    let res = book.submit_market(Side::Ask, 5);
    for t in &res.trades {
        println!("  fill: {} @ {} (maker #{})", t.qty, t.price, t.maker);
    }
    if res.resting > 0 {
        println!("  {} unfilled (book exhausted)", res.resting);
    }

    println!("\nbook after:");
    print_l2(&book);
}
