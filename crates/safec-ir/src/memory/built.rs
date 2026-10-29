//! Building a value out of operands: what the result of arithmetic holds and
//! where in its allocation it points, which locals a write this check cannot
//! pin down may have replaced, and the sites out of everything a place reached.

use crate::ir::{BinOp, Function, LocalId, Operand, Place, TranslationUnit, Ty};

use super::derefs;
use super::known::{ArrayWrite, Known};
use super::parts::{Held, Offset, Reached};
use super::transfer::innermost_element;

/// What the operands of a binary operation build.
///
/// **Where one operand is a pointer, it is the only one that contributed.**
/// C17 6.5.6 p8 keeps the result of pointer arithmetic inside the object the
/// pointer operand points into, so the integer beside it cannot decide what the
/// result reaches whatever that integer happens to hold. That is a statement
/// about the program rather than a belief about a type, which is what lets a
/// may-set be narrowed here at all. See ADR-0030.
///
/// **The proof survives only where nothing else contributed.** One contributing
/// operand and a constant is `p + 1`: the result is that operand offset, and
/// the clause above keeps it inside the same object, so the set the proof is
/// about is the set the result names. Two of them is an expression whose value
/// may be either, and [`Held::accumulated`] drops the proof for the reason
/// written there.
///
/// **Where in its sites the result points is answered last**, by
/// [`offset_of`], and it is the one thing here that turns on the operator.
///
/// One function with two callers, because the same question is asked where a
/// value is assigned and where one is written through a pointer, and one rule
/// in two places drifts apart inside the change that touches one of them.
///
/// Each closure is a question about the function being checked, which this
/// module has no handle on; bundling them would be a type with one caller.
#[allow(clippy::too_many_arguments)]
pub(super) fn built_from(
    op: BinOp,
    operands: [&Operand; 2],
    value: &Known,
    is_pointer: impl Fn(LocalId) -> bool,
    may_be_pointer: impl Fn(&Place) -> bool,
    reads_a_byte: impl Fn(&Place) -> bool,
    reads_caller_memory: impl Fn(LocalId, usize) -> bool,
    is_array: impl Fn(usize) -> bool,
) -> Held {
    let followed: Vec<LocalId> = operands
        .iter()
        .filter_map(|operand| match operand {
            // A constant is not a value this check follows. A read through a
            // projection is a place rather than a local, and is followed below
            // as the load it is.
            Operand::Copy(source) if source.projection.is_empty() => Some(source.local),
            _ => None,
        })
        .collect();

    // **An operation with no pointer operand keeps every one of them**, and
    // this is the half C says nothing about: two integers added together are
    // not pointer arithmetic, so no clause says the result cannot reach what
    // its operands reach. Narrowing there would be narrowing on nobody's
    // authority, and what an emptied set costs is silence: a dereference of a
    // local that reaches no site is reported by nothing at all.
    //
    // **`i + j` reaches this constantly**, and what is rare is one of those
    // integers holding an allocation. It takes a program C forbids, which this
    // compiler does not yet refuse: `int i = p;` is a constraint violation
    // under C17 6.5.16.1 p1 and #154 is the check that is missing. See
    // ADR-0030, which measures what this branch is worth on such a program.
    // **A load that may be a pointer is the pointer operand**, so the
    // integers beside it contribute nothing, as below for a local pointer:
    // `*tab + i` stays inside what `*tab` points into, whatever `i` holds,
    // and an integer can hold sites: one returned by a call this check cannot
    // read holds what the call was handed. `n = h(r); free(r); q = *t2 + n;`
    // doubted `*q` about `r` while it kept them. See ADR-0030.
    let loads: Vec<&Place> = operands
        .iter()
        .filter_map(|operand| match operand {
            Operand::Copy(source) if derefs(source) > 0 && may_be_pointer(source) => Some(source),
            _ => None,
        })
        .collect();
    let followed: Vec<usize> = if !loads.is_empty() {
        followed
            .iter()
            .filter(|&&local| is_pointer(local))
            .map(|local| local.index())
            .collect()
    } else if followed.iter().any(|&local| is_pointer(local)) {
        followed
            .iter()
            .filter(|&&local| is_pointer(local))
            .map(|local| local.index())
            .collect()
    } else {
        followed.iter().map(|local| local.index()).collect()
    };

    // **A byte read out of memory contributes what was stored where it was
    // read**, as a load does, but is no pointer operand: two bytes added are
    // not pointer arithmetic, so it narrows nothing beside it. `*d = *s + 0`
    // is the byte `*d = *s` copies. See ADR-0046.
    let bytes: Vec<&Place> = operands
        .iter()
        .filter_map(|operand| match operand {
            Operand::Copy(source) if derefs(source) > 0 && reads_a_byte(source) => Some(source),
            _ => None,
        })
        .collect();

    let mut reached = Held::none(value.points_to.len());
    for &source in &followed {
        reached.accumulated(&value.points_to[source]);
    }

    if let [one] = followed[..] {
        reached.freed = value.points_to[one].freed;
    }

    reached.offset = offset_of(op, operands, &followed, value);
    // **And it contributes what it holds**, what a load out of the same place
    // assigned to a local is given, at an offset nobody said since the
    // distance is not carried (ADR-0036). `loaded` below keeps every reader
    // from proving anything with it, so `t3[i][i]` is asked as `t3[0][0]` is.
    // See ADR-0045.
    for load in loads.iter().chain(&bytes) {
        let (sites, locals) = value.levels_below(load.local, derefs(load));
        for site in sites {
            reached.hold(site, Offset::Unknown);
        }
        // **And the locals whose address it may be**, as `read_through`
        // carries them, so `(*ppo)[k]` keeps the edge `*ppo` does where the
        // arithmetic leaves it, [`Held::moved_by_arithmetic`]'s to decide.
        // Found by review. See ADR-0019 and ADR-0045.
        for target in locals {
            reached.writes_to.set(target);
            // And it may be a pointer that reached an array only by #639's
            // rule. See `Known::stored_in_part`.
            if value.stored_in_part[target] {
                reached.array_edge = true;
            }
        }
        // Or read out of a local, not an array, that holds one, as
        // `read_through` says.
        if value
            .levels_below(load.local, derefs(load) - 1)
            .1
            .into_iter()
            .any(|local| !is_array(local) && value.points_to[local].array_edge)
        {
            reached.array_edge = true;
        }
        // And lost where the load would be. See ADR-0045.
        if value.stale_below(load.local, derefs(load)) {
            reached.lost = true;
            reached.stale_read = true;
        }
        // And read out of a slot a call could not replace, as `read_through`
        // says. See ADR-0047.
        if value.marked_below(load.local, derefs(load), &value.unreplaced) {
            reached.unreplaced_read = true;
        }
        // And read out of memory a lost value may be in, as `read_through`
        // says. See `Held::lost_read`.
        if value.marked_below(load.local, derefs(load), &value.holds_lost) {
            reached.lost_read = true;
        }
        if value.lost_through(load.local, derefs(load)) {
            reached.lost = true;
            reached.stale_read |= value.stale_through(load.local, derefs(load));
        }
        // And what the caller stored, as `read_through` says. See ADR-0040.
        if reads_caller_memory(load.local, derefs(load)) {
            reached.from_caller = true;
        }
    }

    // **A read through a projection is not followed, and is still a load.**
    // `*tab + 1` is the pointer `*tab` moved, and the lowering hands it here
    // as one operand rather than through a temporary, so the bit an operand
    // carries in [`Held`] never arrives for it: `q = *tab + 1; show(q);`
    // exposed nothing, found by mutating [`Held::accumulated`]. See ADR-0040.
    let read_here = operands.iter().any(|operand| match operand {
        Operand::Copy(source) => {
            !source.projection.is_empty() && (may_be_pointer(source) || reads_a_byte(source))
        }
        Operand::Constant(_) => false,
    });
    reached.loaded |= read_here;
    // As of the read, as `read_through` records it, and only for a read made
    // here: an operand read earlier carries its own, and `t + 1` after
    // `*b = m;` is still `t`. See `Held::named_at_read`.
    if read_here {
        for named in value.locals_named_by_unknown_code() {
            reached.named_at_read.set(named);
        }
    }

    reached
}

/// Where the result of a binary operation points in the sites it reaches.
///
/// `Offset::NonZero` where all of these hold, and `Offset::Unknown` otherwise:
///
/// * the operator is `+`, or `-` with the followed operand on the left. C17
///   6.5.6 p8 keeps `P + N`, `N + P` and `P - N` inside the object `P` points
///   into, since it is about an integer added to or subtracted from a pointer,
///   and p3 allows a pointer only on the left of a `-`.
/// * exactly one operand was followed. Two is `p - q`, which 6.5.6 p9 makes a
///   `ptrdiff_t` rather than a pointer, or a shape no C program builds.
/// * the other operand is a constant, and **the constant is read rather than
///   assumed non-zero**. ADR-0021 folds `p + 0` away where the IR is built, and
///   `docs/c-family.md` records that nothing enforces it, so an IR from a
///   frontend that skipped the fold would hand this a literal zero. Answering
///   `Unknown` there is a false report the reader can see, where assuming
///   would be saying safe wrongly, on the strength of an invariant nobody
///   enforces.
/// * the followed operand is itself at the start. `q = p + 1; r = q - 1;` has
///   `r` back at the start, and nothing here carries a distance to know it.
///
/// Every other operator answers `Unknown`, a `*` included: C17 6.5.5 p2 gives
/// it arithmetic operands only, and a hand-built IR can hold one anyway. See
/// ADR-0036.
fn offset_of(op: BinOp, operands: [&Operand; 2], followed: &[usize], value: &Known) -> Offset {
    let [one] = followed[..] else {
        return Offset::Unknown;
    };

    let moved = match (op, operands) {
        (BinOp::Add | BinOp::Sub, [Operand::Copy(_), Operand::Constant(by)])
        | (BinOp::Add, [Operand::Constant(by), Operand::Copy(_)]) => *by != 0,
        _ => false,
    };

    if moved && value.points_to[one].offset == Offset::Zero {
        Offset::NonZero
    } else {
        Offset::Unknown
    }
}

/// Distrust every escaped local a write through this place may have reached.
///
/// ADR-0029's fact with the writer inside the function: whoever holds an
/// escaped address may have been handed it through a projection this check
/// does not follow, so a write it cannot pin down may replace what an escaped
/// local holds, and writing `SiteState::Freed` on that local's sites later is
/// a proof about an allocation this write may have swapped out. See ADR-0031.
///
/// **Only a local declared with the type this write writes.** C17 6.5 p7 gives
/// an object an effective type and lets an lvalue of another type access it
/// only where that type is a character type, so a write of an `int` cannot
/// replace a pointer and the rule costs what it should rather than every
/// escaped local at every `*p = 1`. `docs/c-family.md` carries what that asks
/// of a frontend, and getting it wrong keeps a proof rather than losing one,
/// which is a false positive and not a silence.
///
/// **A character type reaches everything, which is the exception that clause
/// carries**, and no cast is needed to reach it: C17 6.3.2.3 p1 and 6.5.16.1
/// p1 make the `void *` round trip implicit both ways, so `void *v = &p;
/// char *c = v;` is a conforming program this frontend accepts, and copying
/// one pointer's object representation through `c` is defined. Review found
/// that one, and compiled and ran the C under a sanitiser to show the program
/// has no use after free in it.
///
/// [`TranslationUnit::place_ty`] answers `None` for a `Deref` of something
/// that is not a pointer, which the lowering does not build and a hand-built
/// unit can. Every local then, because a write whose type this cannot name is
/// a write it cannot narrow.
///
/// **What it may have written is a second, narrower answer**, about
/// `Known::unwritten` alone: a local of that type which
/// `Known::locals_a_write_may_land_in` says the place may land in. A local
/// the write cannot reach keeps its mark, so a read of it nothing wrote is
/// still `SC0411`. See ADR-0057.
///
/// A free function rather than a method, because it is the whole of what one
/// call site does and reads nothing of [`Allocations`](super::transfer::Allocations) but the unit.
pub(super) fn replaced_by(
    unit: &TranslationUnit,
    function: &Function,
    place: &Place,
    value: &mut Known,
) {
    let written_ty = unit.place_ty(function, place);
    let everything = matches!(written_ty.map(|ty| unit.ty(ty)), None | Some(Ty::Char));
    // A row of bytes per local, against a value that is already square in
    // them, because the predicate is asked per index and `LocalId` cannot be
    // built from one.
    //
    // **An array holds its elements**, so `*p = &x;` with `p` an `int **`
    // may land in an `int *m[1]`: compared against the local's own type, the
    // array was believed unwritten after it, and a read of `m[0]` was reported
    // as indeterminate (#640).
    let may_hold: Vec<bool> = function
        .locals()
        .map(|local| {
            everything || written_ty == Some(innermost_element(unit, function.local(local)))
        })
        .collect();

    // Written: a local of the type stored that the place may land in.
    let mut lands = vec![false; may_hold.len()];
    for local in value.locals_a_write_may_land_in(place.local, derefs(place)) {
        lands[local] = true;
    }
    // **Through a pointer moved off an array, an element of it**, as the
    // placed store answers: still a write `replaced` makes of every local it
    // may land in, then partly written where it was unwritten before. See
    // `Known::partly_written`.
    let writes = write_effects(
        unit,
        function,
        value,
        place.local,
        derefs(place),
        (0..lands.len()).filter(|&local| lands[local] && may_hold[local]),
        false,
    );
    let pending = value.array_writes_before(writes);
    value.replaced(
        |local| may_hold[local],
        |local| may_hold[local] && lands[local],
    );
    value.array_writes_after(pending);
}

/// What a write through `local`, `dereferences` deep, is to each array among
/// `lands` that it does not write whole: one element, where every pointer it
/// goes through that names the array names the same known element, and an
/// element that may be any of them, where the edge to the array exists only
/// by #639's rule or the pointer was read out of memory. An array missing
/// from the answer is written whole, as every array was before #667.
///
/// **Whole where `main` kept the edge, in part where only the array rule
/// did**, by the user's decision on #667: every other line measured lost a
/// proof `main` gives or doubted every loop that fills an array. A `call`,
/// a library copy or a call this check cannot read, may write past the
/// element it is handed, so what it is handed at a known element other
/// than the start is in part too. One answer for the placed store, a store
/// this check cannot place, a library copy and a call. See
/// `Known::unwritten_elements` and ADR-0057.
pub(super) fn write_effects(
    unit: &TranslationUnit,
    function: &Function,
    value: &Known,
    local: LocalId,
    dereferences: usize,
    lands: impl IntoIterator<Item = usize>,
    call: bool,
) -> Vec<(usize, ArrayWrite)> {
    let lands: Vec<usize> = lands.into_iter().collect();
    let (mut through, from_memory): (Vec<usize>, bool) = match dereferences {
        0 => return Vec::new(),
        1 => (vec![local.index()], false),
        _ => {
            let (sites, locals) = value.levels_below(local, dereferences - 2);
            (locals.into_iter().collect(), !sites.is_empty())
        }
    };
    // **A call writes through every pointer it can reach**, not only the
    // one it is handed: `fill(&e, &x)` with `e = m + 1` writes `m` through
    // `e`, a level below the argument. Every local it may write is one it
    // may read a pointer out of, so those are what is asked.
    if call {
        through.extend(lands.iter().copied());
    }
    lands
        .into_iter()
        .filter(|&each| is_array(unit, function, each))
        .filter_map(|each| {
            let naming: Vec<&super::parts::Held> = through
                .iter()
                .map(|&pointer| &value.points_to[pointer])
                .filter(|held| held.writes_to.get(each))
                .collect();
            // **And a call may read a moved pointer out of memory** a store
            // put there, which no local it reaches by name holds: `*h = m +
            // 1; fill(h);` with `h` an allocation. See
            // `Known::stored_in_part`. Found by review of #667.
            let array_edge = from_memory
                || naming.iter().any(|held| held.array_edge)
                || (call && value.stored_in_part[each]);
            let element = match naming.split_first() {
                Some((first, rest)) if rest.iter().all(|held| held.element == first.element) => {
                    first.element
                }
                _ => None,
            };
            let kept = value.unwritten_elements[each].len() > 0;
            match element {
                Some(0) if call && !array_edge => None,
                Some(_) if call => Some(ArrayWrite::Maybe),
                // Not where a pointer it goes through was read out of memory,
                // whose element nobody recorded: `**q` with `q` either `&e`
                // or an allocation holding `m` writes `m[1]` or `m[0]`.
                // Found by review of #667.
                Some(element) if kept && !from_memory => Some(ArrayWrite::Element(element)),
                _ if array_edge => Some(ArrayWrite::Maybe),
                _ => None,
            }
            .map(|write| (each, write))
        })
        .collect()
}

/// Whether the local at `index` is an array. One answer for the edge a
/// pointer moved off it keeps and for the write through such a pointer
/// being of an element. See `Held::moved_by_arithmetic` and
/// `Known::partly_written`.
pub(super) fn is_array(unit: &TranslationUnit, function: &Function, index: usize) -> bool {
    function
        .locals()
        .nth(index)
        .is_some_and(|local| matches!(unit.ty(function.local(local)), Ty::Array { .. }))
}

/// The sites out of everything a place or an argument reached.
///
/// **One fold with three callers, because the three have to agree.** It is what
/// a free writes `Freed` on, what a read carries forwards as the allocations it
/// may have touched, and what the two are compared against when the order
/// between them is open. A free that wrote on a set this did not answer, or a
/// read that carried one, would be a report about an allocation the other half
/// never considered. One rule written in two places drifts apart.
///
/// Neither of the other two variants names a site: one is a fact about a set,
/// which ADR-0020 records on the local rather than on its members, and the
/// other is this check having lost the pointer.
pub(super) fn named(reached: &[Reached]) -> impl Iterator<Item = usize> + '_ {
    reached.iter().filter_map(|reached| match reached {
        Reached::Site(site) => Some(*site),
        Reached::SetFreed(_) | Reached::Lost(_) | Reached::Partial => None,
    })
}
