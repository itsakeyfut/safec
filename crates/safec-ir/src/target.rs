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

/// Which way a value is widened when an ABI asks for it.
///
/// LLVM spells these `signext` and `zeroext`, and takes a call site whose
/// attribute disagrees with its callee's as undefined rather than as a mistake.
/// The names here are the question rather than that spelling, because a WASM or
/// a C backend needs the same answer and writes it differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Extend {
    /// Widened by repeating the top bit.
    Sign,
    /// Widened with zeroes.
    Zero,
}

/// Whether an ABI widens a value on the way into and out of a call at all.
///
/// Private, and the reason [`Target::extension`] exists: *which* values an ABI
/// widens, and which way, is the machine's to say and not a caller's to work
/// out. Measured elsewhere and not hypothetically: `riscv64-unknown-linux-gnu`
/// widens an `int` itself, so "narrower than `int`" is not the rule everywhere,
/// and widens an `unsigned int` with `signext`, so the value's own signedness
/// is not the direction everywhere either. A caller holding this flag would
/// have had to know both.
///
/// An enum rather than a `bool` because the constructor behind [`Target::ALL`]
/// would otherwise take two adjacent booleans, and swapping them in a table of
/// eight rows is `error[E0308]` this way and a test failure the other. A guard
/// the compiler holds beats one somebody has to remember to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Widening {
    /// The caller widens an argument, and the callee widens a result.
    Required,
    /// Neither does, and whoever receives one narrows it itself.
    None,
}

/// A machine, and what C's types are worth on it, and what it asks of a value
/// crossing a call.
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
    widening: Widening,
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
    /// The eight are what is needed rather than what exists: five so that each
    /// of the three CI runners and this project's own machines can host
    /// themselves, `i686` and `wasm32` for the 32-bit cases a later phase will
    /// want, and two where `char` is unsigned. Two, because one of them widens
    /// a narrow value and the other does not, and a [`Extend::Zero`] nothing
    /// can reach is a branch nothing can execute.
    ///
    /// The widening column came from
    /// `clang 20.1.6 --target=<triple> -S -emit-llvm -O0` on a function of one
    /// `char` returning one, reading the attribute on the result and on the
    /// parameter.
    pub const ALL: &'static [Self] = &[
        Self::new("aarch64-apple-darwin", 32, 8, true, Widening::Required),
        Self::new("aarch64-unknown-linux-gnu", 32, 8, false, Widening::None),
        Self::new(
            "armv7-unknown-linux-gnueabihf",
            32,
            8,
            false,
            Widening::Required,
        ),
        Self::new("i686-unknown-linux-gnu", 32, 8, true, Widening::Required),
        Self::new("wasm32-unknown-unknown", 32, 8, true, Widening::Required),
        Self::new("x86_64-apple-darwin", 32, 8, true, Widening::Required),
        Self::new("x86_64-pc-windows-msvc", 32, 8, true, Widening::None),
        Self::new("x86_64-unknown-linux-gnu", 32, 8, true, Widening::Required),
    ];

    /// One row of [`Self::ALL`], and nothing else builds one.
    ///
    /// Private, so a `Target` is always a machine somebody measured rather than
    /// four numbers a caller chose. The same guard `Policy` has in ADR-0004,
    /// held the same way: by the field being unreachable from outside.
    const fn new(
        triple: &'static str,
        int_bits: u32,
        char_bits: u32,
        char_signed: bool,
        widening: Widening,
    ) -> Self {
        Self {
            triple,
            widening,
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
    /// target's to say, and it is unsigned on two rows of [`Self::ALL`],
    /// `aarch64-unknown-linux-gnu` and `armv7-unknown-linux-gnueabihf`.
    pub fn char(self) -> Integer {
        self.character
    }

    /// Which way this machine widens that value on the way into and out of a
    /// call, or `None` where it leaves it alone.
    ///
    /// The only question here that is about how a function is called rather
    /// than about what a value is worth, which is why a backend asks it and the
    /// interpreter does not: nothing crosses a real ABI in there.
    ///
    /// **The rule is here rather than at the call site**, because every part of
    /// it is the machine's to say. Two of the eight rows leave a `char` alone;
    /// `riscv64-unknown-linux-gnu`, which is not in the table yet, widens an
    /// `int` as well and widens an `unsigned int` by its *sign*. A backend that
    /// worked any of that out for itself would be a second place to get it
    /// wrong, and `docs/architecture.md` names two more backends after this
    /// one.
    pub fn extension(self, value: Integer) -> Option<Extend> {
        if self.widening != Widening::Required {
            return None;
        }
        // C17 6.3.1.1 p2 promotes an arithmetic operand to `int`, so nothing
        // below that width is ever computed with and nothing at or above it is
        // narrow. True of every row measured so far and not of every machine
        // there is, which is why it is here where a row can disagree.
        if value.bits() >= self.int.bits() {
            return None;
        }

        Some(if value.signed() {
            Extend::Sign
        } else {
            Extend::Zero
        })
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
    /// `__SIZEOF_INT__`, `__CHAR_BIT__` and `__CHAR_UNSIGNED__`. The extension
    /// column came from `clang 20.1.6 --target=<triple> -S -emit-llvm -O0` on
    /// `char csink(char x) { return x; }`, reading the attribute on the result
    /// and on the parameter, which agree on every row.
    ///
    /// Mutation: make `char` signed on `aarch64-unknown-linux-gnu`, or change
    /// any width. This fails. Mutation: add a row to `Target::ALL` without
    /// measuring it. The count fails. Mutation: give
    /// `x86_64-pc-windows-msvc` `Widening::Required`. The `char` column fails,
    /// and so do four other tests: the two machines that ask for nothing are
    /// not predictable from the signedness beside them, so everything
    /// downstream of the column moves with it.
    #[test]
    fn every_target_is_what_clang_says_it_is() {
        // The last column is what `clang` writes for a `char`, which is the
        // measurement, rather than the flag behind it, which is this module's
        // own business.
        let measured: &[(&str, u32, u32, bool, Option<Extend>)] = &[
            ("aarch64-apple-darwin", 32, 8, true, Some(Extend::Sign)),
            ("aarch64-unknown-linux-gnu", 32, 8, false, None),
            (
                "armv7-unknown-linux-gnueabihf",
                32,
                8,
                false,
                Some(Extend::Zero),
            ),
            ("i686-unknown-linux-gnu", 32, 8, true, Some(Extend::Sign)),
            ("wasm32-unknown-unknown", 32, 8, true, Some(Extend::Sign)),
            ("x86_64-apple-darwin", 32, 8, true, Some(Extend::Sign)),
            ("x86_64-pc-windows-msvc", 32, 8, true, None),
            ("x86_64-unknown-linux-gnu", 32, 8, true, Some(Extend::Sign)),
        ];

        assert_eq!(
            Target::ALL.len(),
            measured.len(),
            "a target nobody measured is a machine this compiler invented"
        );

        for &(triple, int_bits, char_bits, char_signed, extension) in measured {
            let target = Target::from_triple(triple).expect("a measured triple is known");
            assert_eq!(target.int().bits(), int_bits, "{triple}");
            assert!(target.int().signed(), "{triple}: C17 6.2.5 p4");
            assert_eq!(target.char().bits(), char_bits, "{triple}");
            assert_eq!(target.char().signed(), char_signed, "{triple}");
            assert_eq!(target.extension(target.char()), extension, "{triple}");
            // Nothing widens an `int`, on every row measured so far. It is what
            // C17 6.3.1.1 p2 promotes to, so nothing is narrower than it that
            // an operation ever sees.
            assert_eq!(target.extension(target.int()), None, "{triple}");
        }
    }

    /// Every answer this can give has a machine that gives it.
    ///
    /// Without one, `zeroext` is a branch no C program on any known target can
    /// reach: the direction follows the value's signedness and whether there is
    /// an attribute at all follows the target, and until
    /// `armv7-unknown-linux-gnueabihf` was measured the only unsigned `char`
    /// was on a machine that widens nothing.
    ///
    /// Mutation: drop the `armv7` row. This fails, and so does the corpus case
    /// that is the only `zeroext` in the tree.
    #[test]
    fn every_answer_to_the_extension_question_has_a_machine() {
        let answers = |answer| {
            Target::ALL
                .iter()
                .any(|target| target.extension(target.char()) == answer)
        };

        assert!(answers(Some(Extend::Sign)), "signext");
        assert!(answers(Some(Extend::Zero)), "zeroext");
        assert!(answers(None), "nothing");
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
