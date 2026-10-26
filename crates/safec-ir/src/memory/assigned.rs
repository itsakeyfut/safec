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
//! and is #581's; a pointer local whose address is taken is the memory
//! check's, which follows a write through an address (#640), and
//! [`address_taken`] is the one answer to which local each of them asks.
//!
//! **A read no path assigns is a proof.** One some paths assigned is
//! undefined on the others, and this analysis follows no condition a path
//! took, so `if (key && (p = find(key)) != 0) use(p);`, a loop that runs at
//! least once and a flag set beside the assignment look the same as the
//! undefined read. Reporting every such read refused programs C defines.
//!
//! **So a read some paths assigned is reported only after a branch edge that
//! certainly leads to it unassigned**, as `Unknown`: the local is unassigned
//! on every path to the branch, no path from the edge to the read assigns it,
//! and every path from the edge reaches the read. `if (c) p = q; use(p);` is
//! that read, on `c`'s false edge. Each of those programs carries an assigned
//! path through every edge before its read, or a path from the edge that
//! avoids it. The cost is an edge no execution takes: `if (n >= 0) p = q;
//! else if (n < 0) p = r; use(p);` is refused, which C defines.

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve};
use crate::ir::{
    BlockId, DOES_NOT_RETURN, Element, FuncId, Function, LocalId, Operand, Operation, Place,
    Projection, Rvalue, Terminator, TranslationUnit, Ty,
};
use crate::source::{SourceMap, Span};

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
pub(super) fn element_reads(element: &Element) -> Vec<(LocalId, Span)> {
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
                // An address reads a local only where it goes through it,
                // `&*p`: `&a[0]`, which converts an array (#621), reads
                // nothing, and `&p` reads nothing either.
                Rvalue::Address(place) => {
                    if place
                        .projection
                        .iter()
                        .any(|step| matches!(step, Projection::Deref))
                    {
                        reads.push((place.local, at));
                    }
                }
            }
            place_reads(&operation.place, at, &mut reads);
        }
        // Only a place read through reaches here: the lowering makes no
        // `Evaluate` for a bare `p;` or the left of `(p, 1)`, so those reads
        // are not seen.
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
pub(super) fn terminator_reads(terminator: &Terminator) -> Vec<(LocalId, Span)> {
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

/// Where in its block something happens: an element's index, or the
/// number of elements for the terminator.
type At = (BlockId, usize);

/// Whether `local` is assigned at `position` in `function`: an assignment to
/// the plain local, or a call's destination.
fn assigns(function: &Function, (block, index): At, local: LocalId) -> bool {
    let block = function.block(block);
    match block.elements.get(index) {
        Some(Element::Assign(operation)) => {
            operation.place.projection.is_empty() && operation.place.local == local
        }
        Some(_) => false,
        None => matches!(
            &block.terminator,
            Terminator::Call { destination: Some(place), .. }
                if place.projection.is_empty() && place.local == local
        ),
    }
}

/// Every block reachable from `from`, `from` included.
fn reachable(function: &Function, from: BlockId) -> Vec<bool> {
    let mut seen = vec![false; function.blocks().len()];
    let mut stack = vec![from];
    let mut next = Vec::new();
    while let Some(id) = stack.pop() {
        if std::mem::replace(&mut seen[id.index()], true) {
            continue;
        }
        next.clear();
        function.block(id).terminator.successors(&mut next);
        stack.extend(next.iter().copied());
    }
    seen
}

/// (b): whether a path from the start of `target` to `read` passes an
/// assignment of `local`. A `StorageLive` after it is not credited, which
/// can only leave a read unreported.
///
/// `reach` is [`reachable`] for every block the function reaches, asked once
/// per function rather than once per edge and assignment, which was cubic in
/// the blocks and cost seconds at the size the memory check still reads.
fn assigned_between(
    function: &Function,
    order: &[BlockId],
    reach: &[Vec<bool>],
    target: BlockId,
    read: At,
    local: LocalId,
) -> bool {
    let from_target = &reach[target.index()];
    order.iter().any(|&id| {
        let block = function.block(id);
        if !from_target[id.index()] {
            return false;
        }
        (0..=block.elements.len()).any(|index| {
            if !assigns(function, (id, index), local) {
                return false;
            }
            if id == read.0 && index < read.1 {
                return true;
            }
            let mut next = Vec::new();
            block.terminator.successors(&mut next);
            next.into_iter()
                .any(|after| reach[after.index()][read.0.index()])
        })
    })
}

/// (c): whether every path from the start of `target` reaches `read`'s
/// block: none ends first, at a `Return` or at a call that does not return,
/// and none goes round a loop that never enters it. Entering the block runs
/// the read, since a block's elements cannot branch.
///
/// **A call to one of [`DOES_NOT_RETURN`] ends the path even where it keeps
/// an edge**, which the lowering gives it inside a larger full expression:
/// `ok = n > 0 ? 1 : (exit(2), 0);` before the read was refused, and C
/// defines it. Mutation: read only `then`. Then
/// `a_pointer_whose_unassigned_path_ends_in_exit_inside_an_expression_is_read_in_silence`
/// is refused.
fn reaches_certainly(
    unit: &TranslationUnit,
    sources: &SourceMap,
    function: &Function,
    target: BlockId,
    read: BlockId,
) -> bool {
    // 0 unseen, 1 on the walk's stack, 2 done.
    let mut state = vec![0u8; function.blocks().len()];
    let ends_here = |callee: FuncId| {
        let name = sources.snippet(unit.function(callee).name);
        DOES_NOT_RETURN
            .iter()
            .any(|&(reserved, _)| reserved == name)
    };
    fn walk(
        function: &Function,
        id: BlockId,
        read: BlockId,
        state: &mut [u8],
        ends_here: &dyn Fn(FuncId) -> bool,
    ) -> bool {
        if id == read || state[id.index()] == 2 {
            return true;
        }
        if state[id.index()] == 1 {
            return false;
        }
        state[id.index()] = 1;
        let terminator = &function.block(id).terminator;
        let ends = match terminator {
            Terminator::Return => true,
            Terminator::Call { callee, then, .. } => then.is_none() || ends_here(*callee),
            Terminator::Goto(_) | Terminator::Branch { .. } | Terminator::Abnormal { .. } => false,
        };
        if ends {
            return false;
        }
        let mut next = Vec::new();
        terminator.successors(&mut next);
        let every = next
            .into_iter()
            .all(|after| walk(function, after, read, state, ends_here));
        state[id.index()] = 2;
        every
    }
    walk(function, target, read, &mut state, &ends_here)
}

/// Every local whose own address is taken anywhere in `function`: what this
/// analysis does not watch, since a write through that address is not
/// followed here, and what the memory check's [`Known::unwritten`] asks by
/// name instead, since it does follow one. One answer for both, so that a
/// local is asked by exactly one of them (#640).
///
/// [`Known::unwritten`]: super::known::Known::unwritten
pub(super) fn address_taken(function: &Function) -> Bits {
    let mut taken = Bits::new(function.locals().len());
    for block in function.blocks() {
        for element in &block.elements {
            if let Element::Assign(Operation {
                value: Rvalue::Address(place),
                ..
            }) = element
            {
                // **The whole local's address, or an element's.** `&a[0]` is
                // the address an array converts to (#621), and its own start.
                // `&*p` is `p`'s value and not `p`'s address.
                if !place
                    .projection
                    .iter()
                    .any(|step| matches!(step, Projection::Deref))
                {
                    taken.set(place.local.index());
                }
            }
        }
    }
    taken
}

/// Every read of a watched pointer local that no path to it assigned, each a
/// proof, and every read some path assigned that a branch edge certainly
/// leads to unassigned, each `Unknown`. See the module comment.
pub(super) fn findings(
    unit: &TranslationUnit,
    sources: &SourceMap,
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
    for local in address_taken(function).ones() {
        watched.remove(local);
    }
    if !watched.any() {
        return;
    }
    let analysis = Assigned { watched, locals };
    let cfg = Cfg::of(function);
    let solution = solve(&analysis, function, &cfg);
    let mut said: Vec<(LocalId, Span)> = Vec::new();
    // What the proof walk leaves to the edges: reads some path assigned.
    let mut doubted: Vec<(LocalId, Span, At)> = Vec::new();
    // Per block, what is assigned where its terminator runs, for (a).
    let mut at_end: Vec<Option<State>> = vec![None; function.blocks().len()];
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
        for (index, element) in block.elements.iter().enumerate() {
            for (local, at) in element_reads(element) {
                if value[local.index()] {
                    doubted.push((local, at, (id, index)));
                }
                report(local, at, value[local.index()], findings);
            }
            analysis.element(function, element, &mut value);
        }
        for (local, at) in terminator_reads(&block.terminator) {
            if value[local.index()] {
                doubted.push((local, at, (id, block.elements.len())));
            }
            report(local, at, value[local.index()], findings);
        }
        at_end[id.index()] = Some(value);
    }

    let mut reach = vec![Vec::new(); function.blocks().len()];
    if !doubted.is_empty() {
        for &id in cfg.order() {
            reach[id.index()] = reachable(function, id);
        }
    }
    for (local, at, read) in doubted {
        if !analysis.watched.get(local.index()) || said.contains(&(local, at)) {
            continue;
        }
        let witness = cfg.order().iter().find_map(|&id| {
            let Terminator::Branch {
                then,
                otherwise,
                origin,
                ..
            } = &function.block(id).terminator
            else {
                return None;
            };
            // Both edges to one block decide nothing, and "when this is
            // false" would be a claim about a condition that changed nothing.
            // **No case holds this**, measured: the lowering builds no such
            // branch from C.
            if then == otherwise {
                return None;
            }
            // (a)
            if at_end[id.index()].as_ref()?[local.index()] {
                return None;
            }
            [(*then, true), (*otherwise, false)]
                .into_iter()
                .find(|&(target, _)| {
                    !assigned_between(function, cfg.order(), &reach, target, read, local)
                        && reaches_certainly(unit, sources, function, target, read.0)
                })
                .map(|(_, taken)| (origin.span(), taken))
        });
        let Some((edge, taken)) = witness else {
            continue;
        };
        said.push((local, at));
        findings.push(Finding {
            function: func,
            kind: Kind::SometimesUnassigned { edge, taken },
            conclusion: Conclusion::Unknown,
            at,
            local: Some(local),
            freed: None,
            made: None,
            unproven: None,
        });
    }
}
