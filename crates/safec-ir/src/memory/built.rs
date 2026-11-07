//! Building a value out of operands: what the result of arithmetic holds and
//! where in its allocation it points, which locals a write this check cannot
//! pin down may have replaced, and the sites out of everything a place reached.

use crate::ir::{BinOp, Function, LocalId, Operand, Place, TranslationUnit, Ty};

use super::allocations::innermost_element;
use super::index::{Idx, LocalIdx, Site, Table};
use super::known::{ArrayWrite, Known};
use super::parts::{Held, Init, Inits, Offset, Reached};
use crate::reads::derefs;

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
    is_array: impl Fn(LocalIdx) -> bool,
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
    let followed: Vec<LocalIdx> = if !loads.is_empty() {
        followed
            .iter()
            .filter(|&&local| is_pointer(local))
            .map(|&local| LocalIdx::of(local))
            .collect()
    } else if followed.iter().any(|&local| is_pointer(local)) {
        followed
            .iter()
            .filter(|&&local| is_pointer(local))
            .map(|&local| LocalIdx::of(local))
            .collect()
    } else {
        followed.iter().map(|&local| LocalIdx::of(local)).collect()
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
        // What memory holds there and the marks a load out of it carries,
        // as `Allocations::read_through` takes them for a load assigned to a
        // local. See `Known::load_into`.
        value.load_into(
            &mut reached,
            load.local,
            derefs(load),
            &is_array,
            reads_caller_memory(load.local, derefs(load)),
        );
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
fn offset_of(op: BinOp, operands: [&Operand; 2], followed: &[LocalIdx], value: &Known) -> Offset {
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
/// `Known::init` alone: a local of that type which
/// `Known::locals_a_write_may_land_in` says the place may land in. A local
/// the write cannot reach keeps its mark, so a read of it nothing wrote is
/// still `SC0411`. See ADR-0057.
///
/// A free function rather than a method, because it is the whole of what one
/// call site does and reads nothing of [`Allocations`](super::allocations::Allocations) but the unit.
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
    let may_hold: Table<LocalIdx, bool> = Table::from_vec(
        function
            .locals()
            .map(|local| {
                everything || written_ty == Some(innermost_element(unit, function.local(local)))
            })
            .collect(),
    );

    // Written: a local of the type stored that the place may land in.
    let mut lands: Table<LocalIdx, bool> = Table::filled(false, may_hold.len());
    for local in value.locals_a_write_may_land_in(place.local, derefs(place)) {
        lands[local] = true;
    }
    // **Through a pointer moved off an array, an element of it**, as the
    // placed store answers: still a write `replaced` makes of every local it
    // may land in, then partly written where it was unwritten before. See
    // `Known::init`.
    let writes = write_effects(
        unit,
        function,
        value,
        place.local,
        derefs(place),
        lands
            .indices()
            .filter(|&local| lands[local] && may_hold[local]),
        false,
        written_ty.and_then(|ty| unit.size_of(ty)),
    );
    // **A store this check reads that may land in more than one place may
    // have missed each local it may land in**, which may still be unwritten,
    // as a site the landing names beside another may (#721): with `g`
    // either `&m` or `&n`, or either `m + 1` or an allocation, `*g = &x;`
    // let a read of `m` or `m[1]` nothing wrote go silent, since
    // `Known::replaced` takes what it may write as written. That is a call
    // this check cannot read's belief (ADR-0057), and it is kept where the
    // store may land in one local alone, which a pointer read out of memory
    // is believed to reach as #726 believes a block's. Found by review.
    let landing: Vec<LocalIdx> = lands
        .indices()
        .filter(|&local| lands[local] && may_hold[local])
        .collect();
    let one_place =
        landing.len() == 1 && !(derefs(place) == 1 && value.points_to[place.local].holds_a_site());
    let before: Vec<(LocalIdx, Init, Inits)> = if one_place {
        Vec::new()
    } else {
        landing
            .iter()
            .map(|&local| (local, value.init[local], value.element_init[local].clone()))
            .collect()
    };
    let pending = value.array_writes_before(writes);
    value.replaced(
        |local| may_hold[local],
        |local| may_hold[local] && lands[local],
    );
    value.array_writes_after(pending);
    for (local, mut init, mut elements) in before {
        init.may_write();
        elements.may_write_all();
        value.init[local] = init;
        value.element_init[local] = elements;
    }
}

/// What a write through `local`, `dereferences` deep, is to each local among
/// `lands` that it does not write whole: one element, where every pointer it
/// goes through that names the array names the same known element; an
/// element that may be any of them, where the edge to the array exists only
/// by #639's rule or the pointer was read out of memory; the elements a copy
/// covers whole; and a write of part of a local kept whole, which leaves it
/// possibly unwritten (#826). A local missing from the answer is written
/// whole, as every array was before #667.
///
/// **`width` is the bytes written from where the write lands**, a store's
/// type or a library copy's constant length, and `None` where it is not
/// known: a call this check cannot read, or a copy of a length it cannot
/// read, which writes whole. Eight arguments, each one a door's own fact;
/// bundled, they would be a type with one caller.
///
/// **Whole where `main` kept the edge, in part where only the array rule
/// did**, by the user's decision on #667: every other line measured lost a
/// proof `main` gives or doubted every loop that fills an array. A `call`,
/// a library copy or a call this check cannot read, may write past the
/// element it is handed, so what it is handed at a known element other
/// than the start is in part too. One answer for the placed store, a store
/// this check cannot place, a library copy and a call. See
/// `Known::element_init` and ADR-0057.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_effects(
    unit: &TranslationUnit,
    function: &Function,
    value: &Known,
    local: LocalId,
    dereferences: usize,
    lands: impl IntoIterator<Item = LocalIdx>,
    call: bool,
    width: Option<u64>,
) -> Vec<(LocalIdx, ArrayWrite)> {
    let lands: Vec<LocalIdx> = lands.into_iter().collect();
    let (mut through, from_memory): (Vec<LocalIdx>, bool) = match dereferences {
        0 => return Vec::new(),
        1 => (vec![LocalIdx::of(local)], false),
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
    let one_landing = lands.len() == 1;
    let target = unit.target();
    let pointer_width = u64::from(target.pointer_bits() / target.char().bits());
    lands
        .into_iter()
        // **And a local that is no array, for a write of a width this check
        // reads**, which may write part of it: `memcpy(&p, &x, 4)`, or a
        // store through an `int *` made from `&p` (#826). Found by review.
        .filter(|&each| is_array(unit, function, each) || width.is_some())
        .filter_map(|each| {
            let naming: Vec<&super::parts::Held> = through
                .iter()
                .map(|&pointer| &value.points_to[pointer])
                .filter(|held| held.names_local(each))
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
            let kept = value.element_init[each].len() > 0;
            // **A write of part of a local kept whole leaves it possibly
            // unwritten** (#826): where it lands is known, an element of an
            // array or the start of a local that is no array, and it is
            // narrower than the local. Where it lands is not known, `b[i] =
            // 0;` in a loop that fills it, it is written whole as before. A
            // pointer local that is no array is written in part by a copy
            // that ends part way through it. **Not an array of pointers kept
            // whole**: a store writes a pointer whole, and a read of such an
            // array is asked only at its start, so a copy ending part way
            // through `big[1]` would doubt `big[0]`, written whole, and leave
            // `big[1]` silent, measured. See ADR-0073 rule 2.
            let array = is_array(unit, function, each);
            let ty = function
                .locals()
                .nth(each.index())
                .map(|id| function.local(id));
            let size = ty.and_then(|ty| unit.size_of(ty));
            let holds_pointers =
                ty.is_some_and(|ty| matches!(unit.ty(innermost_element(unit, ty)), Ty::Pointer(_)));
            let placed = if array {
                element.is_some()
            } else {
                !naming.is_empty() && naming.iter().all(|held| !held.inner_address)
            };
            // `placed` is where the write lands being known: an element of an
            // array, or the start of a local that is no array, which is what
            // an edge names. Two locals need no test of their own here: an
            // array kept per element holds pointers, so `in_part` is false for
            // it, and an array a pointer reaches by #639's rule is doubted by
            // the `array_edge` arm of the match below either way.
            if placed {
                if let (Some(width), Some(size)) = (width, size) {
                    // Narrower than the local, a copy of nothing included: a
                    // test of where it ends in a pointer let `memcpy(&p, &x,
                    // 0)` write `p` whole. Found by review.
                    let in_part = !(holds_pointers && array) && width < size;
                    if in_part {
                        return Some((each, ArrayWrite::Maybe));
                    }
                }
            }
            match element {
                // **A copy into an array kept per element writes the
                // elements it covers whole**, from its start (#826): `int
                // *m[2]; memcpy(m, &x, 12);` writes `m[0]` and leaves `m[1]`.
                //
                // **Only where it lands in this one array**: a copy that may
                // land in either of two writes neither for certain, as a
                // store that may land in two does not (`before` in
                // `replaced_by`), so each may have been missed. Found by
                // review.
                Some(0) if call && !array_edge => match width.map(|width| width / pointer_width) {
                    Some(_) if kept && !one_landing => Some(ArrayWrite::Maybe),
                    Some(covered) if kept && covered < value.element_init[each].len() as u64 => {
                        Some(ArrayWrite::Covered(covered))
                    }
                    _ => None,
                },
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
/// `Known::init`.
pub(super) fn is_array(unit: &TranslationUnit, function: &Function, index: LocalIdx) -> bool {
    function
        .locals()
        .nth(index.index())
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
pub(super) fn named(reached: &[Reached]) -> impl Iterator<Item = Site> + '_ {
    reached.iter().filter_map(|reached| match reached {
        Reached::Site(site) => Some(*site),
        Reached::SetFreed(_) | Reached::Lost(_) | Reached::Partial => None,
    })
}
