//! The analysis, [`Allocations`], and the facts about a function both the
//! walk and the report read from it.
//!
//! The walk is `transfer.rs` and the report is `report.rs`; what both read
//! is here, so that the walk imports nothing of the report's to get it. The
//! report still reads the walk's `CopiedPointers`, which is the direction a
//! report reads in. See ADR-0063.

use crate::cfg::Cfg;
use crate::ir::{FuncId, Function, LocalId, Operand, Place, TranslationUnit, Ty, TyId};
use crate::library::Callee;
use crate::reads::derefs;
use crate::source::SourceMap;

use super::index::{LocalIdx, Table};

/// The analysis: where an allocation is, and whether it has been freed.
pub(super) struct Allocations<'a> {
    pub(super) sources: &'a SourceMap,
    pub(super) unit: &'a TranslationUnit,
    /// The function this is the analysis of, which every finding names.
    pub(super) function: FuncId,
    /// How many locals the function has, which is how many sites there can be.
    pub(super) locals: usize,
    /// How deep the deepest pointer type among the function's locals is,
    /// counted in `*`s, which is as long as a [`Levels`](super::parts::Levels) here gets.
    ///
    /// Mutation: two shorter, and
    /// `a_void_pointer_three_levels_deep_converted_whole_is_believed_below_its_first_level`
    /// goes silent, since the `0` a `void ***` believes two levels down is
    /// the entry dropped. One shorter changes no case, for the reason
    /// [`Levels`](super::parts::Levels) gives; the margin is kept so that the length needs no
    /// argument to be long enough.
    pub(super) deepest_pointer: usize,
    /// The locals a caller filled, which are sites because an allocation can
    /// arrive through one.
    pub(super) parameters: Vec<LocalId>,
    /// The parameters that hold a pointer, whose allocations are exposed
    /// where the function starts: the caller, which this check cannot read,
    /// had the pointer, and may have left it where a call can free it.
    ///
    /// **Not every parameter.** Every one is a site, for the reason the module
    /// comment of `memory` gives, and an integer parameter can hold an allocation only
    /// through a conversion this frontend does not accept yet: a cast, which
    /// does not parse, or an implicit one, which C17 6.5.16.1 p1 forbids and
    /// #154 is about. Exposing one made every opaque call's result reach it.
    /// The day casts parse, the type below stops being enough.
    ///
    /// **Not `main`'s either.** Its caller is the host, and C17 5.1.2.2.1 p2
    /// has `argv` and its strings keep their values until the program ends:
    /// no allocation function returned them, so no call can free them
    /// without the behaviour 7.22.3.3 p2 leaves undefined. Exposing them
    /// refused `log_line(); char *name = argv[0];`. A program that calls
    /// `main` itself hands it arguments this does not see. See ADR-0040.
    pub(super) exposed_parameters: Vec<LocalId>,
    /// Per local, whether a call this check cannot read writes its result
    /// into it: [`Self::opaque_results_of`] answers it, once per function.
    /// Read at a site as well, through [`Site::local`](super::index::Site::local), since a site is named by
    /// the local the call that made it wrote into. Not by position: a local
    /// written by such a
    /// call anywhere is marked everywhere, which can only add a doubt.
    ///
    /// What such a result points at is memory the callee handed over, as
    /// what a parameter points at is memory the caller did, so a pointer read
    /// out of it is one a later such call may free. See ADR-0040.
    pub(super) opaque_results: Table<LocalIdx, bool>,
    /// Per block, the locals read from its entry before they are written:
    /// what [`live_in`](crate::liveness::live_in) answers, once per function. See ADR-0048.
    pub(super) live_in: Vec<Vec<bool>>,
    /// The function's graph, for the walk
    /// [`tested_against_null`](crate::nullability::tested_against_null)
    /// makes back across a `Goto`.
    pub(super) cfg: Cfg,
    /// Per block, whether it is the head of a loop shown to run its body at
    /// least once, [`shown_to_run`](super::runs::shown_to_run): the one loop
    /// whose back edge creates no doubt. See ADR-0064.
    pub(super) runs_once: Vec<bool>,
}

/// The type an array's elements have, through every dimension, and `ty`
/// itself where it is no array: what memory holding a `ty` holds one
/// object after another of. Memory holding `int *[2][3]` holds `int *`s.
/// Not one dimension's element, which `Ty::Array`'s `element` is: that of
/// `int *[2][3]` is `int *[3]`.
pub(super) fn innermost_element(unit: &TranslationUnit, ty: TyId) -> TyId {
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
/// `Allocations::first_level_holds_a_pointer`, and in `report.rs`
/// `report_returned_unwritten`, `copied_unwritten`, `part_of_a_pointer`,
/// and `contents_leaving` for what the receiver reads and for each slot it
/// reads. Five of them once stopped at an array, so a copy of unwritten
/// bytes into a caller's `int *[1]` was silent where one into an `int *`
/// was `SC0411`, and a `char`'s address read through an `int *(*)[1]` was
/// silent where through an `int **` it was `SC0413` (#843). A kind of type
/// added later is asked at [`Ty::is_pointer`], an exhaustive `match`.
pub(super) fn stored_pointer(unit: &TranslationUnit, stored: TyId) -> Option<TyId> {
    let element = innermost_element(unit, stored);
    unit.ty(element).is_pointer().then_some(element)
}

/// The arguments a call is asked about as a pointer it was handed, one place
/// per local.
///
/// **One function for its two readers**, [`handed`](super::report::handed) at the call and
/// `Allocations::terminator` carrying them forwards, because one rule written
/// in two places drifts apart inside the change that touches one of them.
///
/// **Which arguments turn on the callee**, and the `match` is written out so
/// that a new kind of callee answers here. What `free` and `realloc`'s first
/// argument are handed is asked already, as a double free, by [`reported`](super::report::reported);
/// an allocator and `realloc`'s size are handed integers.
///
/// **A local of pointer type, or a place of dereferences that may be a
/// pointer.** The second is asked what its deepest level holds, through
/// [`Known::handed_reached`](super::known::Known::handed_reached), so `release(*tab)` is asked what `q = *tab;
/// release(q);` is (ADR-0045). What the call reaches through it is
/// `read_out`'s (ADR-0040). One place is asked once, compared as a place, so
/// `g(p, *p)` asks both. The pointer test on a place of dereferences changes
/// no case, measured: without a cast, an integer is read out of an allocation
/// that holds integers, and nothing was stored there for it to hold. An integer can hold sites, since
/// an addition keeps its operands' (ADR-0030), and asking one refused
/// `h(f() + g())`, a program with no pointer in it.
///
/// **The repeat test is answered twice, and a mutation sees only the other
/// answer.** Since [`handed`](super::report::handed) reports through [`say`](super::report::say), a place handed twice
/// lands on one key, so dropping the test leaves the whole suite green. It is
/// kept, because it says what is asked rather than what happens to collapse
/// afterwards, and measured without `say` it fails
/// `a_freed_pointer_handed_twice_to_one_call_is_one_report`.
///
/// **A place of dereferences lands on the key a dereference of the same
/// place at the same call holds**, which [`used`](super::report::used) fills first. So
/// `free(tab); release(*tab);` keeps the dereference's proof about reading
/// `*tab`, and what `*tab` holds is asked only when that says nothing.
pub(super) fn handed_places<'a>(
    analysis: &Allocations<'_>,
    function: &Function,
    callee: FuncId,
    arguments: &'a [Operand],
) -> Vec<&'a Place> {
    let arguments = match analysis.callee(callee) {
        Callee::Opaque | Callee::ReturnsFirst | Callee::Copies => arguments,
        Callee::Reallocates => &arguments[arguments.len().min(1)..],
        // A call that does not return reads nothing it is handed after its
        // arguments are evaluated, which `dereferenced_in_terminator` asks.
        // No C program reaches the difference: none of the four takes a
        // pointer as the library declares it, and one redeclared to take one
        // is undefined and believed all the same, as ADR-0051 believes it.
        Callee::Frees | Callee::Allocates | Callee::Ends => return Vec::new(),
    };

    let mut asked: Vec<&Place> = Vec::new();
    for argument in arguments {
        let Operand::Copy(place) = argument else {
            continue;
        };
        let pointer = if place.projection.is_empty() {
            analysis.is_pointer(function, place.local)
        } else {
            derefs(place) > 0 && analysis.may_be_pointer(function, place)
        };
        if !pointer || asked.contains(&place) {
            continue;
        }
        asked.push(place);
    }
    asked
}
