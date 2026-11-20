//! Writes that may reach storage the program may not modify: a static
//! defined `const`, a `const` local, a string literal (C17 6.7.3 p6, 6.4.5
//! p7). Asked at the two places a write is made: a store through a pointer,
//! and a call handed a pointer it may write through. See ADR-0120.

use crate::analysis::Conclusion;
use crate::ir::{Element, FuncId, Function, Operand, Rvalue, Shape, Terminator};
use crate::library::Callee;

use super::allocations::Allocations;
use super::index::LocalIdx;
use super::known::Known;
use super::store::Landing;
use super::{Finding, Kind};

/// The locals of `function` defined `const`, by their position.
fn read_only_locals(function: &Function) -> Vec<LocalIdx> {
    function
        .locals()
        .filter(|&local| function.read_only(local))
        .map(LocalIdx::of)
        .collect()
}

/// Whether a write landing as `landing` may land in read-only storage: a
/// read-only local it may land in by an edge, or a static the pointer it
/// goes through may name, which the value's mark says (`Held::read_only`).
/// The value written through is the pointer's own one dereference down,
/// and what was read out of memory the levels above it deeper.
fn may_land_read_only(landing: &Landing, read_only: &[LocalIdx], known: &Known) -> bool {
    let marked = if landing.deep == 1 {
        known.points_to[landing.pointer].read_only
    } else {
        known.read_only_on_the_way(landing.pointer, landing.deep - 1)
    };
    marked
        || landing
            .targets
            .iter()
            .any(|target| read_only.contains(target))
}

/// Whether the pointer `operand` holds may name read-only storage: a static
/// its value's mark says it may (`Held::read_only`), or a read-only local it
/// may point at, read as the value is, by name or `depth` dereferences down.
fn names_read_only(operand: &Operand, read_only: &[LocalIdx], known: &Known) -> bool {
    let Operand::Copy(place) = operand else {
        return false;
    };
    match place.shape() {
        Shape::Local => {
            let held = &known.points_to[place.local];
            held.read_only || held.locals().any(|local| read_only.contains(&local))
        }
        // **And the locals it may point at, read as the value is**: `set(*pp)`
        // with `*pp` holding a `const` local's address was asked for a static
        // alone. Found by review.
        Shape::Deref(depth) => {
            known.read_only_on_the_way(place.local, depth)
                || known
                    .levels_below(place.local, depth)
                    .1
                    .iter()
                    .any(|local| read_only.contains(local))
        }
        Shape::Indexed => false,
    }
}

/// Report a `return` of a pointer that may name read-only storage from a
/// function whose result is no pointer to `const`: the caller believes it
/// may write through what it is given, as a callee handed one for a
/// non-`const` parameter does, which [`handed_read_only`] asks from the other
/// side. Found by review: `char *get(void) { return "ab"; }` and a write
/// through what `get` returns were believed. Mutation: skip this;
/// `a_write_to_a_string_literal_is_unproven`'s `returned` goes silent.
pub(super) fn returned_read_only(
    findings: &mut Vec<Finding>,
    function: &Function,
    element: &Element,
    known: &Known,
    func: FuncId,
) {
    let Element::Assign(operation) = element else {
        return;
    };
    if operation.place.local != function.return_place()
        || !operation.place.steps().is_empty()
        || function.returns_reads_only()
    {
        return;
    }
    let Rvalue::Use(operand) = &operation.value else {
        return;
    };
    if names_read_only(operand, &read_only_locals(function), known) {
        findings.push(Finding {
            function: func,
            kind: Kind::WritesReadOnly,
            conclusion: Conclusion::Unknown,
            at: operation.origin.span(),
            local: None,
            freed: None,
            made: None,
            unproven: None,
        });
    }
}

/// Report a store through a pointer that may land in read-only storage.
pub(super) fn written_read_only(
    findings: &mut Vec<Finding>,
    analysis: &Allocations<'_>,
    function: &Function,
    element: &Element,
    known: &Known,
    func: FuncId,
) {
    let Some(store) = analysis.store_in(element, known) else {
        return;
    };
    let Some(landing) = &store.landing else {
        return;
    };
    if may_land_read_only(landing, &read_only_locals(function), known) {
        findings.push(Finding {
            function: func,
            kind: Kind::WritesReadOnly,
            conclusion: Conclusion::Unknown,
            at: store.at,
            local: Some(landing.pointer),
            freed: None,
            made: None,
            unproven: None,
        });
    }
}

/// Report a call handed, for a parameter that is no pointer to `const`, a
/// pointer that may name read-only storage: the callee may write through it,
/// whether its body is here or not, and a library writes through its
/// destination. A parameter past the last, where nothing says the callee
/// only reads, is asked as one that may write. **Not `free` or `realloc`**,
/// which release what they are handed rather than write it: a literal
/// freed is the free rules' (`SC0401`), and said as a write it was said
/// wrongly. Mutation: ask them too; `a_string_literal_freed_is_doubted`
/// says `SC0417`.
pub(super) fn handed_read_only(
    findings: &mut Vec<Finding>,
    analysis: &Allocations<'_>,
    function: &Function,
    terminator: &Terminator,
    known: &Known,
    func: FuncId,
) {
    let Terminator::Call {
        callee,
        arguments,
        origin,
        ..
    } = terminator
    else {
        return;
    };
    match analysis.callee(*callee) {
        Callee::Frees | Callee::Reallocates => return,
        Callee::Allocates
        | Callee::ReturnsFirst
        | Callee::Copies
        | Callee::Ends
        | Callee::Reads
        | Callee::Opaque => {}
    }
    let called = analysis.unit.function(*callee);
    let read_only = read_only_locals(function);
    for (index, argument) in arguments.iter().enumerate() {
        if called.reads_only_through(index) {
            continue;
        }
        if names_read_only(argument, &read_only, known) {
            let Operand::Copy(place) = argument else {
                unreachable!("only a place names storage")
            };
            findings.push(Finding {
                function: func,
                kind: Kind::WritesReadOnly,
                conclusion: Conclusion::Unknown,
                at: origin.span(),
                local: Some(place.local),
                freed: None,
                made: None,
                unproven: None,
            });
        }
    }
}
