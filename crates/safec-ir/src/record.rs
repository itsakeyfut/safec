//! Where each member of a struct or union goes on a target, bit-fields
//! included: **the one rule for an aggregate's layout** (ADR-0124), asked by
//! [`TranslationUnit::complete_struct`](crate::ir::TranslationUnit::complete_struct)
//! and by the type check for `sizeof`, which is a constant before any IR
//! exists.
//!
//! **Bit-fields are packed as `clang` packs them on each target** (#1206),
//! since C17 6.7.2.1 p11 leaves the storage unit, whether a bit-field may
//! straddle one, and its order within one to the implementation, and a
//! struct laid out otherwise is a different struct from the one every other
//! object on the target was built with. There are three conventions among
//! the targets here, and each is a `match` arm, so a target added later
//! answers for which it follows:
//!
//! - **Microsoft** (`x86_64-pc-windows-msvc`): consecutive bit-fields of one
//!   type's size share a unit of that size while they fit; another size, or
//!   one that does not fit, starts a unit of its own, and an ordinary member
//!   closes it. A zero-width bit-field closes a unit only where one is open.
//! - **Itanium** (Linux, Darwin, WebAssembly): a bit-field goes at the next
//!   bit, unless a unit of its type's size starting on a boundary of its
//!   type's alignment cannot hold it whole there, and then at that next
//!   boundary. Only a named bit-field adds its type's alignment to the
//!   whole's; a zero-width one moves the next bit-field to a boundary and
//!   adds nothing.
//! - **AAPCS** (`armv7` and AArch64 Linux): Itanium's, except that an unnamed
//!   bit-field, a zero-width one included, adds its type's alignment too.
//!
//! Every answer here is held against `clang`'s own record layout on every
//! target by `the_layout_of_every_bit_field_shape_is_clangs` in
//! `tests/layout.rs`.

use crate::target::Target;

/// One member as a layout rule takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// How many bytes its type takes.
    pub size: u64,
    /// Its type's alignment in bytes, as the target aligns it in a struct.
    pub align: u64,
    /// Its width in bits where it is a bit-field, `0` for a zero-width one.
    pub width: Option<u64>,
    /// Whether it has a name: an unnamed bit-field is laid out and, on most
    /// targets, aligns nothing.
    pub named: bool,
}

/// What a layout rule answers: where each member starts, in bits from the
/// start of the whole, and the whole's size and alignment in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Each member's first bit, in the order given. A member that is not a
    /// bit-field always starts on a byte. Counted in a `u128`, since a struct
    /// whose size fits a `u64` of bytes may not fit one of bits.
    pub bits: Vec<u128>,
    /// How many bytes the whole takes, trailing padding included.
    pub size: u64,
    /// The strictest alignment any member adds.
    pub align: u64,
}

/// Which packing convention a target follows. See the module's comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Packing {
    Microsoft,
    Itanium,
    Aapcs,
}

/// The convention `target` packs bit-fields by.
fn packing(target: Target) -> Packing {
    match target.triple() {
        "x86_64-pc-windows-msvc" => Packing::Microsoft,
        "armv7-unknown-linux-gnueabihf" | "aarch64-unknown-linux-gnu" => Packing::Aapcs,
        _ => Packing::Itanium,
    }
}

/// Lay out `fields`, in order, as a struct or, where `union`, a union, on
/// `target`. `None` where a count overflows a `u64`.
///
/// Without a bit-field this is C17 6.7.2.1 p15 and p17 for a struct, each
/// member at the first offset after the one before it that its alignment
/// divides, and p16 for a union, every member at 0; either rounded up to
/// the strictest alignment, so that an array of them keeps each aligned.
///
/// Mutation: place each member at the end of the one before it;
/// `the_layout_of_every_struct_shape_is_clangs` fails. Mutation: let a
/// bit-field straddle its unit on Itanium; `the_layout_of_every_bit_field_shape_is_clangs`
/// fails.
pub fn lay_out(target: Target, union: bool, fields: &[Field]) -> Option<Record> {
    let packing = packing(target);
    if union {
        return lay_out_union(packing, fields);
    }
    match packing {
        Packing::Microsoft => lay_out_microsoft(fields),
        Packing::Itanium | Packing::Aapcs => lay_out_itanium(packing == Packing::Aapcs, fields),
    }
}

/// `value` rounded up to a multiple of `to`.
fn up(value: u64, to: u64) -> Option<u64> {
    value.checked_next_multiple_of(to)
}

/// `value` bits rounded up to a multiple of `to` bits.
fn up_bits(value: u128, to: u64) -> Option<u128> {
    value.checked_next_multiple_of(u128::from(to))
}

/// The bytes `bits` bits take, rounded up, where that fits a `u64`.
fn bytes(bits: u128) -> Option<u64> {
    u64::try_from(bits.checked_add(7)? / 8).ok()
}

/// The bits `bytes` bytes take.
fn bits_of(bytes: u64) -> u128 {
    u128::from(bytes) * 8
}

fn lay_out_itanium(aapcs: bool, fields: &[Field]) -> Option<Record> {
    let mut at: u128 = 0;
    let mut align: u64 = 1;
    let mut bits = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(width) = field.width else {
            let offset = up(bytes(at)?, field.align)?;
            bits.push(bits_of(offset));
            at = bits_of(offset.checked_add(field.size)?);
            align = align.max(field.align);
            continue;
        };
        let width = u128::from(width);
        let boundary = bits_of(field.align);
        if width == 0 {
            at = up_bits(at, boundary as u64)?;
        } else {
            // It fits where a unit of its type's size, starting on a
            // boundary of its type's alignment, holds it whole. On `i686` a
            // `long long` is aligned to 4 and takes 8, so one may start past
            // a boundary of 8.
            let start = at / boundary * boundary;
            if at + width > start + bits_of(field.size) {
                at = up_bits(at, boundary as u64)?;
            }
        }
        bits.push(at);
        if field.named || aapcs {
            align = align.max(field.align);
        }
        at += width;
    }
    Some(Record {
        bits,
        size: up(bytes(at)?, align)?,
        align,
    })
}

fn lay_out_microsoft(fields: &[Field]) -> Option<Record> {
    // The end of what is laid out, in bytes, and the unit bit-fields are
    // being packed into: where it starts, how many bytes it takes, and how
    // many of its bits are spent.
    let mut end: u64 = 0;
    let mut unit: Option<(u64, u64, u64)> = None;
    let mut align: u64 = 1;
    let mut bits = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(width) = field.width else {
            if let Some((start, size, _)) = unit.take() {
                end = start.checked_add(size)?;
            }
            let offset = up(end, field.align)?;
            bits.push(bits_of(offset));
            end = offset.checked_add(field.size)?;
            align = align.max(field.align);
            continue;
        };
        if width == 0 {
            // Closes the unit open, and aligns what follows as its type is
            // aligned; with none open it is nothing at all.
            if let Some((start, size, _)) = unit.take() {
                end = up(start.checked_add(size)?, field.align)?;
                align = align.max(field.align);
            }
            bits.push(bits_of(end));
            continue;
        }
        match unit {
            Some((start, size, spent))
                if size == field.size && spent.checked_add(width)? <= size.checked_mul(8)? =>
            {
                bits.push(bits_of(start) + u128::from(spent));
                unit = Some((start, size, spent + width));
            }
            _ => {
                if let Some((start, size, _)) = unit {
                    end = start.checked_add(size)?;
                }
                let start = up(end, field.align)?;
                bits.push(bits_of(start));
                unit = Some((start, field.size, width));
            }
        }
        align = align.max(field.align);
    }
    if let Some((start, size, _)) = unit {
        end = start.checked_add(size)?;
    }
    Some(Record {
        bits,
        size: up(end, align)?,
        align,
    })
}

/// Every member of a union starts at 0. An ordinary member takes its size
/// and its alignment. A bit-field takes, on Itanium and AAPCS, its width
/// rounded up to its type's alignment, and that alignment, except that on
/// Itanium an unnamed one takes nothing; on Microsoft, its type's whole size
/// and no alignment. A zero-width one takes nothing, but on AAPCS its
/// alignment.
fn lay_out_union(packing: Packing, fields: &[Field]) -> Option<Record> {
    let mut largest: u64 = 0;
    let mut align: u64 = 1;
    for field in fields {
        let (size, aligns) = match (field.width, packing) {
            (None, _) => (field.size, true),
            (Some(0), Packing::Microsoft | Packing::Itanium) => (0, false),
            (Some(0), Packing::Aapcs) => (0, true),
            (Some(_), Packing::Microsoft) => (field.size, false),
            (Some(_), Packing::Itanium) if !field.named => (0, false),
            (Some(width), Packing::Itanium | Packing::Aapcs) => {
                (up(bytes(u128::from(width))?, field.align)?, true)
            }
        };
        largest = largest.max(size);
        if aligns {
            align = align.max(field.align);
        }
    }
    Some(Record {
        bits: vec![0; fields.len()],
        size: up(largest, align)?,
        align,
    })
}
