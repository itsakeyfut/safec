//! A set of indices below a length fixed when it is made, a bit each.

/// A set of indices below `len`, a bit each.
///
/// **What the memory check's square tables are made of**: [`Held::sites`],
/// [`Held::writes_to`], [`Known::inside`] and [`Known::inside_locals`] each
/// hold a bit per local for every local, and as `Vec<bool>` they spent a byte
/// on each. Hand-written because this crate takes no dependencies
/// (ADR-0011), and dense rather than a sorted list of indices so that its size
/// is set by the locals alone, which is what lets the check's bound on
/// `blocks * locals * locals` bound its memory.
///
/// **An index at or past `len` panics**, as indexing the `Vec<bool>` did. The
/// last word has room past `len`, and answering `false` there would be a site
/// read as holding nothing that the check never looked at.
///
/// **The bits past `len` are always clear**, because `PartialEq` compares the
/// words and the solver stops when a value compares equal to the last one. A
/// stray bit there would keep two equal values apart for ever. Nothing here
/// sets every bit, so it holds by construction.
///
/// [`Held::sites`]: super::parts::Held::sites
/// [`Held::writes_to`]: super::parts::Held::writes_to
/// [`Known::inside`]: super::known::Known::inside
/// [`Known::inside_locals`]: super::known::Known::inside_locals
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Bits {
    len: usize,
    words: Vec<u64>,
}

impl Bits {
    /// The empty set, below `len`.
    pub(super) fn new(len: usize) -> Self {
        Bits {
            len,
            words: vec![0; len.div_ceil(64)],
        }
    }

    /// Whether `index` is in the set.
    pub(super) fn get(&self, index: usize) -> bool {
        self.check(index);
        self.words[index / 64] & (1 << (index % 64)) != 0
    }

    /// Put `index` in the set.
    pub(super) fn set(&mut self, index: usize) {
        self.check(index);
        self.words[index / 64] |= 1 << (index % 64);
    }

    /// Take `index` out of the set.
    pub(super) fn remove(&mut self, index: usize) {
        self.check(index);
        self.words[index / 64] &= !(1 << (index % 64));
    }

    /// Put every index of `other` in the set, and say whether that added any.
    pub(super) fn union_with(&mut self, other: &Bits) -> bool {
        assert_eq!(self.len, other.len, "a union of sets of two lengths");
        let mut grew = false;
        for (here, there) in self.words.iter_mut().zip(&other.words) {
            grew |= *there & !*here != 0;
            *here |= *there;
        }
        grew
    }

    /// Every index in the set, in order.
    pub(super) fn ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(at, &word)| {
            let mut rest = word;
            std::iter::from_fn(move || {
                if rest == 0 {
                    return None;
                }
                let bit = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                Some(at * 64 + bit)
            })
        })
    }

    /// Whether the set holds anything.
    pub(super) fn any(&self) -> bool {
        self.words.iter().any(|&word| word != 0)
    }

    /// Empty the set.
    pub(super) fn clear_all(&mut self) {
        self.words.fill(0);
    }

    fn check(&self, index: usize) {
        assert!(
            index < self.len,
            "index {index} is past a set of {} indices",
            self.len
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Bits;

    /// Mutation: drop the length check. The index lands in the last word's
    /// spare room and answers `false`, and this does not panic.
    #[test]
    #[should_panic(expected = "index 70 is past a set of 70 indices")]
    fn an_index_at_the_length_panics() {
        Bits::new(70).get(70);
    }

    /// Mutation: index the word by `index % 64`, or the bit by `index / 64`,
    /// or have `any` read only the first word. This fails.
    #[test]
    fn a_set_index_is_read_back_and_its_neighbours_across_a_word_are_not() {
        let mut bits = Bits::new(130);
        bits.set(64);
        assert!(bits.get(64));
        assert!(bits.any());
        assert!(!bits.get(63));
        assert!(!bits.get(65));
        assert!(!bits.get(0));
        assert!(!bits.get(128));
        bits.remove(64);
        assert!(!bits.get(64));
        assert!(!bits.any());
    }

    /// Mutation: answer `true` whatever the union added, or `false`. This
    /// fails.
    #[test]
    fn a_union_says_whether_it_added_anything() {
        let mut here = Bits::new(100);
        let mut there = Bits::new(100);
        there.set(99);
        assert!(here.union_with(&there));
        assert!(here.get(99));
        assert!(!here.union_with(&there));
        here.set(3);
        assert!(!here.union_with(&there));
    }

    /// Mutation: leave out the word's offset. This fails. Leave out the
    /// clearing of the lowest bit, and it never stops: it ran out of memory
    /// after most of a minute.
    #[test]
    fn the_ones_are_every_index_in_the_set_in_order() {
        let mut bits = Bits::new(131);
        for index in [0, 5, 63, 64, 127, 130] {
            bits.set(index);
        }
        assert_eq!(bits.ones().collect::<Vec<_>>(), [0, 5, 63, 64, 127, 130]);
        bits.clear_all();
        assert_eq!(bits.ones().next(), None);
    }

    /// Two sets built apart compare equal when they hold the same indices,
    /// which is what the solver's fixed point is read from.
    ///
    /// Mutation: make `remove` leave the bit. This fails.
    #[test]
    fn two_sets_of_the_same_indices_are_equal_and_one_apart_are_not() {
        let mut here = Bits::new(70);
        here.set(69);
        here.set(2);
        here.remove(2);
        let mut there = Bits::new(70);
        there.set(69);
        assert!(here == there);
        there.set(1);
        assert!(here != there);
    }
}
