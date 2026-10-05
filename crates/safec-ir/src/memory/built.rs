//! Building a value out of operands: what the result of arithmetic holds and
//! where in its allocation it points, which locals a write this check cannot
//! pin down may have replaced, and the sites out of everything a place reached.

use crate::ir::{BinOp, Function, LocalId, Operand, Place, TranslationUnit, Ty};

use super::derefs;
use super::known::Known;
use super::parts::{Held, Offset, Reached};

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
pub(super) fn built_from(
    op: BinOp,
    operands: [&Operand; 2],
    value: &Known,
    is_pointer: impl Fn(LocalId) -> bool,
    may_be_pointer: impl Fn(&Place) -> bool,
    reads_a_byte: impl Fn(&Place) -> bool,
    reads_caller_memory: impl Fn(LocalId, usize) -> bool,
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
            reached.writes_to[target] = true;
        }
        // And lost where the load would be. See ADR-0045.
        if value.stale_below(load.local, derefs(load)) {
            reached.lost = true;
            reached.stale_read = true;
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
    reached.loaded |= operands.iter().any(|operand| match operand {
        Operand::Copy(source) => {
            !source.projection.is_empty() && (may_be_pointer(source) || reads_a_byte(source))
        }
        Operand::Constant(_) => false,
    });

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
    let may_hold: Vec<bool> = function
        .locals()
        .map(|local| everything || written_ty == Some(function.local(local)))
        .collect();

    value.replaced(|local| may_hold[local]);
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
        Reached::SetFreed(_) | Reached::Lost | Reached::Partial => None,
    })
}
