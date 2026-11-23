//! How a struct crosses a call on each target, as `clang` passes and returns
//! one: the C ABI the IR's [`Passes`](safec_ir::ir::Passes) marks are written
//! in. See ADR-0124.
//!
//! **One function, a `match` per target**, so a target this does not write
//! is answered here, by name, rather than given another's convention. Only
//! integer and pointer members exist yet, so a struct's eightbytes are all
//! INTEGER in System V's terms; a floating member is #1195's, and changes
//! what this answers.

use safec_ir::ir::StructLayout;
use safec_ir::target::Target;

/// How a struct is passed or returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// In integer registers, one per piece: each piece is this many bits of
    /// the struct's bytes, the first from its start and each next from where
    /// the one before it ends.
    Registers(Vec<u32>),
    /// In memory: an argument as the address of the caller's copy, which
    /// System V marks `byval` and Win64 passes as a plain pointer, and a
    /// result through a pointer the caller passes first, `sret`.
    Memory {
        /// Whether an argument is marked `byval`: System V copies it into the
        /// callee's frame, and Win64 hands the caller's copy over as it is.
        byval: bool,
    },
}

/// How `layout` crosses a call on `target`, or `None` on a target whose C
/// ABI this backend does not write yet, which the caller refuses.
///
/// - **Win64** (`x86_64-pc-windows-msvc`): a struct of 1, 2, 4 or 8 bytes is
///   an integer of that width, and any other is passed as the address of
///   the caller's copy and returned through `sret`.
/// - **x86-64 System V** (Linux and Darwin): a struct of at most 16 bytes is
///   one or two integer registers, the first holding its first eight bytes;
///   any larger one is `byval` and returned through `sret`.
///
/// Mutation: answer `Memory` for a Win64 struct of 1, 2, 4 or 8 bytes;
/// `a_struct_by_value_meets_clangs_abi` fails on Windows, and
/// `each_size_class_is_passed_as_its_target_says` everywhere.
pub(crate) fn classify(target: Target, layout: &StructLayout) -> Option<Class> {
    let triple = target.triple();
    let size = layout.size;
    let bits = |bytes: u64| u32::try_from(bytes * 8).expect("a struct of at most 16 bytes");
    if triple == "x86_64-pc-windows-msvc" {
        return Some(match size {
            1 | 2 | 4 | 8 => Class::Registers(vec![bits(size)]),
            _ => Class::Memory { byval: false },
        });
    }
    if triple == "x86_64-unknown-linux-gnu" || triple == "x86_64-apple-darwin" {
        return Some(match size {
            0 => Class::Memory { byval: true },
            1..=8 => Class::Registers(vec![bits(size)]),
            9..=16 => Class::Registers(vec![64, bits(size - 8)]),
            _ => Class::Memory { byval: true },
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use safec_ir::ir::{Rank, Sign, TranslationUnit, Ty};

    fn layout_of(target: Target, ints: usize) -> StructLayout {
        let mut unit = TranslationUnit::new(target);
        let int = unit.push_type(Ty::Integer {
            rank: Rank::Char,
            sign: Sign::Signed,
        });
        let members = vec![int; ints];
        let id = unit.push_struct(&members).expect("laid out");
        unit.struct_layout(id).clone()
    }

    /// Each size class lands where the two ABIs put it. Mutation: answer
    /// `Memory` for a Win64 struct of 8 bytes; this fails.
    #[test]
    fn each_size_class_is_passed_as_its_target_says() {
        let win64 = Target::from_triple("x86_64-pc-windows-msvc").expect("known");
        let sysv = Target::from_triple("x86_64-unknown-linux-gnu").expect("known");
        assert_eq!(
            classify(win64, &layout_of(win64, 8)),
            Some(Class::Registers(vec![64]))
        );
        assert_eq!(
            classify(win64, &layout_of(win64, 3)),
            Some(Class::Memory { byval: false })
        );
        assert_eq!(
            classify(sysv, &layout_of(sysv, 3)),
            Some(Class::Registers(vec![24]))
        );
        assert_eq!(
            classify(sysv, &layout_of(sysv, 12)),
            Some(Class::Registers(vec![64, 32]))
        );
        assert_eq!(
            classify(sysv, &layout_of(sysv, 24)),
            Some(Class::Memory { byval: true })
        );
        let arm = Target::from_triple("aarch64-apple-darwin").expect("known");
        assert_eq!(classify(arm, &layout_of(arm, 8)), None);
    }
}
