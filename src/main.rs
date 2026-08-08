//! A tiny demo that pushes a handful of orders through the book and prints the
//! resulting trades and book state. Run with `cargo run`.

use tome::{OrderBook, Side};

fn main() {
    let mut book = OrderBook::new();

    // Build a resting book: two asks and one bid, none of which cross.
    book.submit_limit(Side::Ask, 101, 3);
    book.submit_limit(Side::Ask, 102, 5);
    book.submit_limit(Side::Bid, 99, 4);

    println!("resting book:");
    println!("  best bid: {:?}", book.best_bid());
    println!("  best ask: {:?}", book.best_ask());
    println!("  spread:   {:?}", book.spread());

    // An aggressive buy for 6 @ up to 102: sweeps 3@101 and 3@102.
    println!("\nsubmitting BUY 6 @ 102 ...");
    let res = book.submit_limit(Side::Bid, 102, 6);
    for t in &res.trades {
        println!("  trade: taker #{} took {} @ {} from maker #{}", t.taker, t.qty, t.price, t.maker);
    }
    println!("  {} left unfilled (rests as a bid)", res.resting);

    println!("\nbook after:");
    println!("  best bid: {:?}", book.best_bid());
    println!("  best ask: {:?}  (2 left resting @ 102)", book.best_ask());
}
