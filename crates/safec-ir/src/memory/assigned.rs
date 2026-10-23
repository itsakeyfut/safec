//! Whether a pointer local has been assigned before it is read.
//!
//! A local's initial value is indeterminate (C17 6.2.4 p6), and reading a
//! local whose address is never taken while it is indeterminate is undefined
//! (6.3.2.1 p2), whatever the read is for: a comparison, a copy, a
//! dereference, a free. So `int *p; if (p != 0) { *p = 1; }` writes through
//! a pointer that points at no object, and the null test that the
//! nullability check reads as making `p` not null is itself the undefined
//! read.
//!
//! Its own small analysis rather than part of `Allocations`, whose value is
//! square in the locals, or of the nullability check, whose answer after a
//! passed null test is exactly the belief that was wrong here. A pointer read
//! out of an allocation nothing wrote is the same question asked of a slot,
//! and is #581's.
//!
//! **Only a read no path assigns.** One where some paths assigned the local
//! is undefined on the others, and is not reported: this analysis does not
//! follow the condition a path took, so `if (key && (p = find(key)) != 0)
//! use(p);`, a loop that runs at least once and a flag set beside the
//! assignment look the same as the undefined read, and reporting it refused
//! programs C defines that clang and gcc build in silence. Telling them apart
//! is #582's.

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve};
use crate::ir::{
    Element, FuncId, Function, LocalId, Operand, Operation, Place, Rvalue, Terminator,
    TranslationUnit, Ty,
};
use crate::source::Span;

use super::bits::Bits;
use super::{Finding, Kind};

/// The analysis over one function.
struct Assigned {
    /// The locals this asks about: named, of pointer type, never a
    /// parameter, and with no `&` taken of them anywhere in the function. A
    /// write through an address is not followed here, so a local whose address
    /// is taken may be assigned where this cannot see, and 6.3.2.1 p2 is about
    /// the locals whose address is not taken in any case.
    watched: Bits,
    locals: usize,
}

/// Per local: assigned on some path that reaches here.
type State = Vec<bool>;

impl Analysis for Assigned {
    type Value = State;

    /// One step per local: its bit rises to `true` once, at a join. A
    /// transfer that resets a local rebuilds from the value it was given
    /// rather than lowering one.
    fn height(&self, _function: &Function) -> usize {
        self.locals + 1
    }

    /// Nothing is assigned where the function starts but its parameters,
    /// which the caller gave a value. A parameter is never watched, so this
    /// is the same as every local unassigned, and says why.
    fn on_entry(&self) -> State {
        vec![false; self.locals]
    }

    /// Assigned on some path where either is.
    fn join(&self, into: &mut State, from: &State) {
        for (here, there) in into.iter_mut().zip(from) {
            *here |= there;
        }
    }

    fn element(&self, _function: &Function, element: &Element, value: &mut State) {
        match element {
            Element::Assign(operation) if operation.place.projection.is_empty() => {
                value[operation.place.local.index()] = true;
            }
            // Each entry to the block a local is declared in is a new object,
            // indeterminate again (C17 6.2.4 p6), so a pointer assigned late
            // in one turn of a loop is not assigned at the start of the next.
            Element::StorageLive { local, .. } => {
                value[local.index()] = false;
            }
            _ => {}
        }
    }

    fn terminator(&self, _function: &Function, terminator: &Terminator, value: &mut State) {
        if let Terminator::Call {
            destination: Some(place),
            ..
        } = terminator
        {
            if place.projection.is_empty() {
                value[place.local.index()] = true;
            }
        }
    }
}

/// Every local an element reads, and where. Its write, if it has one, comes
/// after: `p = p + 1;` reads `p` first.
fn element_reads(element: &Element) -> Vec<(LocalId, Span)> {
    let mut reads = Vec::new();
    match element {
        Element::Assign(operation) => {
            let at = operation.origin.span();
            match &operation.value {
                Rvalue::Use(operand) | Rvalue::Unary { operand, .. } => {
                    operand_reads(operand, at, &mut reads)
                }
                Rvalue::Binary { lhs, rhs, .. } => {
                    operand_reads(lhs, at, &mut reads);
                    operand_reads(rhs, at, &mut reads);
                }
                Rvalue::Address(place) => place_reads(place, at, &mut reads),
            }
            place_reads(&operation.place, at, &mut reads);
        }
        Element::Evaluate { place, origin } => {
            reads.push((place.local, origin.span()));
        }
        // A marker, and an access the lowering already lowered as its own
        // reads and writes.
        Element::StorageLive { .. }
        | Element::StorageDead { .. }
        | Element::Sequenced { .. }
        | Element::ArgumentsEvaluated { .. }
        | Element::UnsequencedAccess { .. } => {}
    }
    reads
}

/// Every local a terminator reads, and where.
fn terminator_reads(terminator: &Terminator) -> Vec<(LocalId, Span)> {
    let mut reads = Vec::new();
    match terminator {
        Terminator::Branch {
            condition, origin, ..
        } => operand_reads(condition, origin.span(), &mut reads),
        Terminator::Call {
            arguments,
            destination,
            origin,
            ..
        } => {
            for argument in arguments {
                operand_reads(argument, origin.span(), &mut reads);
            }
            if let Some(place) = destination {
                place_reads(place, origin.span(), &mut reads);
            }
        }
        Terminator::Goto(_) | Terminator::Return | Terminator::Abnormal { .. } => {}
    }
    reads
}

fn operand_reads(operand: &Operand, at: Span, reads: &mut Vec<(LocalId, Span)>) {
    if let Operand::Copy(place) = operand {
        reads.push((place.local, at));
    }
}

/// A place reads its local where it goes through it: `*p = 1;` reads `p`,
/// and `p = 1;` does not.
fn place_reads(place: &Place, at: Span, reads: &mut Vec<(LocalId, Span)>) {
    if !place.projection.is_empty() {
        reads.push((place.local, at));
    }
}

/// Every read of a watched pointer local that no path to it assigned, each a
/// proof.
pub(super) fn findings(
    unit: &TranslationUnit,
    func: FuncId,
    function: &Function,
    findings: &mut Vec<Finding>,
) {
    let locals = function.locals().len();
    let mut watched = Bits::new(locals);
    let parameters: Vec<LocalId> = function.parameters().collect();
    for local in function.locals() {
        if function.name_of(local).is_some()
            && !parameters.contains(&local)
            && matches!(unit.ty(function.local(local)), Ty::Pointer(_))
        {
            watched.set(local.index());
        }
    }
    for block in function.blocks() {
        for element in &block.elements {
            if let Element::Assign(Operation {
                value: Rvalue::Address(place),
                ..
            }) = element
            {
                if place.projection.is_empty() {
                    watched.remove(place.local.index());
                }
            }
        }
    }
    if !watched.any() {
        return;
    }
    let analysis = Assigned { watched, locals };
    let cfg = Cfg::of(function);
    let solution = solve(&analysis, function, &cfg);
    let mut said: Vec<(LocalId, Span)> = Vec::new();
    let mut report = |local: LocalId, at: Span, assigned: bool, findings: &mut Vec<Finding>| {
        if !analysis.watched.get(local.index()) || assigned || said.contains(&(local, at)) {
            return;
        }
        said.push((local, at));
        findings.push(Finding {
            function: func,
            kind: Kind::Unassigned,
            conclusion: Conclusion::Unsafe,
            at,
            local: Some(local),
            freed: None,
            made: None,
            unproven: None,
        });
    };
    for &id in cfg.order() {
        let Some(start) = solution.value(id) else {
            continue;
        };
        let mut value = start.clone();
        let block = function.block(id);
        for element in &block.elements {
            for (local, at) in element_reads(element) {
                report(local, at, value[local.index()], findings);
            }
            analysis.element(function, element, &mut value);
        }
        for (local, at) in terminator_reads(&block.terminator) {
            report(local, at, value[local.index()], findings);
        }
    }
}
