//! What an element or a terminator reads through a pointer, and what it reads
//! out of memory as a value.
//!
//! These answer a question about the IR rather than about any one check, so
//! they live apart from either. The two readers of a dereference are asked by
//! both the memory check and the nullability check, and a change made to them
//! for one is a change to both. The two readers of a load are asked by the
//! memory check alone, and sit here beside the others because they answer the
//! same kind of question; the nullability check's own test of a load is
//! `nullability::read_out_of_memory`, which is a different rule.

use crate::ir::{Element, Operand, Place, Projection, Rvalue, Terminator};
use crate::source::Span;

/// Where this element runs, and every place it reads or writes through a
/// pointer there.
///
/// Through a pointer, so a projection: an unprojected place is the local itself
/// and holding a freed pointer is not using it. The span is the element's,
/// because a [`Place`] has none of its own.
pub(crate) fn dereferenced_in_element(element: &Element) -> Option<(Span, Vec<&Place>)> {
    // Every field written out, never `..`, which would let a field added to a
    // variant that already exists walk past an exhaustive match.
    match element {
        Element::Assign(operation) => {
            let mut places = projected(&operation.place);
            places.extend(dereferenced_in_rvalue(&operation.value));
            Some((operation.origin.span(), places))
        }
        // The element that exists so this can see it. `projected` rather than
        // the place itself, even though `Element::Evaluate`'s doc says a
        // producer owes a projection: one rule about what counts as reaching
        // through a pointer, applied everywhere, beats two that agree today.
        Element::Evaluate { place, origin } => Some((origin.span(), projected(place))),
        // Neither marker, nor storage beginning or ending, reads anything
        // through anything.
        Element::Sequenced { origin: _ } | Element::ArgumentsEvaluated { origin: _ } => None,
        // Nor does a conflict between two arguments: it names a local the
        // arguments' own elements and the call read.
        Element::UnsequencedAccess {
            local: _,
            written: _,
            accessed: _,
            both_write: _,
            between: _,
            origin: _,
        } => None,
        Element::StorageLive {
            local: _,
            origin: _,
        } => None,
        Element::StorageDead {
            origin: _,
            local: _,
        } => None,
    }
}

/// The same, for what a terminator reads.
pub(crate) fn dereferenced_in_terminator(terminator: &Terminator) -> Option<(Span, Vec<&Place>)> {
    match terminator {
        Terminator::Call {
            callee: _,
            arguments,
            destination,
            then: _,
            origin,
        } => {
            let mut places: Vec<&Place> = arguments.iter().flat_map(dereferenced_in).collect();
            if let Some(destination) = destination {
                places.extend(projected(destination));
            }
            Some((origin.span(), places))
        }
        // **A condition is read here and not from an element, because a
        // condition that is exactly a place never becomes one.** `if (*p + 1)`
        // computes into a temporary and the `Operation` that does it carries
        // the dereference; `if (*p)` hands the place straight to the
        // terminator. Both are one defect and the difference is whether the
        // expression needed a temporary, which is not something a reader could
        // predict, so this arm is what makes the answer the same for both.
        Terminator::Branch {
            condition,
            then: _,
            otherwise: _,
            origin,
        } => Some((origin.span(), dereferenced_in(condition))),
        Terminator::Goto(_) | Terminator::Return | Terminator::Abnormal { to: _ } => None,
    }
}

/// The same, for what an rvalue reads.
fn dereferenced_in_rvalue(value: &Rvalue) -> Vec<&Place> {
    match value {
        Rvalue::Use(operand) => dereferenced_in(operand),
        Rvalue::Unary { op: _, operand } => dereferenced_in(operand),
        Rvalue::Binary { op: _, lhs, rhs } => {
            let mut places = dereferenced_in(lhs);
            places.extend(dereferenced_in(rhs));
            places
        }
        // **Taking an address is not a dereference**, whatever the place
        // it is taken of looks like. C17 6.5.3.2 p3 is why `&*p` used to be
        // the example: "neither that operator nor the `&` operator is
        // evaluated and the result is as if both were omitted". The lowering
        // applies that clause now, so no C program reaches here with one, and
        // what does reach here is an address of a place a frontend really
        // meant to take. Reporting it would be a use of a freed value in a
        // program that never touched one.
        Rvalue::Address(_) => Vec::new(),
    }
}

/// The same, for one operand.
fn dereferenced_in(operand: &Operand) -> Vec<&Place> {
    match operand {
        Operand::Copy(place) => projected(place),
        Operand::Constant(_) => Vec::new(),
    }
}

/// The place, where it goes through a projection, and nothing where it does not.
fn projected(place: &Place) -> Vec<&Place> {
    if place.projection.is_empty() {
        Vec::new()
    } else {
        vec![place]
    }
}

/// Where this element runs, and every `*l` it reads as a value there, each
/// as the place `*l`.
///
/// **Not only what an assignment copies.** A load reaches an operator or a
/// call as an operand, and the lowering hands `*box == q`, `put(*box)` and
/// `*h = *box` their `*box` directly, with no temporary. A place reached
/// through two dereferences reads the first on the way, whether it is read
/// or written. A write to `*l` reads nothing out of it. See ADR-0057.
pub(crate) fn read_out_of_memory_in_element(element: &Element) -> Option<(Span, Vec<Place>)> {
    // Every field written out, as `dereferenced_in_element` does.
    match element {
        Element::Assign(operation) => {
            let mut places: Vec<Place> =
                first_of_two_derefs(&operation.place).into_iter().collect();
            match &operation.value {
                Rvalue::Use(operand) | Rvalue::Unary { op: _, operand } => {
                    places.extend(read_in(operand));
                }
                Rvalue::Binary { op: _, lhs, rhs } => {
                    places.extend(read_in(lhs));
                    places.extend(read_in(rhs));
                }
                // Taking an address reads no value out of the place it
                // names. The lowering applies C17 6.5.3.2 p3, so `&*p` is a
                // copy of `p` and `&**p` a copy of `*p`, an operand this
                // reads.
                Rvalue::Address(_) => {}
            }
            Some((operation.origin.span(), places))
        }
        // Converted, so read (C17 6.3.2.1 p2), which is why the lowering
        // emits it.
        Element::Evaluate { place, origin } => {
            Some((origin.span(), read_as_value(place).into_iter().collect()))
        }
        Element::Sequenced { origin: _ } | Element::ArgumentsEvaluated { origin: _ } => None,
        Element::UnsequencedAccess {
            local: _,
            written: _,
            accessed: _,
            both_write: _,
            between: _,
            origin: _,
        } => None,
        Element::StorageLive {
            local: _,
            origin: _,
        } => None,
        Element::StorageDead {
            origin: _,
            local: _,
        } => None,
    }
}

/// The same, for what a terminator reads.
pub(crate) fn read_out_of_memory_in_terminator(
    terminator: &Terminator,
) -> Option<(Span, Vec<Place>)> {
    match terminator {
        Terminator::Call {
            callee: _,
            arguments,
            destination,
            then: _,
            origin,
        } => {
            let mut places: Vec<Place> = arguments.iter().filter_map(read_in).collect();
            if let Some(destination) = destination {
                places.extend(first_of_two_derefs(destination));
            }
            Some((origin.span(), places))
        }
        Terminator::Branch {
            condition,
            then: _,
            otherwise: _,
            origin,
        } => Some((origin.span(), read_in(condition).into_iter().collect())),
        Terminator::Goto(_) | Terminator::Return | Terminator::Abnormal { to: _ } => None,
    }
}

/// The `*l` an operand reads, if it reads one.
fn read_in(operand: &Operand) -> Option<Place> {
    match operand {
        Operand::Copy(place) => read_as_value(place),
        Operand::Constant(_) => None,
    }
}

/// The `*l` a place read as a value reads: itself, where it is `*l`, or the
/// first dereference on the way to a second.
fn read_as_value(place: &Place) -> Option<Place> {
    if place.projection.as_slice() == [Projection::Deref] {
        Some(place.clone())
    } else {
        first_of_two_derefs(place)
    }
}

/// The `*l` reaching a place reads on the way, read or written: the first
/// of two dereferences at its start.
fn first_of_two_derefs(place: &Place) -> Option<Place> {
    match place.projection.as_slice() {
        [Projection::Deref, Projection::Deref, ..] => Some(Place {
            local: place.local,
            projection: vec![Projection::Deref],
        }),
        _ => None,
    }
}
