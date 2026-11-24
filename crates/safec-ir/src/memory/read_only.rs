//! Writes that may reach storage the program may not modify: a static
//! defined `const`, a `const` local, a string literal (C17 6.7.3 p6, 6.4.5
//! p7). Asked at the two places a write is made: a store through a pointer,
//! and a call handed a pointer it may write through. See ADR-0120.

use crate::analysis::Conclusion;
use crate::ir::{Element, FuncId, Function, Operand, Place, Rvalue, Shape, Terminator, Ty, TyId};
use crate::library::Callee;

use super::allocations::Allocations;
use super::index::{LocalIdx, Storage};
use super::known::Known;
use super::store::{Landing, StoreKind, Written};
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
    let marked = known
        .read_only_levels_at(landing.pointer, landing.deep - 1)
        .here();
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
    match depth_of(place) {
        Some(depth) => names_read_only_at(place.local, depth, read_only, known),
        None => false,
    }
}

/// How many dereferences `place` reads below its local, where it is one this
/// check reads levels of.
fn depth_of(place: &Place) -> Option<usize> {
    match place.shape() {
        Shape::Local => Some(0),
        Shape::Deref(depth) => Some(depth),
        Shape::Indexed => None,
    }
}

/// Record a read-only write, **once per caret**: a store into a `const`
/// object that also leaves holding a literal was asked by
/// [`written_read_only`] and by [`stored_read_only`], and the same `SC0417`
/// in the same words was reported twice (#1177). Every door pushes here.
/// Mutation: push directly in `stored_read_only`;
/// `a_store_both_written_and_leaving_read_only_is_reported_once` fails.
///
/// **The same report, not only the same caret**: the same function, kind
/// and span, and the same local where both name one, so two arguments of
/// one call handed read-only storage stay two reports, and a caret in one
/// function never answers for another. A finding that names no local says
/// the same words as one that does at that caret, and gives way to it.
fn report_once(findings: &mut Vec<Finding>, finding: Finding) {
    let same_caret = |found: &Finding| {
        found.function == finding.function && found.kind == finding.kind && found.at == finding.at
    };
    match finding.local {
        // **Any report at the caret answers for one that names no local**,
        // whose words are the same.
        None if findings.iter().any(same_caret) => {}
        None => findings.push(finding),
        // **One that names a local is dropped only for the same local**, and
        // takes the place of one that named none, so the outcome does not
        // turn on which door ran first: two arguments stay two either way.
        Some(_)
            if findings
                .iter()
                .any(|found| same_caret(found) && found.local == finding.local) => {}
        Some(_) => {
            if let Some(unnamed) = findings
                .iter_mut()
                .find(|found| same_caret(found) && found.local.is_none())
            {
                *unnamed = finding;
            } else {
                findings.push(finding);
            }
        }
    }
}

/// [`names_read_only`] of the value read `depth` dereferences below `local`.
fn names_read_only_at(
    local: crate::ir::LocalId,
    depth: usize,
    read_only: &[LocalIdx],
    known: &Known,
) -> bool {
    if depth == 0 {
        let held = &known.points_to[local];
        return known.read_only_of(held).here()
            || held.locals().any(|local| read_only.contains(&local));
    }
    // **And the locals it may point at, read as the value is**: `set(*pp)`
    // with `*pp` holding a `const` local's address was asked for a static
    // alone. Found by review.
    known.read_only_levels_at(local, depth).here()
        || known
            .levels_below(local, depth)
            .1
            .iter()
            .any(|local| read_only.contains(local))
}

/// Whether what `ty`'s last pointer level points at is `void`: a reader
/// whose type says nothing of what lies below it.
fn ends_in_void(analysis: &Allocations<'_>, ty: TyId) -> bool {
    let mut ty = ty;
    while let Ty::Pointer(pointee) = analysis.unit.ty(ty) {
        ty = pointee;
    }
    matches!(analysis.unit.ty(ty), Ty::Void)
}

/// How many levels of pointer `ty` has: `int **` two.
fn pointer_levels(analysis: &Allocations<'_>, ty: TyId) -> usize {
    let mut levels = 0;
    let mut ty = ty;
    while let Ty::Pointer(pointee) = analysis.unit.ty(ty) {
        levels += 1;
        ty = pointee;
    }
    levels
}

/// Whether a level from `from` down of what `place` holds, leaving for a
/// reader of `receiver` whose `const` levels are `levels`, may name read-only
/// storage where the reader believes it may write: a level not to `const`.
/// `int *c = (int *)&k; set2(&c);` with `set2` taking `int **` hands `k` one
/// level below. See ADR-0121.
///
/// **Past the levels `receiver` has, where `beyond`**, while anything is
/// named there: a reader handed a `void *` or a `char *` may convert it
/// back and write at a level its type never said, and nothing says `const`
/// there. Asked only to the receiver's depth, `void *f(void)` returning a
/// block that holds `(int *)&k` was believed (#1134 review). A library
/// function this check reads by name is not, since what it does with what
/// it is handed is known, and a copy is asked as the store it is
/// ([`copied_read_only`]). Mutation: stop at the receiver's depth;
/// `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
/// `returned_as_void` goes silent.
fn below_read_only(
    analysis: &Allocations<'_>,
    place: &Place,
    (receiver, levels): (TyId, u32),
    (from, beyond): (usize, bool),
    read_only: &[LocalIdx],
    known: &Known,
) -> bool {
    let Some(depth) = depth_of(place) else {
        return false;
    };
    let typed = pointer_levels(analysis, receiver);
    // Only past a pointer: an integer's reader reads no level below it.
    // **Bounded by what is named, not by a declared type**: `void *` slots
    // chain a literal as deep as the program likes, and stopped at the
    // deepest pointer the function declares, `x[0] = y; y[0] = "ab";
    // take(x);` was believed (#1159). A chain of distinct storages is no
    // longer than there are storages, twice the locals, and the walk stops
    // where a level names nothing. See ADR-0123. Mutation: stop at
    // `analysis.deepest_pointer`; `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
    // `chained_through_void_slots` goes silent.
    let last = if beyond && typed > 0 {
        typed.max(2 * analysis.locals + 1)
    } else {
        typed
    };
    // **Or where the value carries a read-only level there**, which names
    // nothing: a `const char ***` parameter's memory holds no name a level
    // down, and stopped there, `take_any(pp)` was believed where main
    // reported it (#1165 review). Mutation: ask only what is named;
    // `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
    // `arrived_three_deep_handed_as_void` goes silent.
    let named_at = |level: usize| {
        level < typed
            || level == 0
            || known.read_only_levels_at(place.local, depth + level) != super::parts::ReadOnly::NONE
            || {
                let (sites, locals) = known.levels_below(place.local, depth + level - 1);
                !sites.is_empty() || !locals.is_empty()
            }
    };
    // A level past `typed` has no bit set, so it is asked as writable. **Of
    // what the value certainly reaches where the reader keeps `const` at its
    // last level**: through a pointer read out of a global, every escaped
    // block stood for the `char` a `const char *` reader reaches, `*out =
    // G;` was reported (#1145). **And of everything it may reach where the
    // reader does not**, a `void *` or a `char *`, which may convert back
    // and write: asked of what it certainly reaches alone, a call's result
    // handed as `void *` lost every block it may return, and a literal kept
    // behind one was believed (#1163 review). See ADR-0122 and ADR-0123.
    // Mutation: ask past `typed` as within it;
    // `a_pointer_read_out_of_a_global_stored_through_a_const_out_parameter_is_not_a_write`'s
    // `stored_beside_a_table_handed_on` is reported. Mutation: ask every
    // reader of what is certain alone;
    // `a_call_result_names_what_it_was_shown_apart_from_what_it_may_return`'s
    // `handed_as_void_beside_a_literal_kept` goes silent.
    // **Not a reader whose `const` is on `void`**: a `const void *` may be
    // converted to `char *const *` and written a level down with no cast,
    // so its `const` keeps nothing past its type, and leaving guesses out
    // there believed `use_c(get())` (#1178). Mutation: answer `false` from
    // `ends_in_void`; `a_call_result_names_what_it_was_shown_apart_from_what_it_may_return`'s
    // `handed_as_const_void_beside_a_literal_kept` goes silent.
    let keeps_const =
        typed > 0 && levels >> (typed - 1) & 1 != 0 && !ends_in_void(analysis, receiver);
    (from..last)
        .take_while(|&level| named_at(level))
        .any(|level| {
            levels
                .checked_shr(u32::try_from(level).unwrap_or(u32::MAX))
                .unwrap_or(0)
                & 1
                == 0
                && if level < typed || !keeps_const {
                    read_only_exactly_at(place.local, depth + level, read_only, known)
                } else {
                    read_only_certainly_at(place.local, depth + level, read_only, known)
                }
        })
}

/// [`read_only_exactly_at`] of what the value read `depth` down certainly
/// reaches, for a level past its reader's type. See ADR-0122.
fn read_only_certainly_at(
    local: crate::ir::LocalId,
    depth: usize,
    read_only: &[LocalIdx],
    known: &Known,
) -> bool {
    known.read_only_levels_named_at(local, depth).here()
        || known
            .levels_named_below(local, depth)
            .1
            .iter()
            .any(|local| read_only.contains(local))
}

/// Whether the value read `depth` dereferences below `local` may name
/// read-only storage, from that level alone: [`names_read_only_at`] reads
/// every level above it as one, which is what a load takes, and asked of a
/// level a reader's type keeps `const` above, `reads(&c)` with `reads`
/// taking `const int **`, it answered for the level the reader may not
/// write. Found while widening the walk for #1134's review.
fn read_only_exactly_at(
    local: crate::ir::LocalId,
    depth: usize,
    read_only: &[LocalIdx],
    known: &Known,
) -> bool {
    // Exact now that each level is read by itself (#1152).
    names_read_only_at(local, depth, read_only, known)
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
    analysis: &Allocations<'_>,
    function: &Function,
    element: &Element,
    known: &Known,
    func: FuncId,
) {
    let Element::Assign(operation) = element else {
        return;
    };
    if operation.place.local != function.return_place() || !operation.place.steps().is_empty() {
        return;
    }
    let returns_reads_only = function.returns_reads_only();
    let Rvalue::Use(operand) = &operation.value else {
        return;
    };
    let read_only = read_only_locals(function);
    let return_place = function.return_place();
    // **And every level below it its type believes writable**, a block
    // returned holding a pointer to `const` (#1134). Mutation: skip this;
    // `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
    // `returned_block` goes silent.
    let below = match operand {
        Operand::Copy(place) => below_read_only(
            analysis,
            place,
            (
                function.local(return_place),
                function.const_levels(return_place),
            ),
            (1, true),
            &read_only,
            known,
        ),
        // A function's code is not storage a return hands back to write.
        Operand::Constant(_) | Operand::Function(_) => false,
    };
    if below || (!returns_reads_only && names_read_only(operand, &read_only, known)) {
        report_once(
            findings,
            Finding {
                function: func,
                kind: Kind::WritesReadOnly,
                conclusion: Conclusion::Unknown,
                at: operation.origin.span(),
                local: None,
                freed: None,
                made: None,
                unproven: None,
            },
        );
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
        report_once(
            findings,
            Finding {
                function: func,
                kind: Kind::WritesReadOnly,
                conclusion: Conclusion::Unknown,
                at: store.at,
                local: Some(landing.pointer),
                freed: None,
                made: None,
                unproven: None,
            },
        );
    }
}

/// Report a call handed, for a parameter that is no pointer to `const`, a
/// pointer that may name read-only storage: the callee may write through it,
/// whether its body is here or not, and a library writes through its
/// destination. A parameter past the last, where nothing says the callee
/// only reads, is asked as one that may write. **Not `free` or `realloc`**,
/// which release what they are handed rather than write it: a literal
/// freed is the free rules' (`SC0404`), and said as a write it was said
/// wrongly. Mutation: ask them too; `a_string_literal_freed_is_reported`
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
    let beyond = match analysis.callee(callee) {
        Callee::Frees | Callee::Reallocates => return,
        Callee::Allocates
        | Callee::ReturnsFirst
        | Callee::Copies
        | Callee::Ends
        | Callee::Reads => false,
        Callee::Opaque => true,
    };
    // **A call through a pointer is read as one through `void unp();`**:
    // its pointee's type carries no `const` levels per parameter, so every
    // argument is asked at its own and none is believed only read. See
    // ADR-0125.
    let called = callee.direct().map(|id| analysis.unit.function(id));
    let read_only = read_only_locals(function);
    let parameters: Vec<_> = called
        .map(|called| called.parameters().collect())
        .unwrap_or_default();
    for (index, argument) in arguments.iter().enumerate() {
        let Operand::Copy(place) = argument else {
            continue;
        };
        // **And every level below it the callee's type believes writable**,
        // `set2(&c)` with `c` holding a pointer to `const` (#1134); past the
        // last parameter, the argument's own type at every level. Mutation:
        // skip this; `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
        // `handed_below` goes silent.
        // **The argument's own `const` levels where no parameter says**, a
        // call through `void unp();`: what it says of itself is what the
        // program wrote, and `unp(&p)` with `p` a `const char *` was asked
        // as writable one level down (#1134 review). Mutation: use `0`;
        // `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
        // `handed_unprototyped` is reported.
        let receiver = match (called, parameters.get(index)) {
            (Some(called), Some(&parameter)) => {
                Some((called.local(parameter), called.const_levels(parameter)))
            }
            (_, None) | (None, Some(_)) => analysis.unit.place_ty(function, place).map(|ty| {
                let depth = depth_of(place).unwrap_or(0);
                let own = function
                    .const_levels(place.local)
                    .checked_shr(u32::try_from(depth).unwrap_or(u32::MAX))
                    .unwrap_or(0);
                (ty, own)
            }),
        };
        let below = receiver.is_some_and(|receiver| {
            below_read_only(analysis, place, receiver, (1, beyond), &read_only, known)
        });
        if below
            || (!called.is_some_and(|called| called.reads_only_through(index))
                && names_read_only(argument, &read_only, known))
        {
            report_once(
                findings,
                Finding {
                    function: func,
                    kind: Kind::WritesReadOnly,
                    conclusion: Conclusion::Unknown,
                    at: origin.span(),
                    local: Some(place.local),
                    freed: None,
                    made: None,
                    unproven: None,
                },
            );
        }
    }
}

/// At which levels the reader of the storage a store or a copy lands in
/// keeps `const`: every recorded reader's levels where the landing reaches
/// one, a level kept only where all keep it, and `declared`, what the place
/// written through says, for a storage whose reader was not recorded.
/// **One answer for every leaving door**, so that a store and a copy into
/// the same storage cannot read different readers (#1147). See ADR-0123.
///
/// **Only where the landing is placed**: through a pointer read out of a
/// global, every exposed block stood for where it may land, a parameter's
/// memory among them, and that one reader answered for all, so `q = GW; *q
/// = "ab";` was believed beside an unrelated `const char **` parameter
/// (#1165 review). Mutation: ask the records of an unplaced landing too;
/// `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
/// `stored_through_a_global_beside_a_const_parameter` goes silent.
fn reader_levels(known: &Known, landing: Option<&Landing>, declared: u32) -> u32 {
    let Some(landing) = landing else {
        return declared;
    };
    if landing.through_unrecorded || known.points_to[landing.pointer].beyond().any_unknown() {
        return declared;
    }
    landing
        .containers
        .iter()
        .map(|&site| Storage::Heap(site))
        .chain(landing.targets.iter().map(|&local| Storage::Local(local)))
        .map(|place| known.receiver_levels[place].map_or(declared, |levels| levels.mask()))
        .reduce(|here, there| here & there)
        .unwrap_or(declared)
}

/// Report a store, into memory that leaves this function, of a pointer that
/// may name read-only storage at a level the memory's reader believes it may
/// write: `*out = "ab";` through an out-parameter, `G = "ab";` into a static,
/// `*b = (int *)&k;` into a block returned. The reader reads it as the
/// stored-into type, which says nothing of `const` (#1134). The return place
/// is [`returned_read_only`]'s. See ADR-0121. Mutation: skip this;
/// `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
/// `out_parameter` goes silent.
pub(super) fn stored_read_only(
    findings: &mut Vec<Finding>,
    analysis: &Allocations<'_>,
    function: &Function,
    element: &Element,
    known: &Known,
    func: FuncId,
) {
    let Element::Assign(operation) = element else {
        return;
    };
    if operation.place.local == function.return_place() {
        return;
    }
    let Some(store) = analysis.store_in(element, known) else {
        return;
    };
    // Into memory that leaves, as `departed_in_element` reads a departure.
    let leaves = match store.kind {
        StoreKind::Through { .. } | StoreKind::Direct => store.leaves,
        StoreKind::Copy => false,
    };
    if !leaves {
        return;
    }
    let (Rvalue::Use(Operand::Copy(value)), Some(depth), Some(receiver)) = (
        &operation.value,
        depth_of(&operation.place),
        analysis.unit.place_ty(function, &operation.place),
    ) else {
        return;
    };
    // **What the place written through holds**, a cast's operand among it:
    // `*(char **)CG = "ab"` with `CG` a `const char *[2]` lands where every
    // reader keeps `const`, and asked at the cast's type alone was reported
    // (#1165 review). A reader recorded for the memory takes levels away.
    let declared = function
        .held_levels(operation.place.local)
        .checked_shr(u32::try_from(depth).unwrap_or(u32::MAX))
        .unwrap_or(0);
    let levels = reader_levels(known, store.landing.as_ref(), declared);
    if below_read_only(
        analysis,
        value,
        (receiver, levels),
        (0, true),
        &read_only_locals(function),
        known,
    ) {
        report_once(
            findings,
            Finding {
                function: func,
                kind: Kind::WritesReadOnly,
                conclusion: Conclusion::Unknown,
                at: operation.origin.span(),
                local: None,
                freed: None,
                made: None,
                unproven: None,
            },
        );
    }
}

/// Report a library copy, into memory that leaves this function, of bytes
/// that may hold a pointer to read-only storage: `memcpy(out, &c, sizeof c)`
/// with `c` a literal is `*out = c;` spelled as a copy, which
/// [`stored_read_only`] asks and this, before #1134's review, did not. The
/// destination's type says nothing of what the bytes are, so every level
/// is asked as writable. See ADR-0121. Mutation: skip this;
/// `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
/// `copied_out` goes silent.
pub(super) fn copied_read_only(
    findings: &mut Vec<Finding>,
    analysis: &Allocations<'_>,
    function: &Function,
    terminator: &Terminator,
    known: &Known,
    func: FuncId,
) {
    let Some(store) = analysis.store_at(terminator, known) else {
        return;
    };
    let Written::Bytes(source) = store.from else {
        return;
    };
    let leaves = match store.kind {
        StoreKind::Copy => store.leaves,
        StoreKind::Through { .. } | StoreKind::Direct => false,
    };
    let Some(receiver) = analysis.unit.place_ty(function, source) else {
        return;
    };
    // **What the destination's reader keeps `const` at**, as a store into it
    // asks: read as `0`, `memcpy(out, &c, 8)` through a `const char **` was
    // `SC0417` where `*out = c;` was not. Where no reader was recorded the
    // bytes say nothing, so every level is asked. See ADR-0123. Mutation:
    // ignore a recorded reader; `copied_into_a_const_out_parameter` reports.
    // **Not what the destination place says**, which a cast to `const`
    // raises: read so, `memcpy((const char **)&G, &s, 8)` into a static was
    // believed (#1176). **This costs a false report** where the destination
    // is declared `const` and no reader was recorded, `memcpy(&CG, &s, 8)`
    // with `CG` a `const char *`, as main before #1165 had it: the IR does
    // not tell a `const` the program declared from one a cast raised, and a
    // copy believed wrongly is the worse failure. Shifted up one, since the source is asked from its
    // level `1`, which is what the destination's reader reads at its level
    // `0`. Mutation: fall back to the destination's `held_levels`;
    // `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
    // `copied_into_a_static_through_a_const_cast` goes silent.
    let levels = reader_levels(known, store.landing.as_ref(), 0)
        .checked_shl(1)
        .unwrap_or(0);
    if leaves
        && below_read_only(
            analysis,
            source,
            (receiver, levels),
            (1, true),
            &read_only_locals(function),
            known,
        )
    {
        report_once(
            findings,
            Finding {
                function: func,
                kind: Kind::WritesReadOnly,
                conclusion: Conclusion::Unknown,
                at: store.at,
                local: None,
                freed: None,
                made: None,
                unproven: None,
            },
        );
    }
}
