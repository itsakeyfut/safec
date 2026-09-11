//! The machine a translation unit is for.
//!
//! `docs/architecture.md` opens its output section with "Output depends on the
//! target, never on the host", and its table says the Safety IR joins that list
//! "once type widths reach them". This is where they reach it. See [ADR-0013].
//!
//! A target is one of a known few rather than anything a triple can spell. What
//! each one is worth was measured with `clang 20.1.6 -dM -E` per triple rather
//! than recalled, and [`Target::ALL`] is written out so that a reader can check
//! it against the same command.
//!
//! [ADR-0013]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0013-the-translation-unit-carries-the-target-and-answers-what-a-type-is-worth.md

/// What an integer type is worth on some target.
///
/// The two facts C needs to say what an operation means: how wide the value is,
/// and whether the top bit is a sign. Everything about overflow and conversion
/// follows from them, which is why they are one type rather than two numbers
/// passed around together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Integer {
    bits: u32,
    signed: bool,
}

impl Integer {
    /// How wide a value of this type is.
    pub fn bits(self) -> u32 {
        self.bits
    }

    /// Whether the top bit is a sign.
    pub fn signed(self) -> bool {
        self.signed
    }

    /// The largest value this type holds.
    pub fn max(self) -> i128 {
        if self.signed {
            (1i128 << (self.bits - 1)) - 1
        } else {
            (1i128 << self.bits) - 1
        }
    }

    /// The smallest value this type holds.
    pub fn min(self) -> i128 {
        if self.signed {
            -(1i128 << (self.bits - 1))
        } else {
            0
        }
    }

    /// Whether this type can represent that value.
    ///
    /// What C17 6.5 p5 asks of the result of an operation: "if the result is
    /// not mathematically defined or not in the range of representable values
    /// for its type, the behavior is undefined". A caller that gets `false` has
    /// a program C says nothing about.
    pub fn holds(self, value: i128) -> bool {
        self.min() <= value && value <= self.max()
    }

    /// That value, converted to this type.
    ///
    /// C17 6.3.1.3. A value the type holds is unchanged (p1). To an unsigned
    /// type, one that does not fit is brought into range by "repeatedly adding
    /// or subtracting one more than the maximum value that can be represented"
    /// (p2). To a signed type, p3 says the result "is implementation-defined or
    /// an implementation-defined signal is raised", and this implementation
    /// answers what two's complement gives, which is what `clang` answers on
    /// every target in [`Target::ALL`].
    ///
    /// # Panics
    ///
    /// If `bits` is 128 or more, where a value of the type would not fit the
    /// carrier. No target names such a type and none can while `Target` is a
    /// fixed table.
    pub fn convert(self, value: i128) -> i128 {
        assert!(self.bits < 128, "an integer wider than the arithmetic here");

        let span = 1i128 << self.bits;
        // `rem_euclid` rather than `%`, because `%` keeps the sign of the left
        // operand in Rust, and C's rule is about adding or subtracting the span
        // until the value is in range, which is what a non-negative remainder
        // is.
        //
        // Taken before anything is added to `value`, because `value` can be
        // anywhere an `i128` reaches: a constant in the IR is whatever the
        // source spelled, and `value - self.min()` on a number near
        // `i128::MAX` overflows the carrier and panics. This way the only
        // arithmetic on the full-width value is a remainder, which cannot.
        let folded = value.rem_euclid(span);
        if folded > self.max() {
            folded - span
        } else {
            folded
        }
    }
}

/// A machine, and what C's types are worth on it.
///
/// Only `int` and `char` are described, because they are the only integer types
/// the frontend parses. A pointer's width is deliberately absent: nothing can
/// observe one yet, and ADR-0013 records that as the rule rather than an
/// oversight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    triple: &'static str,
    int: Integer,
    character: Integer,
}

impl Target {
    /// Every target this compiler knows.
    ///
    /// Written out, and each row measured with
    /// `clang 20.1.6 --target=<triple> -dM -E -x c /dev/null` reading
    /// `__SIZEOF_INT__`, `__CHAR_BIT__` and `__CHAR_UNSIGNED__`. A row nobody
    /// measured is a machine this compiler would be inventing, so the test
    /// beside this writes the same numbers out again rather than walking this
    /// table against itself. RK-001 is why.
    ///
    /// The seven are what is needed rather than what exists: five so that each
    /// of the three CI runners and this project's own machines can host
    /// themselves, `i686` and `wasm32` for the 32-bit cases a later phase will
    /// want, and `aarch64-unknown-linux-gnu` because it is the only one
    /// measured where `char` is unsigned.
    pub const ALL: &'static [Self] = &[
        Self::new("aarch64-apple-darwin", 32, 8, true),
        Self::new("aarch64-unknown-linux-gnu", 32, 8, false),
        Self::new("i686-unknown-linux-gnu", 32, 8, true),
        Self::new("wasm32-unknown-unknown", 32, 8, true),
        Self::new("x86_64-apple-darwin", 32, 8, true),
        Self::new("x86_64-pc-windows-msvc", 32, 8, true),
        Self::new("x86_64-unknown-linux-gnu", 32, 8, true),
    ];

    /// One row of [`Self::ALL`], and nothing else builds one.
    ///
    /// Private, so a `Target` is always a machine somebody measured rather than
    /// four numbers a caller chose. The same guard `Policy` has in ADR-0004,
    /// held the same way: by the field being unreachable from outside.
    const fn new(triple: &'static str, int_bits: u32, char_bits: u32, char_signed: bool) -> Self {
        Self {
            triple,
            int: Integer {
                bits: int_bits,
                // C17 6.2.5 p4: `int` is one of the standard signed integer
                // types, so this is not a target's choice to make.
                signed: true,
            },
            character: Integer {
                bits: char_bits,
                signed: char_signed,
            },
        }
    }

    /// The target that triple names, or `None` for one this compiler does not
    /// know.
    pub fn from_triple(triple: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|known| known.triple == triple)
    }

    /// What this machine is called.
    pub fn triple(self) -> &'static str {
        self.triple
    }

    /// What `int` is worth here.
    pub fn int(self) -> Integer {
        self.int
    }

    /// What `char` is worth here.
    ///
    /// Named after the C keyword, the way [`Self::int`] is and the way `clang`
    /// names the same thing (`getCharWidth`, `SignedChar`, `UnsignedChar`). A
    /// method and a primitive type do not share a namespace in Rust, so this
    /// costs nothing and keeps `Ty::Char` and `char()` reading as one pair.
    ///
    /// C17 6.2.5 p15 makes plain `char` a third type beside `signed char` and
    /// `unsigned char`: "the implementation shall define char to have the same
    /// range, representation, and behavior as either". Which one is the
    /// target's to say, and it is unsigned on `aarch64-unknown-linux-gnu` and
    /// signed on every other row of [`Self::ALL`].
    pub fn char(self) -> Integer {
        self.character
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table says what `clang` says, written out rather than walked.
    ///
    /// A test that asked `Target::ALL` about itself would hold for any numbers
    /// at all, which is RK-001 in the review knowledge bank and the reason the
    /// keyword table is written out too. These numbers came from
    /// `clang 20.1.6 --target=<triple> -dM -E -x c /dev/null`, reading
    /// `__SIZEOF_INT__`, `__CHAR_BIT__` and `__CHAR_UNSIGNED__`.
    ///
    /// Mutation: make `char` signed on `aarch64-unknown-linux-gnu`, or change
    /// any width. This fails. Mutation: add a row to `Target::ALL` without
    /// measuring it. The count fails.
    #[test]
    fn every_target_is_what_clang_says_it_is() {
        let measured: &[(&str, u32, u32, bool)] = &[
            ("aarch64-apple-darwin", 32, 8, true),
            ("aarch64-unknown-linux-gnu", 32, 8, false),
            ("i686-unknown-linux-gnu", 32, 8, true),
            ("wasm32-unknown-unknown", 32, 8, true),
            ("x86_64-apple-darwin", 32, 8, true),
            ("x86_64-pc-windows-msvc", 32, 8, true),
            ("x86_64-unknown-linux-gnu", 32, 8, true),
        ];

        assert_eq!(
            Target::ALL.len(),
            measured.len(),
            "a target nobody measured is a machine this compiler invented"
        );

        for &(triple, int_bits, char_bits, char_signed) in measured {
            let target = Target::from_triple(triple).expect("a measured triple is known");
            assert_eq!(target.int().bits(), int_bits, "{triple}");
            assert!(target.int().signed(), "{triple}: C17 6.2.5 p4");
            assert_eq!(target.char().bits(), char_bits, "{triple}");
            assert_eq!(target.char().signed(), char_signed, "{triple}");
        }
    }

    /// A triple nothing measured is not a target.
    ///
    /// Mutation: answer the first row for anything. This fails.
    #[test]
    fn a_triple_this_compiler_does_not_know_is_not_a_target() {
        assert_eq!(Target::from_triple("x86_64-unknown-none"), None);
        assert_eq!(Target::from_triple(""), None);
        assert_eq!(Target::from_triple("X86_64-PC-WINDOWS-MSVC"), None);
    }

    /// What a signed type holds, at both ends.
    ///
    /// Mutation: drop the `- 1` from `max`, or the negation from `min`. Either
    /// fails, and either would make an overflow look representable.
    #[test]
    fn a_signed_type_holds_what_two_s_complement_holds() {
        let int = Target::from_triple("x86_64-pc-windows-msvc")
            .expect("a known triple")
            .int();

        assert_eq!(int.max(), 2_147_483_647);
        assert_eq!(int.min(), -2_147_483_648);
        assert!(int.holds(2_147_483_647));
        assert!(!int.holds(2_147_483_648));
        assert!(int.holds(-2_147_483_648));
        assert!(!int.holds(-2_147_483_649));
    }

    /// What an unsigned type holds starts at zero.
    ///
    /// Mutation: give `min` the signed answer whatever the type. This fails.
    #[test]
    fn an_unsigned_type_holds_nothing_below_zero() {
        let character = Target::from_triple("aarch64-unknown-linux-gnu")
            .expect("a known triple")
            .char();

        assert_eq!((character.min(), character.max()), (0, 255));
        assert!(!character.holds(-1));
        assert!(!character.holds(256));
    }

    /// A value a type holds is converted to itself.
    ///
    /// C17 6.3.1.3 p1: "if the value can be represented by the new type, it is
    /// unchanged". Mutation: drop the early return and convert unconditionally.
    /// The arithmetic answers the same for every value in range, so nothing
    /// fails, which is worth knowing: this test is about the sentence rather
    /// than about a behaviour only it can hold. What it does catch is a `min`
    /// or `max` that has narrowed, because then a value that fits stops being
    /// recognised and comes back changed.
    #[test]
    fn a_value_that_fits_is_unchanged() {
        let character = Target::from_triple("x86_64-pc-windows-msvc")
            .expect("a known triple")
            .char();

        for value in [0, 1, -1, 127, -128] {
            assert_eq!(character.convert(value), value);
        }
    }

    /// A value anywhere an `i128` reaches converts without overflowing it.
    ///
    /// A constant in the IR is whatever the source spelled, and this frontend
    /// parses a decimal literal into an `i128` with no range check of its own,
    /// so `int x = 170141183460469231731687303715884105727;` reaches here. An
    /// implementation that shifted the value by `min` before folding it
    /// overflowed the carrier and panicked, which is the one answer a compiler
    /// must not give.
    ///
    /// Mutation: write it as `(value - self.min()).rem_euclid(span) +
    /// self.min()`. The extremes panic with "attempt to subtract with
    /// overflow" and this fails.
    #[test]
    fn a_value_anywhere_in_the_carrier_converts() {
        let int = Target::from_triple("x86_64-pc-windows-msvc")
            .expect("a known triple")
            .int();

        for value in [i128::MAX, i128::MIN, i128::MAX - 1, i128::MIN + 1] {
            let converted = int.convert(value);
            assert!(int.holds(converted), "{value} became {converted}");
        }

        // The answer is the two's complement one, not merely something in
        // range: `i128::MAX` is all ones below the sign, so its low 32 bits are
        // all ones, which is -1 as a 32-bit signed integer.
        assert_eq!(int.convert(i128::MAX), -1);
        assert_eq!(int.convert(i128::MIN), 0);
    }

    /// A value that does not fit is converted the way the target's `clang` does.
    ///
    /// The numbers are `clang 20.1.6`'s: it compiles `char c; c = 200; return
    /// c;` to a `sext i8` for `x86_64-pc-windows-msvc` and a `zext i8` for
    /// `aarch64-unknown-linux-gnu`, which is -56 and 200. 300 answers 44 on
    /// both, because 300 truncates to `0x2C` and 44 is positive, so it is the
    /// truncation rather than the sign that it shows.
    ///
    /// Mutation: use `%` rather than `rem_euclid`. Rust's `%` keeps the sign of
    /// the left operand, so `convert(-200)` answers -200 on the unsigned row
    /// where this answers 56, and the last two assertions fail. The positive
    /// cases pass either way, which is why the negative ones are here.
    #[test]
    fn a_value_that_does_not_fit_is_converted_the_way_the_target_does() {
        let signed = Target::from_triple("x86_64-pc-windows-msvc")
            .expect("a known triple")
            .char();
        let unsigned = Target::from_triple("aarch64-unknown-linux-gnu")
            .expect("a known triple")
            .char();

        assert_eq!(signed.convert(200), -56);
        assert_eq!(unsigned.convert(200), 200);

        assert_eq!(signed.convert(300), 44);
        assert_eq!(unsigned.convert(300), 44);

        assert_eq!(signed.convert(-200), 56);
        assert_eq!(unsigned.convert(-200), 56);
    }
}
