//! The shape of a type as an analysis asks it: what memory holding it holds,
//! whether that is a pointer, and how many `*`s deep it goes.
//!
//! At crate level because none of it is about memory, and one rule asked in
//! two places drifts: "does memory of this type hold pointers" was written
//! five times once (#843) and twice again until #895.

use crate::ir::Rank;
use crate::ir::{TranslationUnit, Ty, TyId};

/// The type an array's elements have, through every dimension, and `ty`
/// itself where it is no array: what memory holding a `ty` holds one
/// object after another of. Memory holding `int *[2][3]` holds `int *`s.
/// Not one dimension's element, which `Ty::Array`'s `element` is: that of
/// `int *[2][3]` is `int *[3]`.
pub(crate) fn innermost_element(unit: &TranslationUnit, ty: TyId) -> TyId {
    match unit.ty(ty) {
        Ty::Array { element, .. } => innermost_element(unit, element),
        // Memory holding a struct holds that struct, whose members differ:
        // [`stored_pointer`] answers for them.
        Ty::Integer {
            rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
            sign: _,
        }
        | Ty::Void
        | Ty::Pointer(_)
        | Ty::Struct(_) => ty,
    }
}

/// The pointer type memory holding a `stored` holds, through an array to its
/// element, and `None` where it holds no pointer: `int *` for `int *`,
/// `int *[1]` and `int *[2][3]`, nothing for `int` or `int [4]`.
///
/// **The one answer to whether memory holds pointers**:
/// `report::unwritten_read` for a read by name (once
/// `Allocations::holds_a_pointer`, a second spelling of this, #895),
/// `Allocations::first_level_holds_a_pointer`, `built::write_effects` for
/// a local kept whole, and in `report.rs`
/// `report_returned_unwritten`, `copied_unwritten`, `part_of_a_pointer`,
/// and `contents_leaving` for what the receiver reads and for each slot it
/// reads. Five of them once stopped at an array, so a copy of unwritten
/// bytes into a caller's `int *[1]` was silent where one into an `int *`
/// was `SC0411`, and a `char`'s address read through an `int *(*)[1]` was
/// silent where through an `int **` it was `SC0413` (#843). A kind of type
/// added later is asked at [`Ty::is_pointer`], an exhaustive `match`.
///
/// **A struct holds the deepest pointer any member holds**, and none where no
/// member holds one: its memory is pointers at some slots and not others, and
/// answering that it holds none would leave a pointer member unasked by every
/// reader above, a silence, where answering that it holds one asks the
/// slots that hold none too, a report the reader sees (#1200). **Held by
/// nothing, measured**: answering none changes no case, since every member
/// is read and written through a pointer typed as the member, which asks
/// the member's own type; a struct copied or handed whole read the same.
pub(crate) fn stored_pointer(unit: &TranslationUnit, stored: TyId) -> Option<TyId> {
    let element = innermost_element(unit, stored);
    if let Ty::Struct(layout) = unit.ty(element) {
        return unit
            .struct_layout(layout)
            .members
            .iter()
            .filter_map(|member| stored_pointer(unit, member.ty))
            .max_by_key(|&pointer| pointer_depth(unit, pointer));
    }
    unit.ty(element).is_pointer().then_some(element)
}

/// How many `*`s deep a type is: `0` for one that is no pointer. What
/// `Allocations::deepest_pointer` in the memory check is the largest of.
///
/// Written out rather than as a `matches!`, so a kind of type added later
/// is asked whether it has a pointee.
pub(crate) fn pointer_depth(unit: &TranslationUnit, ty: TyId) -> usize {
    match unit.ty(ty) {
        Ty::Pointer(pointee) => 1 + pointer_depth(unit, pointee),
        Ty::Integer {
            rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
            sign: _,
        }
        | Ty::Void => 0,
        // As deep as its element: `int ***a[2]` holds pointers three `*`s
        // deep, which `Allocations::deepest_pointer` has to be long enough for.
        // **Held by nothing, measured**: answering `0` changes no case,
        // because every use of an array converts it into a temporary of the
        // element's pointer type, which is one `*` deeper still.
        Ty::Array { element, .. } => pointer_depth(unit, element),
        // As deep as its deepest member, for the same reason, and held by
        // nothing either, measured.
        Ty::Struct(layout) => unit
            .struct_layout(layout)
            .members
            .iter()
            .map(|member| pointer_depth(unit, member.ty))
            .max()
            .unwrap_or(0),
    }
}
