//! How a struct crosses a call on each target, as `clang` passes and returns
//! one: the C ABI the IR's [`Passes`] marks are written in. See ADR-0124.
//!
//! **One function, a `match` per target**, so a target this does not write
//! is answered here, by name, rather than given another's convention. Only
//! integer and pointer members and parameters exist yet, so every piece of a
//! struct is an integer and every parameter takes an integer register; a
//! floating member or parameter is #1195's, and changes what this answers.

use safec_ir::ir::{Passes, StructLayout};
use safec_ir::target::Target;

/// Some of a struct's bytes, carried in one integer register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    /// The register's width in bits, as the signature spells it.
    pub(crate) bits: u32,
    /// How many of the struct's bytes it carries, from `at`. Fewer than the
    /// register holds where the struct ends inside it: the rest is padding a
    /// caller widens into and a callee drops, and nothing reads past the
    /// struct's last byte for it.
    pub(crate) bytes: u64,
    /// The offset of the first of them.
    pub(crate) at: u64,
}

impl Piece {
    /// The integer type that holds exactly the bytes this carries.
    pub(crate) fn carried(self) -> String {
        format!("i{}", self.bytes * 8)
    }

    /// The integer type the register is spelled as.
    pub(crate) fn spelled(self) -> String {
        format!("i{}", self.bits)
    }
}

/// How a struct is passed or returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// In integer registers, one per piece, in order.
    Registers {
        pieces: Vec<Piece>,
        /// Whether more than one piece is one LLVM value of array type,
        /// `[2 x i64]`, which is how AArch64 asks for consecutive registers,
        /// rather than a separate argument each, as System V does.
        together: bool,
    },
    /// In memory: an argument as the address of the caller's copy, which
    /// System V marks `byval` and Win64 and AArch64 pass as a plain pointer,
    /// and a result through a pointer the caller passes first, `sret`.
    Memory {
        /// Whether an argument is marked `byval`: System V copies it into the
        /// callee's frame, and the others hand the caller's copy over as it
        /// is.
        byval: bool,
    },
}

impl Class {
    /// The one LLVM type a struct returned in registers is returned as, and a
    /// struct passed `together` is passed as: an integer for one piece, an
    /// array for AArch64's pair, and a literal struct for System V's.
    pub(crate) fn aggregate(pieces: &[Piece], together: bool) -> String {
        match pieces {
            [one] => one.spelled(),
            _ if together => format!("[{} x {}]", pieces.len(), pieces[0].spelled()),
            _ => format!(
                "{{ {} }}",
                pieces
                    .iter()
                    .map(|piece| piece.spelled())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// One parameter of a signature, as far as how structs cross is concerned.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Parameter<'a> {
    /// A parameter of its own, which takes one integer register.
    Scalar,
    /// A struct passed by value, or where one is returned: see [`Passes`].
    /// The flag says whether it holds a flexible array member, itself or
    /// in a member, which Win64 and System V pass in memory whatever its
    /// size (#1208 review).
    Struct(Passes, &'a StructLayout, bool),
}

/// A target's convention, as `classify` takes it: the rule a struct is
/// classified by from its size and whether it is a result, whether the
/// integer registers left are counted, and what a struct holding a flexible
/// array member crosses as whatever its size, where the target sets one
/// apart.
type Convention = (fn(u64, bool) -> Class, bool, Option<Class>);

/// How each struct of a signature crosses a call on `target`, in its
/// parameters' order and `None` for a scalar, or `None` for the whole on a
/// target whose C ABI this backend does not write yet, or for a struct
/// aligned past 8 bytes, which no member here can make yet.
///
/// **Answered for the whole signature at once**, because on System V where
/// a struct goes depends on what came before it: a struct of two pieces that
/// finds one integer register left goes into memory whole rather than
/// splitting between the register and the stack, which is where LLVM puts
/// two separate pieces and not where `clang` looks for them.
///
/// - **Win64** (`x86_64-pc-windows-msvc`): a struct of 1, 2, 4 or 8 bytes is
///   an integer of that width, and any other is passed as the address of
///   the caller's copy and returned through `sret`.
/// - **x86-64 System V** (Linux and Darwin): a struct of at most 16 bytes is
///   one or two integer registers, the first holding its first eight bytes,
///   while six registers, less those taken before it, hold all of it; a
///   larger one, or one that does not fit, is `byval`, and returned through
///   `sret`, which takes a register.
/// - **AArch64** (Linux and Darwin): a struct of at most 16 bytes is passed
///   as `i64` or `[2 x i64]`, and returned in the integer of its own width
///   or as `[2 x i64]`; a larger one is passed as the address of the
///   caller's copy and returned through `sret`, in `x8`, which takes none of
///   the argument registers. LLVM places a `[2 x i64]` that does not fit
///   on the stack whole, so nothing here counts.
///
/// Mutation: drop the count of free registers on System V;
/// `a_struct_that_does_not_fit_the_registers_left_goes_in_memory` fails.
pub(crate) fn classify(target: Target, parameters: &[Parameter]) -> Option<Vec<Option<Class>>> {
    // A signature with no struct in it is written on every target, whatever
    // this knows of its C ABI for structs. Mutation: drop this; every call
    // on `armv7` is refused, and
    // `a_narrow_unsigned_value_is_extended_without_a_sign` fails.
    if parameters
        .iter()
        .all(|parameter| matches!(parameter, Parameter::Scalar))
    {
        return Some(vec![None; parameters.len()]);
    }
    let triple = target.triple();
    // The rule, whether it counts the integer registers left, and what a
    // struct holding a flexible array member is passed as whatever its size:
    // memory on Win64 and System V, as `clang` passes one, and by its size
    // on AArch64, which `clang` does not set apart (#1208 review).
    let (rule, counts_registers, flexible): Convention = match triple {
        "x86_64-pc-windows-msvc" => (win64, false, Some(Class::Memory { byval: false })),
        "x86_64-unknown-linux-gnu" | "x86_64-apple-darwin" => {
            (system_v, true, Some(Class::Memory { byval: true }))
        }
        "aarch64-unknown-linux-gnu" | "aarch64-apple-darwin" => (aarch64, false, None),
        _ => return None,
    };
    let mut free: usize = 6;
    let mut classes = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        let Parameter::Struct(passes, layout, holds_flexible) = *parameter else {
            free = free.saturating_sub(1);
            classes.push(None);
            continue;
        };
        if layout.align > 8 {
            return None;
        }
        let result = passes == Passes::StructReturn;
        // Mutation: classify by size alone; `a_struct_with_a_flexible_array_member_crosses_in_memory`
        // passes one of 4 bytes in a register on System V, and `..._as_win64_says` on Win64.
        let mut class = match &flexible {
            Some(memory) if holds_flexible => memory.clone(),
            _ => rule(layout.size, result),
        };
        if counts_registers {
            match (&class, result) {
                (Class::Registers { pieces, .. }, false) if pieces.len() > free => {
                    class = Class::Memory { byval: true };
                }
                (Class::Registers { pieces, .. }, false) => free -= pieces.len(),
                (Class::Memory { .. }, true) => free = free.saturating_sub(1),
                _ => {}
            }
        }
        classes.push(Some(class));
    }
    Some(classes)
}

/// The pieces of the first `size` bytes, each register `bits` wide: the last
/// carries what is left.
fn pieces(size: u64, bits: impl Fn(u64) -> u32) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut at = 0;
    while at < size {
        let bytes = (size - at).min(8);
        pieces.push(Piece {
            bits: bits(bytes),
            bytes,
            at,
        });
        at += bytes;
    }
    pieces
}

/// The width in bits of an integer of `bytes` bytes.
fn exact(bytes: u64) -> u32 {
    u32::try_from(bytes * 8).expect("a piece is at most 8 bytes")
}

fn win64(size: u64, _result: bool) -> Class {
    match size {
        1 | 2 | 4 | 8 => Class::Registers {
            pieces: pieces(size, exact),
            together: false,
        },
        _ => Class::Memory { byval: false },
    }
}

fn system_v(size: u64, _result: bool) -> Class {
    match size {
        1..=16 => Class::Registers {
            pieces: pieces(size, exact),
            together: false,
        },
        _ => Class::Memory { byval: true },
    }
}

fn aarch64(size: u64, result: bool) -> Class {
    match size {
        1..=8 if result => Class::Registers {
            pieces: pieces(size, exact),
            together: true,
        },
        1..=16 => Class::Registers {
            pieces: pieces(size, |_| 64),
            together: true,
        },
        _ => Class::Memory { byval: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use safec_ir::ir::{Rank, Sign, TranslationUnit, Ty};

    fn layout_of(target: Target, bytes: usize) -> StructLayout {
        let mut unit = TranslationUnit::new(target);
        let char = unit.push_type(Ty::Integer {
            rank: Rank::Char,
            sign: Sign::Signed,
        });
        let id = unit.push_struct(&vec![char; bytes]).expect("laid out");
        unit.struct_layout(id).clone()
    }

    /// How a struct of `bytes` bytes crosses as `passes` alone.
    fn alone(triple: &str, bytes: usize, passes: Passes) -> Option<Class> {
        let target = Target::from_triple(triple).expect("known");
        let layout = layout_of(target, bytes);
        classify(target, &[Parameter::Struct(passes, &layout, false)])
            .map(|mut classes| classes.remove(0).expect("a struct"))
    }

    fn registers(pieces: &[(u32, u64, u64)], together: bool) -> Option<Class> {
        Some(Class::Registers {
            pieces: pieces
                .iter()
                .map(|&(bits, bytes, at)| Piece { bits, bytes, at })
                .collect(),
            together,
        })
    }

    const WIN64: &str = "x86_64-pc-windows-msvc";
    const LINUX: &str = "x86_64-unknown-linux-gnu";
    const DARWIN: &str = "x86_64-apple-darwin";
    const ARM: &str = "aarch64-apple-darwin";
    const ARM_LINUX: &str = "aarch64-unknown-linux-gnu";

    /// Each size class lands where `clang` 22 puts it, read from its
    /// `-emit-llvm` for each triple. Mutation: drop `2` from Win64's sizes;
    /// this fails.
    #[test]
    fn each_size_class_is_passed_as_its_target_says() {
        use Passes::{StructCopy as Arg, StructReturn as Ret};
        for size in [1, 2, 4, 8] {
            let bits = u32::try_from(size * 8).expect("small");
            assert_eq!(
                alone(WIN64, size, Arg),
                registers(&[(bits, size as u64, 0)], false)
            );
        }
        assert_eq!(alone(WIN64, 3, Arg), Some(Class::Memory { byval: false }));
        for triple in [LINUX, DARWIN] {
            assert_eq!(alone(triple, 3, Arg), registers(&[(24, 3, 0)], false));
            assert_eq!(
                alone(triple, 12, Ret),
                registers(&[(64, 8, 0), (32, 4, 8)], false)
            );
            assert_eq!(alone(triple, 24, Arg), Some(Class::Memory { byval: true }));
        }
        for triple in [ARM, ARM_LINUX] {
            assert_eq!(alone(triple, 3, Arg), registers(&[(64, 3, 0)], true));
            assert_eq!(alone(triple, 3, Ret), registers(&[(24, 3, 0)], true));
            assert_eq!(
                alone(triple, 12, Ret),
                registers(&[(64, 8, 0), (64, 4, 8)], true)
            );
            assert_eq!(alone(triple, 24, Arg), Some(Class::Memory { byval: false }));
        }
        assert_eq!(alone("i686-unknown-linux-gnu", 8, Arg), None);
    }

    /// A struct of two pieces that finds one System V register left goes
    /// into memory whole, as `clang` passes `take(long long x5, struct D)`;
    /// a result in memory takes a register for its pointer, and one in
    /// registers takes none. Mutation: drop the count; this fails.
    #[test]
    fn a_struct_that_does_not_fit_the_registers_left_goes_in_memory() {
        let target = Target::from_triple(LINUX).expect("known");
        let pair = layout_of(target, 16);
        let big = layout_of(target, 24);
        let exhausted = classify(
            target,
            &[
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Struct(Passes::StructCopy, &pair, false),
            ],
        )
        .expect("written");
        assert_eq!(exhausted[5], Some(Class::Memory { byval: true }));
        let behind_sret = classify(
            target,
            &[
                Parameter::Struct(Passes::StructReturn, &big, false),
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Struct(Passes::StructCopy, &pair, false),
            ],
        )
        .expect("written");
        assert_eq!(behind_sret[5], Some(Class::Memory { byval: true }));
        let behind_registers = classify(
            target,
            &[
                Parameter::Struct(Passes::StructReturn, &pair, false),
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Scalar,
                Parameter::Struct(Passes::StructCopy, &pair, false),
            ],
        )
        .expect("written");
        assert!(matches!(behind_registers[5], Some(Class::Registers { .. })));
    }
}
