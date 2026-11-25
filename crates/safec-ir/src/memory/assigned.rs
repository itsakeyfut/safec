//! Whether a pointer local, or a struct local holding a pointer, has been
//! assigned before it is read.
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
//! undefined read.
//!
//! **So this analysis reports a read some paths assigned only after a branch
//! edge that certainly leads to it unassigned**, as `Unknown`: the local is unassigned
//! on every path to the branch, no path from the edge to the read assigns it,
//! and every path from the edge reaches the read. `if (c) p = q; use(p);` is
//! that read, on `c`'s false edge. Each of those programs carries an assigned
//! path through every edge before its read, or a path from the edge that
//! avoids it. The cost is an edge no execution takes: `if (n >= 0) p = q;
//! else if (n < 0) p = r; use(p);` is refused, which C defines.
//!
//! **Every other read some paths assigned is the memory check's doubt**, as
//! it is for a local whose address is taken: `report::unwritten_read` asks
//! `Known::written`, which is `Maybe` there, where nothing here was said at the
//! caret. So the flag set beside the assignment, a test made twice and a
//! read on a later turn of a loop are doubted, as programs C may define; the
//! edge above is what this analysis adds, a place to say why. See ADR-0075.
//!
//! **A struct or union local is watched too** (#1302), where it holds a
//! pointer somewhere and the program never takes its address, nor a
//! member's. The lowering writes `u = t`, passing `t` by value and
//! `return t` each as a `memcpy` from an address of `t` that it makes
//! itself (`Origin::Generated`), and it reaches a member through such an
//! address too. An address only the compiler takes is not one that 6.3.2.1
//! p2's "could have been declared with the `register` storage class"
//! counts, so `t` is still held to p2. `taken::address_taken` still counts
//! those addresses, since the other checks that ask it do not follow them;
//! this check follows them itself and adds the structs back.
//!
//! [`Watch`] says which structs, and what each use of the lowering's
//! addresses is. The `memcpy`'s source is a read of `t` whole. Its
//! destination is a write. A write into a member is an assignment
//! performed into `t`, which is the reading the design settled on: a
//! struct with one member written is not reported when it is copied.
//! Taking
//! a member's address writes nothing, since a member read goes through it
//! too. Any other use of such an address lets it go where this does not
//! follow it, and `t` is not watched.
//!
//! **A loop's own test is such a branch as of its first turn**, unless
//! `runs::shown_to_run` shows the loop runs (#783): its edge out of the loop
//! is asked against what arrived from before the loop, since the turns that
//! came round assigned it. So `while (i < n) q = &x; r = q;` is the same
//! doubt, at `i < n` when it is false, and a loop C runs that this cannot
//! show runs is the cost, as it is in the memory check. See ADR-0064.

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve, solve_first_turn_within};
use crate::ir::{
    BlockId, Called, DOES_NOT_RETURN, Element, FuncId, Function, Intrinsic, LocalId, Operand,
    Origin, Place, Projection, Rvalue, Shape, Terminator, TranslationUnit, Ty,
};
use crate::source::{SourceMap, Span};
use std::collections::HashMap;

use super::bits::Bits;
use super::index::LocalIdx;
use super::runs;
use super::{Finding, Kind};

/// The analysis over one function.
struct Assigned {
    /// The locals this asks about: named, of pointer type, never a
    /// parameter, and with no `&` taken of them anywhere in the function. A
    /// write through an address is not followed here, so a local whose address
    /// is taken may be assigned where this cannot see, and 6.3.2.1 p2 is about
    /// the locals whose address is not taken in any case. And the struct
    /// locals [`Watch`] follows, whose only addresses are the lowering's.
    watched: Bits<LocalIdx>,
    locals: usize,
    /// The watched struct locals, by the generated addresses that reach
    /// them. See [`Watch::structs`].
    structs: Watch,
    /// The watched lists of variadic arguments. See [`Lists::of`].
    lists: Lists,
}

/// The local lists of variadic arguments watched here, and the
/// temporaries holding the address of each that the operations on lists
/// are handed (#1214).
///
/// **A list is assigned by `__builtin_va_start` or by `__builtin_va_copy`
/// into it**, and read by `__builtin_va_arg`, `__builtin_va_end` and
/// `__builtin_va_copy` out of it, which C17 7.16.1 p3 and 7.16.1.4 p3 make
/// undefined before it was started: a read of what nothing wrote, as a
/// pointer's is. Its address is taken for those operations alone, so a
/// local whose address goes anywhere else is not watched, as any local
/// whose address is taken is not. See ADR-0126.
#[derive(Default)]
struct Lists {
    /// Each list, by the temporary holding its address.
    by_address: HashMap<LocalId, LocalId>,
    /// The lists, in no order.
    lists: Vec<LocalId>,
}

impl Lists {
    /// The named locals of `function` that are lists: read by
    /// `Rvalue::VaArg` or handed to an intrinsic, with every address taken
    /// of them handed to intrinsics and nowhere else.
    fn of(function: &Function) -> Self {
        let mut by_address = HashMap::new();
        let mut handed: Vec<LocalId> = Vec::new();
        let mut lists: Vec<LocalId> = Vec::new();
        for block in function.blocks() {
            for element in &block.elements {
                if let Element::Assign(operation) = element {
                    if let (Shape::Local, Rvalue::Address(taken)) =
                        (operation.place.shape(), &operation.value)
                    {
                        if taken.steps().is_empty() {
                            by_address.insert(operation.place.local, taken.local);
                        }
                    }
                    if let Rvalue::VaArg(list) = &operation.value {
                        if list.steps().is_empty() {
                            lists.push(list.local);
                        }
                    }
                }
            }
            if let Terminator::Call {
                callee: Called::Intrinsic(_),
                arguments,
                ..
            } = &block.terminator
            {
                for argument in arguments {
                    if let Operand::Copy(place) = argument {
                        if let Some(&list) = by_address.get(&place.local) {
                            handed.push(place.local);
                            lists.push(list);
                        }
                    }
                }
            }
        }
        // An address read anywhere but as an intrinsic's argument lets the
        // list be written where this does not look.
        let escapes = |temporary: LocalId| {
            function.blocks().any(|block| {
                block.elements.iter().any(|element| {
                    element_reads(element)
                        .iter()
                        .any(|&(local, _)| local == temporary)
                }) || match &block.terminator {
                    Terminator::Call {
                        callee: Called::Intrinsic(_),
                        ..
                    } => false,
                    terminator => terminator_reads(terminator)
                        .iter()
                        .any(|&(local, _)| local == temporary),
                }
            })
        };
        lists.sort_by_key(|local| local.index());
        lists.dedup();
        lists.retain(|&list| {
            function.name_of(list).is_some()
                && by_address
                    .iter()
                    .filter(|&(_, &of)| of == list)
                    .all(|(&temporary, _)| !escapes(temporary))
        });
        by_address.retain(|temporary, list| lists.contains(list) && handed.contains(temporary));
        Lists { by_address, lists }
    }

    /// The list `argument` is the address of, where it is one watched.
    fn list(&self, argument: Option<&Operand>) -> Option<LocalId> {
        match argument? {
            Operand::Copy(place) if place.steps().is_empty() => {
                self.by_address.get(&place.local).copied()
            }
            _ => None,
        }
    }

    /// The list `terminator` starts, or copies into. Mutation: answer
    /// nothing; `a_list_read_before_it_was_started_is_reported`'s
    /// `started` reports `SC0411`.
    fn written(&self, terminator: &Terminator) -> Option<LocalId> {
        match terminator {
            Terminator::Call {
                callee: Called::Intrinsic(Intrinsic::VaStart | Intrinsic::VaCopy),
                arguments,
                ..
            } => self.list(arguments.first()),
            _ => None,
        }
    }

    /// The lists `terminator` reads: the one it ends, or the one it copies
    /// out of, at the call.
    fn read(&self, terminator: &Terminator) -> Vec<(LocalId, Span)> {
        let Terminator::Call {
            callee: Called::Intrinsic(intrinsic),
            arguments,
            origin,
            ..
        } = terminator
        else {
            return Vec::new();
        };
        let read = match intrinsic {
            Intrinsic::VaStart => None,
            Intrinsic::VaEnd => self.list(arguments.first()),
            Intrinsic::VaCopy => self.list(arguments.get(1)),
        };
        read.map(|list| (list, origin.span())).into_iter().collect()
    }
}

/// The struct locals watched here, and the temporaries the lowering reaches
/// each through (#1302). See the module comment.
///
/// **Two kinds of temporary.** `whole` holds the lowering's own address of
/// a watched struct, `&t`, which it makes for a struct copy and for a
/// member's access. `member` holds an address of one of its members taken
/// through one of those, `&(*_3).p`, which the lowering writes into a
/// temporary of its own. A program that keeps such an address, `int **q =
/// &t.p;`, copies it out of that temporary, a use this does not follow, so
/// `t` is not watched. A write through either, or a
/// struct copy into either, is an assignment performed into the struct; a
/// copy out of a `whole` one is a read of it whole; and taking a member's
/// address assigns nothing, since a member read goes through it too.
/// Mutation: count a member's address as an assignment;
/// `a_struct_local_nothing_wrote_is_reported_where_it_is_copied`'s
/// `member_read_first` goes silent.
#[derive(Default)]
struct Watch {
    /// Each generated temporary holding a watched struct's own address.
    whole: HashMap<LocalId, LocalId>,
    /// Each temporary holding an address of a member of a watched struct,
    /// taken through one of `whole` or of these.
    member: HashMap<LocalId, LocalId>,
    /// The function the lowering's struct copy calls, where the unit has
    /// one: found by name, and the one the program declares, since the
    /// lowering calls that one (a definition of it is refused).
    memcpy: Option<FuncId>,
}

impl Watch {
    /// The struct locals of `function` to watch, and how each is reached:
    /// named, not a parameter, of struct or union type holding a pointer
    /// somewhere, every address of it the lowering's own, and every use of
    /// every such temporary one followed here. Mutation: watch none; the
    /// case `a_struct_local_nothing_wrote_is_reported_where_it_is_copied`
    /// goes silent.
    fn structs(unit: &TranslationUnit, sources: &SourceMap, function: &Function) -> Watch {
        let memcpy = unit
            .functions()
            .find(|&id| unit.function(id).spelled(sources) == "memcpy");
        let parameters: Vec<LocalId> = function.parameters().collect();
        // Each candidate, and whether it is still one.
        let mut candidates: HashMap<LocalId, bool> = function
            .locals()
            .filter(|&local| {
                function.name_of(local).is_some()
                    && !parameters.contains(&local)
                    && matches!(unit.ty(function.local(local)), Ty::Struct(_))
                    && crate::layout::stored_pointer(unit, function.local(local)).is_some()
            })
            .map(|local| (local, true))
            .collect();
        let mut watch = Watch {
            memcpy,
            ..Watch::default()
        };
        if candidates.is_empty() {
            return watch;
        }
        let through_a_pointer = |place: &Place| {
            place.steps().iter().any(|step| match step {
                Projection::Deref => true,
                Projection::Index(_) | Projection::Field(_) => false,
            })
        };
        // Every address of a candidate: the lowering's, of the whole, into
        // a temporary, or it is no candidate; and every member's address
        // through one of those, into a temporary, or the address has left
        // where this follows it. Until nothing more is found,
        // since a member's address may be taken through another's.
        loop {
            let mut grown = false;
            for block in function.blocks() {
                for element in &block.elements {
                    let Element::Assign(operation) = element else {
                        continue;
                    };
                    let Rvalue::Address(place) = &operation.value else {
                        continue;
                    };
                    let into = operation.place.local;
                    let whole = operation.place.steps().is_empty();
                    if let Some(keep) = candidates.get_mut(&place.local) {
                        if through_a_pointer(place) {
                            continue;
                        }
                        let generated = matches!(operation.origin, Origin::Generated(_));
                        if generated && place.steps().is_empty() && whole {
                            grown |= watch.whole.insert(into, place.local).is_none();
                        } else {
                            *keep = false;
                        }
                        continue;
                    }
                    let Some(&of) = watch
                        .whole
                        .get(&place.local)
                        .or_else(|| watch.member.get(&place.local))
                    else {
                        continue;
                    };
                    if whole {
                        grown |= watch.member.insert(into, of).is_none();
                    } else {
                        candidates.insert(of, false);
                    }
                }
            }
            if !grown {
                break;
            }
        }
        // And every use of such a temporary one followed here: a place
        // through it, or the source or destination of the struct copy.
        // Any other lets the address go where this does not follow it.
        let held = |local: LocalId| {
            watch
                .whole
                .get(&local)
                .or_else(|| watch.member.get(&local))
                .copied()
        };
        let escapes = |operand: &Operand| match operand {
            Operand::Copy(place) if place.steps().is_empty() => held(place.local),
            _ => None,
        };
        let mut lost = Vec::new();
        for block in function.blocks() {
            for element in &block.elements {
                let Element::Assign(operation) = element else {
                    continue;
                };
                // Written again after the address that made it.
                if operation.place.steps().is_empty()
                    && !matches!(operation.value, Rvalue::Address(_))
                {
                    lost.extend(held(operation.place.local));
                }
                match &operation.value {
                    Rvalue::Use(operand)
                    | Rvalue::Unary { operand, .. }
                    | Rvalue::Reinterpret(operand) => {
                        lost.extend(escapes(operand));
                    }
                    Rvalue::Binary { lhs, rhs, .. } => {
                        lost.extend(escapes(lhs));
                        lost.extend(escapes(rhs));
                    }
                    // A list holds nothing this check watches.
                    Rvalue::Address(_) | Rvalue::Static(_) | Rvalue::VaArg(_) => {}
                }
            }
            match &block.terminator {
                Terminator::Call {
                    callee,
                    arguments,
                    destination,
                    ..
                } => {
                    // The copy's destination and source, its first two
                    // arguments, are followed; its length, and anything
                    // handed to any other call, is not.
                    for (index, argument) in arguments.iter().enumerate() {
                        if callee.direct().is_some() && callee.direct() == memcpy && index < 2 {
                            continue;
                        }
                        lost.extend(escapes(argument));
                    }
                    // A pointer called through is a use this does not follow.
                    lost.extend(callee.operand().and_then(escapes));
                    if let Some(place) = destination {
                        if place.steps().is_empty() {
                            lost.extend(held(place.local));
                        }
                    }
                }
                Terminator::Branch { condition, .. } => lost.extend(escapes(condition)),
                Terminator::Goto(_) | Terminator::Return | Terminator::Abnormal { .. } => {}
            }
        }
        for local in lost {
            candidates.insert(local, false);
        }
        let kept = |local: &LocalId| candidates.get(local).copied().unwrap_or(false);
        watch.whole.retain(|_, local| kept(local));
        watch.member.retain(|_, local| kept(local));
        watch
    }

    /// Every struct watched.
    fn watched(&self) -> impl Iterator<Item = LocalId> + '_ {
        self.whole.values().copied()
    }

    /// The struct a write to `place` assigns, where it is one through a
    /// temporary of either kind: `(*_3).p = ...` or `*_4 = ...`.
    fn written(&self, place: &Place) -> Option<LocalId> {
        if place.steps().is_empty() {
            return None;
        }
        self.whole
            .get(&place.local)
            .or_else(|| self.member.get(&place.local))
            .copied()
    }

    /// The `memcpy` the lowering's struct copy is, and its destination and
    /// source temporaries.
    fn copy<'t>(&self, terminator: &'t Terminator) -> Option<(&'t Operand, &'t Operand, Span)> {
        let Terminator::Call {
            callee,
            arguments,
            origin,
            ..
        } = terminator
        else {
            return None;
        };
        if callee.direct().is_none() || callee.direct() != self.memcpy {
            return None;
        }
        Some((arguments.first()?, arguments.get(1)?, origin.span()))
    }

    /// The struct the lowering's copy reads whole at `terminator`, and
    /// where: its source is the struct's own address.
    fn copied_from(&self, terminator: &Terminator) -> Option<(LocalId, Span)> {
        let (_, source, at) = self.copy(terminator)?;
        match source {
            Operand::Copy(place) if place.steps().is_empty() => {
                self.whole.get(&place.local).map(|&local| (local, at))
            }
            _ => None,
        }
    }

    /// The struct the lowering's copy writes at `terminator`, whole or a
    /// member of it.
    fn copied_into(&self, terminator: &Terminator) -> Option<LocalId> {
        let (destination, _, _) = self.copy(terminator)?;
        match destination {
            Operand::Copy(place) if place.steps().is_empty() => self
                .whole
                .get(&place.local)
                .or_else(|| self.member.get(&place.local))
                .copied(),
            _ => None,
        }
    }
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
        // A watched struct written through a temporary that reaches it,
        // whole or a member: an assignment performed (#1302). A member's
        // address taken is not one; what is then written through it is.
        if let Element::Assign(operation) = element {
            if let Some(local) = self.structs.written(&operation.place) {
                value[local.index()] = true;
            }
        }
        match element {
            Element::Assign(operation)
                if (match operation.place.shape() {
                    Shape::Local => true,
                    Shape::Deref(_) | Shape::Indexed => false,
                }) =>
            {
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
        if let Some(local) = self.structs.copied_into(terminator) {
            value[local.index()] = true;
        }
        if let Some(list) = self.lists.written(terminator) {
            value[list.index()] = true;
        }
        if let Terminator::Call {
            destination: Some(place),
            ..
        } = terminator
        {
            if match place.shape() {
                Shape::Local => true,
                Shape::Deref(_) | Shape::Indexed => false,
            } {
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
                Rvalue::Use(operand)
                | Rvalue::Unary { operand, .. }
                | Rvalue::Reinterpret(operand) => operand_reads(operand, at, &mut reads),
                Rvalue::Binary { lhs, rhs, .. } => {
                    operand_reads(lhs, at, &mut reads);
                    operand_reads(rhs, at, &mut reads);
                }
                // The address of a static object reads no local.
                Rvalue::Static(_) => {}
                // **The list is read, a plain local's too**, which is where
                // it is read before `__builtin_va_start`; through a pointer,
                // the pointer is (#1214). Mutation: read only through a
                // projection, as `place_reads` does; `before_start` in
                // `a_list_read_before_it_was_started_is_reported` goes
                // silent.
                Rvalue::VaArg(place) => reads.push((place.local, at)),
                // An address reads a local only where it goes through it,
                // `&*p`: `&a[0]`, which converts an array (#621), reads
                // nothing, and `&p` reads nothing either.
                Rvalue::Address(place) => {
                    if place.steps().iter().any(|step| match step {
                        Projection::Deref => true,
                        // `&s.m` reads nothing, as `&a[0]` does (#1200).
                        Projection::Index(_) | Projection::Field(_) => false,
                    }) {
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
        // **And the pointer a call goes through**, read as a dereference
        // reads it: a call through a pointer nothing assigned is the read
        // 6.3.2.1 p2 makes undefined. See ADR-0125. Mutation: read the
        // arguments alone; `a_call_through_a_function_pointer_is_read_as_a_call_this_check_cannot_read`'s
        // `through_an_unassigned_pointer` goes silent.
        Terminator::Call {
            callee,
            arguments,
            destination,
            then: _,
            origin,
        } => {
            for argument in arguments.iter().chain(callee.operand()) {
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
    if !(match place.shape() {
        Shape::Local => true,
        Shape::Deref(_) | Shape::Indexed => false,
    }) {
        reads.push((place.local, at));
    }
}

/// Where in its block something happens: an element's index, or the
/// number of elements for the terminator.
type At = (BlockId, usize);

/// Whether `local` is assigned at `position` in `function`: an assignment to
/// the plain local, or a call's destination.
///
/// **And a watched struct written through the lowering's temporaries**, as
/// the transfer counts it (#1302): the doubt walk and the analysis are to
/// agree, or a struct both arms wrote is doubted on the arm that wrote it.
/// Mutation: ask only the local itself here;
/// `a_struct_local_nothing_wrote_is_reported_where_it_is_copied`'s
/// `written_on_both_arms` is doubted.
fn assigns(
    function: &Function,
    (structs, lists): (&Watch, &Lists),
    (block, index): At,
    local: LocalId,
) -> bool {
    let block = function.block(block);
    match block.elements.get(index) {
        Some(Element::Assign(operation)) => {
            ((match operation.place.shape() {
                Shape::Local => true,
                Shape::Deref(_) | Shape::Indexed => false,
            }) && operation.place.local == local)
                || structs.written(&operation.place) == Some(local)
        }
        Some(_) => false,
        None => {
            matches!(
                &block.terminator,
                Terminator::Call { destination: Some(place), .. }
                    if (match place.shape() { Shape::Local => true, Shape::Deref(_) | Shape::Indexed => false }) && place.local == local
            ) || structs.copied_into(&block.terminator) == Some(local)
                // **And a list started or copied into**, as the transfer
                // counts it, so that the two agree (#1214).
                || lists.written(&block.terminator) == Some(local)
        }
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
    watched: (&Watch, &Lists),
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
            if !assigns(function, watched, (id, index), local) {
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
/// `a_pointer_whose_unassigned_path_ends_in_exit_inside_an_expression_is_doubted`
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
        let name = unit.function(callee).spelled(sources);
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
            // A call through a pointer is believed to return, as any call
            // this check cannot name is.
            Terminator::Call { callee, then, .. } => {
                then.is_none() || callee.direct().is_some_and(ends_here)
            }
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

/// Every local whose own address is taken. At crate level, so that another
/// analysis asks the same question this one answers. See #895.
pub(super) use crate::taken::address_taken;

/// Every read of a watched local that no path to it assigned, each a
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
            && unit.ty(function.local(local)).is_pointer()
        {
            watched.set(LocalIdx::of(local));
        }
    }
    for local in address_taken(function).ones() {
        watched.remove(local);
    }
    let structs = Watch::structs(unit, sources, function);
    for local in structs.watched() {
        watched.set(LocalIdx::of(local));
    }
    let lists = Lists::of(function);
    for &list in &lists.lists {
        watched.set(LocalIdx::of(list));
    }
    if !watched.any() {
        return;
    }
    let analysis = Assigned {
        watched,
        locals,
        structs,
        lists,
    };
    let cfg = Cfg::of(function);
    let solution = solve(&analysis, function, &cfg);
    let mut said: Vec<(LocalId, Span)> = Vec::new();
    // What the proof walk leaves to the edges: reads some path assigned.
    let mut doubted: Vec<(LocalId, Span, At)> = Vec::new();
    // Per block, what is assigned where its terminator runs, for (a).
    let mut at_end: Vec<Option<State>> = vec![None; function.blocks().len()];
    let mut report = |local: LocalId, at: Span, assigned: bool, findings: &mut Vec<Finding>| {
        if !analysis.watched.get(LocalIdx::of(local)) || assigned || said.contains(&(local, at)) {
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
    // **And every read a loop runs on every turn, as of its first turn**
    // (#670): whatever turn reaches it, the first passed it, so `r = q; q =
    // &x;` in the body is a proof. Only where the read's block dominates every
    // block the loop's back edges leave from. A read on an arm is not asked
    // so here: the memory check's first turn doubts it, `if (i > 0) use(p);
    // p = find(i);` included (ADR-0075). Not bounded, for the reason this
    // analysis's own walk is not. See ADR-0057.
    let loops = cfg.loops();
    let dominators = cfg.dominators();
    let first_turns: Vec<_> = loops
        .iter()
        .map(|each| {
            let solution =
                solve_first_turn_within(&analysis, function, &cfg, each.head(), usize::MAX)
                    .expect("no walk count is past `usize::MAX`");
            (each, solution)
        })
        .collect();
    for &id in cfg.order() {
        let Some(start) = solution.value(id) else {
            continue;
        };
        let mut value = start.clone();
        let block = function.block(id);
        // A loop's first turn is asked only where the block dominates every
        // latch, as in `memory.rs`'s first-turn walk: a block after a
        // `continue` in a `while` dominates one of two, and may first run
        // after a turn the `continue` ended.
        // `a_pointer_a_turn_ended_by_continue_may_have_assigned_is_doubted`
        // is proved under `any`.
        let mut turns: Vec<State> = first_turns
            .iter()
            .filter(|(each, _)| {
                each.contains(id)
                    && each
                        .latches()
                        .iter()
                        .all(|&latch| dominators.dominates(id, latch))
            })
            .filter_map(|(_, solution)| solution.value(id).cloned())
            .collect();
        for (index, element) in block.elements.iter().enumerate() {
            for (local, at) in element_reads(element) {
                if value[local.index()] {
                    doubted.push((local, at, (id, index)));
                }
                report(local, at, value[local.index()], findings);
                for turn in &turns {
                    report(local, at, turn[local.index()], findings);
                }
            }
            analysis.element(function, element, &mut value);
            for turn in &mut turns {
                analysis.element(function, element, turn);
            }
        }
        let copied = analysis.structs.copied_from(&block.terminator);
        for (local, at) in terminator_reads(&block.terminator)
            .into_iter()
            .chain(copied)
            .chain(analysis.lists.read(&block.terminator))
        {
            if value[local.index()] {
                doubted.push((local, at, (id, block.elements.len())));
            }
            report(local, at, value[local.index()], findings);
            for turn in &turns {
                report(local, at, turn[local.index()], findings);
            }
        }
        at_end[id.index()] = Some(value);
    }

    // **And at the head of a loop not shown to run, as of its first turn**
    // (#783): the every-turn state there holds what the back edge assigned,
    // so (a) never held at a loop's own test, and `while (i < n) q = &x; r =
    // q;` read an unassigned `q` in silence when `n <= 0`. Only the edge that
    // leaves the loop is asked this way, since the first turn is the one
    // that may take it before the body ran. A loop shown to run takes that
    // edge on no first turn. See ADR-0064.
    let taken = address_taken(function);
    let mut first_turn_at_end: Vec<Option<(State, BlockId)>> = vec![None; function.blocks().len()];
    for (each, solution) in &first_turns {
        let head = each.head();
        let Terminator::Branch {
            then, otherwise, ..
        } = &function.block(head).terminator
        else {
            continue;
        };
        let leaves = match (each.contains(*then), each.contains(*otherwise)) {
            (true, false) => *otherwise,
            (false, true) => *then,
            _ => continue,
        };
        if runs::shown_to_run(unit, function, &cfg, each, &taken).is_some() {
            continue;
        }
        let Some(start) = solution.value(head) else {
            continue;
        };
        let mut value = start.clone();
        for element in &function.block(head).elements {
            analysis.element(function, element, &mut value);
        }
        first_turn_at_end[head.index()] = Some((value, leaves));
    }

    let mut reach = vec![Vec::new(); function.blocks().len()];
    if !doubted.is_empty() {
        for &id in cfg.order() {
            reach[id.index()] = reachable(function, id);
        }
    }
    for (local, at, read) in doubted {
        if !analysis.watched.get(LocalIdx::of(local)) || said.contains(&(local, at)) {
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
            // **(a) is `every_edge`, and written once**: the local unassigned
            // where the branch is, so either edge may witness; or, at a loop's
            // head, unassigned there as of the first turn, so only the edge
            // that leaves the loop may. An early return beside it answered
            // nothing this filter did not, and no test could hold either copy
            // alone (#848). Mutation: `every_edge` always `true`; about a
            // thousand cases fail.
            //
            // **The first-turn edge alone is held by no case**: widened to
            // either edge of the head, nothing moves, measured. The edge into
            // the body reaches a read after the loop only around the back
            // edge, which `reaches_certainly` refuses, and a read it reaches
            // certainly inside the body is one the first-turn proof already
            // reported at that caret. It is kept to name the edge the doubt is
            // about.
            let first_turn = first_turn_at_end[id.index()]
                .as_ref()
                .filter(|(value, _)| !value[local.index()])
                .map(|&(_, leaves)| leaves);
            let every_edge = !at_end[id.index()].as_ref()?[local.index()];
            [(*then, true), (*otherwise, false)]
                .into_iter()
                .filter(|&(target, _)| every_edge || Some(target) == first_turn)
                .find(|&(target, _)| {
                    !assigned_between(
                        function,
                        (&analysis.structs, &analysis.lists),
                        cfg.order(),
                        &reach,
                        target,
                        read,
                        local,
                    ) && reaches_certainly(unit, sources, function, target, read.0)
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
