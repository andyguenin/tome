//! A hierarchical occupancy bitmap over the price ladder's levels.
//!
//! # The problem it solves
//!
//! When a price level empties, the ladder must find the *next* non-empty level
//! to move its best-price cursor to. A plain linear scan is `O(width)` and, on
//! a sparse book, walks a lot of empty ticks. That scan was the ladder's
//! remaining cost on deep books.
//!
//! # The structure
//!
//! One bit per level (`words`), set when that level has resting orders, plus a
//! coarser *summary* layer with one bit per 64-bit word of `words`, set when
//! that word has any bits. To find the next occupied level we mask the current
//! word and take [`u64::trailing_zeros`]; if the word is empty we consult the
//! summary to jump straight to the next non-empty word — no empty ticks
//! visited. For a ladder up to `64 * 64 = 4096` levels the summary is a single
//! word, so a lookup is a small constant number of instructions; wider ladders
//! scan only summary words (`width / 4096`), still far below a raw scan.

/// A two-level "find next/previous set bit" bitmap.
pub(crate) struct LevelBitset {
    /// Number of representable levels.
    nbits: usize,
    /// Bit `i` set == level `i` is occupied.
    words: Box<[u64]>,
    /// Bit `w` set == `words[w] != 0`.
    summary: Box<[u64]>,
}

impl LevelBitset {
    /// A bitmap covering `nbits` levels, all initially empty.
    pub fn new(nbits: usize) -> Self {
        let words = nbits.div_ceil(64).max(1);
        let summary = words.div_ceil(64);
        Self {
            nbits,
            words: vec![0; words].into_boxed_slice(),
            summary: vec![0; summary].into_boxed_slice(),
        }
    }

    /// Mark level `i` occupied. Idempotent.
    pub fn set(&mut self, i: usize) {
        let (w, b) = (i / 64, i % 64);
        self.words[w] |= 1 << b;
        self.summary[w / 64] |= 1 << (w % 64);
    }

    /// Mark level `i` empty. Idempotent; clears the summary bit if the word is
    /// now entirely empty.
    pub fn clear(&mut self, i: usize) {
        let (w, b) = (i / 64, i % 64);
        self.words[w] &= !(1 << b);
        if self.words[w] == 0 {
            self.summary[w / 64] &= !(1 << (w % 64));
        }
    }

    /// The smallest occupied level `>= from`, if any.
    pub fn next_set_from(&self, from: usize) -> Option<usize> {
        if from >= self.nbits {
            return None;
        }
        let (w, b) = (from / 64, from % 64);
        let masked = self.words[w] & (u64::MAX << b);
        if masked != 0 {
            return Some(w * 64 + masked.trailing_zeros() as usize);
        }
        let nw = self.next_word_from(w + 1)?;
        Some(nw * 64 + self.words[nw].trailing_zeros() as usize)
    }

    /// The largest occupied level `<= from`, if any.
    pub fn prev_set_from(&self, from: usize) -> Option<usize> {
        if from >= self.nbits {
            return None;
        }
        let (w, b) = (from / 64, from % 64);
        let mask = lo_mask_inclusive(b);
        let masked = self.words[w] & mask;
        if masked != 0 {
            return Some(w * 64 + 63 - masked.leading_zeros() as usize);
        }
        if w == 0 {
            return None;
        }
        let pw = self.prev_word_from(w - 1)?;
        Some(pw * 64 + 63 - self.words[pw].leading_zeros() as usize)
    }

    /// First non-empty word index `>= from`, via the summary layer.
    fn next_word_from(&self, from: usize) -> Option<usize> {
        if from >= self.words.len() {
            return None;
        }
        let (mut sw, sb) = (from / 64, from % 64);
        let masked = self.summary[sw] & (u64::MAX << sb);
        if masked != 0 {
            return Some(sw * 64 + masked.trailing_zeros() as usize);
        }
        sw += 1;
        while sw < self.summary.len() {
            if self.summary[sw] != 0 {
                return Some(sw * 64 + self.summary[sw].trailing_zeros() as usize);
            }
            sw += 1;
        }
        None
    }

    /// Last non-empty word index `<= from`, via the summary layer.
    fn prev_word_from(&self, from: usize) -> Option<usize> {
        let (mut sw, sb) = (from / 64, from % 64);
        let masked = self.summary[sw] & lo_mask_inclusive(sb);
        if masked != 0 {
            return Some(sw * 64 + 63 - masked.leading_zeros() as usize);
        }
        while sw > 0 {
            sw -= 1;
            if self.summary[sw] != 0 {
                return Some(sw * 64 + 63 - self.summary[sw].leading_zeros() as usize);
            }
        }
        None
    }
}

/// A mask of the low bits `0..=b` (bit `b` included).
#[inline]
fn lo_mask_inclusive(b: usize) -> u64 {
    if b == 63 {
        u64::MAX
    } else {
        (1 << (b + 1)) - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_and_prev_within_a_word() {
        let mut bs = LevelBitset::new(64);
        bs.set(3);
        bs.set(10);
        assert_eq!(bs.next_set_from(0), Some(3));
        assert_eq!(bs.next_set_from(4), Some(10));
        assert_eq!(bs.next_set_from(11), None);
        assert_eq!(bs.prev_set_from(63), Some(10));
        assert_eq!(bs.prev_set_from(9), Some(3));
        assert_eq!(bs.prev_set_from(2), None);
    }

    #[test]
    fn crosses_word_boundaries() {
        // 200 levels spans 4 words; the summary layer is exercised.
        let mut bs = LevelBitset::new(200);
        bs.set(1);
        bs.set(65); // second word
        bs.set(130); // third word
        assert_eq!(bs.next_set_from(2), Some(65));
        assert_eq!(bs.next_set_from(66), Some(130));
        assert_eq!(bs.next_set_from(131), None);
        assert_eq!(bs.prev_set_from(129), Some(65));
        assert_eq!(bs.prev_set_from(64), Some(1));
    }

    #[test]
    fn set_then_clear_is_gone() {
        let mut bs = LevelBitset::new(128);
        bs.set(70);
        assert_eq!(bs.next_set_from(0), Some(70));
        bs.clear(70);
        assert_eq!(bs.next_set_from(0), None);
        assert_eq!(bs.prev_set_from(127), None);
    }

    #[test]
    fn exact_position_is_inclusive() {
        let mut bs = LevelBitset::new(128);
        bs.set(42);
        assert_eq!(bs.next_set_from(42), Some(42));
        assert_eq!(bs.prev_set_from(42), Some(42));
    }
}
