//! Writes that may reach storage the program may not modify: a static
//! defined `const`, a `const` local, a string literal (C17 6.7.3 p6, 6.4.5
//! p7). Asked at the two places a write is made: a store through a pointer,
//! and a call handed a pointer it may write through. See ADR-0120.

use crate::analysis::Conclusion;
use crate::ir::{Element, FuncId, Function, Operand, Shape, Terminator};
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
        let Operand::Copy(place) = argument else {
            continue;
        };
        let marked = match place.shape() {
            Shape::Local => {
                let held = &known.points_to[place.local];
                held.read_only || held.locals().any(|local| read_only.contains(&local))
            }
            Shape::Deref(depth) => known.read_only_on_the_way(place.local, depth),
            Shape::Indexed => false,
        };
        if marked {
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
