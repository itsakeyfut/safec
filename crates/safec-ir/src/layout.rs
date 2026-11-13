//! The shape of a type as an analysis asks it: what memory holding it holds,
//! whether that is a pointer, and how many `*`s deep it goes.
//!
//! At crate level because none of it is about memory, and one rule asked in
//! two places drifts: "does memory of this type hold pointers" was written
//! five times once (#843) and twice again until #895.

use crate::ir::{TranslationUnit, Ty, TyId};

/// The type an array's elements have, through every dimension, and `ty`
/// itself where it is no array: what memory holding a `ty` holds one
/// object after another of. Memory holding `int *[2][3]` holds `int *`s.
/// Not one dimension's element, which `Ty::Array`'s `element` is: that of
/// `int *[2][3]` is `int *[3]`.
pub(crate) fn innermost_element(unit: &TranslationUnit, ty: TyId) -> TyId {
    match unit.ty(ty) {
        Ty::Array { element, .. } => innermost_element(unit, element),
        Ty::Int | Ty::Char | Ty::Void | Ty::Pointer(_) => ty,
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
pub(crate) fn stored_pointer(unit: &TranslationUnit, stored: TyId) -> Option<TyId> {
    let element = innermost_element(unit, stored);
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
        Ty::Int | Ty::Char | Ty::Void => 0,
        // As deep as its element: `int ***a[2]` holds pointers three `*`s
        // deep, which `Allocations::deepest_pointer` has to be long enough for.
        // **Held by nothing, measured**: answering `0` changes no case,
        // because every use of an array converts it into a temporary of the
        // element's pointer type, which is one `*` deeper still.
        Ty::Array { element, .. } => pointer_depth(unit, element),
    }
}
