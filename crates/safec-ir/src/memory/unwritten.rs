//! What the memory check asks about bytes nothing wrote, after the walk: a
//! read of them, a copy of them, a block left with them where it leaves the
//! function, and part of a pointer written over one. **A file of its own**
//! (the memory model rebuilt on #924, stage 2): one of the four families of
//! question `report.rs` held, each reading its own facts, so a change to
//! one was read beside the other three. The shared fold, `say` and
//! `verdict`, stays in `report.rs`.

use super::report::{made_where_all_agree, say};

use crate::analysis::Conclusion;
use crate::ir::{
    Element, FuncId, Function, LocalId, Operand, Operation, Place, Rvalue, Terminator, Ty, TyId,
};
use crate::library::Callee;
use crate::reads::{derefs, read_out_of_memory_after, read_out_of_memory_in_terminator_after};
use crate::source::{SourceMap, Span};

use super::allocations::{Allocations, innermost_element, stored_pointer};
use super::assigned;
use super::bits::Bits;
use super::fit::{pointee, pointee_width};
use super::index::{Loc, LocalIdx, Site};
use super::known::Known;
use super::parts::{Init, SiteState};
use super::store::{StoreKind, Written};
use super::{Departure, Finding, Kind, PartAsked, Unproven};

/// One place the report asks whether memory a value names may be unwritten.
///
/// **One question, asked by both report walks through one list of doors**:
/// the every-turn walk and a loop's first turn each call
/// [`asked_unwritten_in_element`] and [`asked_unwritten_in_terminator`] and
/// nothing else for it, so a door is asked by both or by neither. Two doors
/// were once added to the every-turn walk alone, and a copy out or a `return`
/// in a loop shown to run built silent where the same body outside the loop
/// was `SC0411` (#838). A door added here is `error[E0004]` in both
/// dispatchers until it says what it asks.
#[derive(Clone, Copy)]
pub(super) enum UnwrittenDoor {
    /// A read of a pointer out of memory nothing wrote, [`loaded_indeterminate`].
    Load,
    /// A read by name of a pointer local nothing wrote, [`unwritten_read`].
    ByName,
    /// A library copy of bytes nothing wrote into memory that leaves,
    /// [`copied_unwritten`]. See ADR-0071 rule 5.
    CopyOut,
    /// A `return` of a block nothing wrote, [`report_returned_unwritten`]. See
    /// ADR-0073 rule 3.
    Return,
    /// A block a store let leave, still unwritten where the function returns,
    /// [`report_left_unwritten`]. See ADR-0076.
    LeftAtExit,
}

impl UnwrittenDoor {
    /// Every door, in the order a caret's findings are pushed.
    const EVERY: [UnwrittenDoor; 5] = [
        UnwrittenDoor::Load,
        UnwrittenDoor::ByName,
        UnwrittenDoor::CopyOut,
        UnwrittenDoor::Return,
        UnwrittenDoor::LeftAtExit,
    ];
}

/// What every door of [`UnwrittenDoor`] is asked with, at one point of one
/// walk.
pub(super) struct UnwrittenAsk<'a, 'u> {
    pub(super) sources: &'a SourceMap,
    pub(super) analysis: &'a Allocations<'u>,
    pub(super) function: &'a Function,
    /// `assigned::address_taken`, which [`unwritten_read`] reads.
    pub(super) taken: &'a Bits<LocalIdx>,
    /// A loop's first turn at a point that turn may not reach, which says no
    /// more than a doubt (#670).
    pub(super) doubt_only: bool,
}

/// Every door of [`UnwrittenDoor`] asked of one element. `stored` is the
/// walk's record of what the block last stored, which a load reads past.
pub(super) fn asked_unwritten_in_element(
    ask: &UnwrittenAsk<'_, '_>,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    stored: &mut Option<Place>,
    element: &Element,
    known: &Known,
) {
    for door in UnwrittenDoor::EVERY {
        match door {
            UnwrittenDoor::Load => loaded_indeterminate(
                ask.sources,
                findings,
                said,
                ask.analysis,
                ask.function,
                read_out_of_memory_after(stored, element),
                known,
                ask.doubt_only,
            ),
            UnwrittenDoor::ByName => unwritten_read(
                ask.sources,
                findings,
                said,
                ask.analysis,
                assigned::element_reads(element),
                ask.taken,
                known,
                ask.doubt_only,
            ),
            // A copy is a call, and leaving is a `return`, which end a block.
            UnwrittenDoor::CopyOut | UnwrittenDoor::LeftAtExit => {}
            // **Whether the block it returns was written**: `int **b =
            // malloc(8); return b;` hands the caller pointers nobody wrote,
            // where reading `*b` here is `SC0411` (#823).
            UnwrittenDoor::Return => {
                if let Element::Assign(operation) = element {
                    if operation.place == Place::local(ask.function.return_place()) {
                        report_returned_unwritten(ask, findings, operation, known);
                    }
                }
            }
        }
    }
}

/// Every door of [`UnwrittenDoor`] asked of one terminator. `stored` is what
/// [`asked_unwritten_in_element`] left at the block's end.
pub(super) fn asked_unwritten_in_terminator(
    ask: &UnwrittenAsk<'_, '_>,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    stored: &Option<Place>,
    terminator: &Terminator,
    known: &Known,
) {
    for door in UnwrittenDoor::EVERY {
        match door {
            UnwrittenDoor::Load => loaded_indeterminate(
                ask.sources,
                findings,
                said,
                ask.analysis,
                ask.function,
                read_out_of_memory_in_terminator_after(stored, terminator),
                known,
                ask.doubt_only,
            ),
            UnwrittenDoor::ByName => unwritten_read(
                ask.sources,
                findings,
                said,
                ask.analysis,
                assigned::terminator_reads(terminator),
                ask.taken,
                known,
                ask.doubt_only,
            ),
            UnwrittenDoor::CopyOut => {
                let Terminator::Call {
                    callee,
                    arguments,
                    origin,
                    ..
                } = terminator
                else {
                    continue;
                };
                if !matches!(ask.analysis.callee(*callee), Callee::Copies) {
                    continue;
                }
                let into_ty = match arguments.first() {
                    Some(Operand::Copy(place)) => ask.analysis.unit.place_ty(ask.function, place),
                    Some(Operand::Constant(_)) | None => None,
                };
                // **Whether what it copies was written**: `int *a[1];
                // memcpy(out, a, 8);` leaves the caller a pointer nobody
                // wrote, where `*out = a[0];` is `SC0411` at the read (#764).
                if copied_unwritten(ask.analysis, ask.function, terminator, into_ty, known) {
                    push_once(
                        findings,
                        Finding {
                            function: ask.analysis.function,
                            kind: Kind::CopiedUnwritten,
                            conclusion: Conclusion::Unknown,
                            at: origin.span(),
                            local: None,
                            freed: None,
                            made: None,
                            unproven: None,
                        },
                    );
                } else if into_ty.is_some_and(|ty| ask.analysis.first_level_holds_a_pointer(ty))
                    || arguments.first().is_some_and(|dest| {
                        points_at_a_pointer(ask.analysis, ask.function, dest, known).is_some()
                    })
                {
                    // **Or a copy into memory of pointers out of a block a
                    // copy wrote part of a pointer into**, at that copy, as
                    // a load of the pointer would be: `memcpy(&s, m, 8);
                    // *pp = s;` built silent (#907). The blocks the source
                    // names at the copy's depth, as the transfer carries
                    // them. Not where what it copies is said unwritten
                    // above, the same defect said more strongly. By the
                    // record of the block the site names now, as a load
                    // asks (ADR-0089). See ADR-0088.
                    //
                    // **Memory of pointers by what the destination points
                    // at, too**, as a partial copy is asked (ADR-0071 rule
                    // 4): `void *d = &s; memcpy(d, m, 8);` with `s` a
                    // `void *` built silent where `void **d` was asked,
                    // found by review. The destination only: the source is
                    // the block with the record, which says nothing of
                    // where the pointer goes.
                    //
                    // **Not through a pointer a store has written through
                    // since it was given its value**, as a load is not: the
                    // copy reads the whole pointer just stored (#914).
                    if let Some(Operand::Copy(source)) = arguments.get(1) {
                        let depth = derefs(source);
                        if (source.projection.is_empty() || depth > 0)
                            && !(depth == 0 && known.points_to[source.local].stored_through)
                        {
                            let (sites, locals) = known.levels_below(source.local, depth);
                            let places = sites
                                .into_iter()
                                .map(Loc::Heap)
                                .chain(locals.into_iter().map(Loc::Local));
                            ask_partial(
                                PartialDoor::CopyOut,
                                places,
                                findings,
                                ask.analysis,
                                known,
                            );
                        }
                    }
                }
            }
            // A `return` is an assignment to the return place, an element.
            UnwrittenDoor::Return => {}
            UnwrittenDoor::LeftAtExit => {
                if matches!(terminator, Terminator::Return) {
                    report_left_unwritten(ask, findings, known);
                }
            }
        }
    }
}

/// Report every block a store let leave holding pointers that is certainly
/// unwritten where the function returns, at the store (ADR-0076).
///
/// **And possibly, wherever it is**: the path where an allocation failed
/// leaves no block, and the null edge of a test of that allocation's result
/// says so (`branch_on_an_allocations_result`), so `if (b) *b = x; *out = b;`
/// is written on every path where the block exists. See ADR-0080.
///
/// **Not a block freed since**: what reaches the caller is a freed pointer,
/// which `SC0414` reports, and calling its memory unwritten beside that was
/// wrong words, found by review. Mutation: ask a freed block too;
/// `a_block_stored_out_and_freed_unwritten_is_reported_freed_alone` gains the
/// unwritten doubt.
fn report_left_unwritten(ask: &UnwrittenAsk<'_, '_>, findings: &mut Vec<Finding>, known: &Known) {
    // **And what left before a typed local said it held pointers**: a block
    // that left through a `void *` and was held as `int **` after is asked
    // now, at the write that let it leave, with what its record reaches
    // below. See `Known::stored_out`, ever, and ADR-0086.
    let mut asked: Vec<(Site, Span)> = known
        .state
        .indices()
        .filter_map(|site| known.left_at[site].map(|at| (site, at)))
        .collect();
    for root in known.state.indices() {
        let Some(at) = known.stored_out[root].ever() else {
            continue;
        };
        for site in known.held_below(vec![root], 0) {
            if !asked.iter().any(|&(named, _)| named == site) {
                asked.push((site, at));
            }
        }
    }
    // **And whether a copy wrote part of a pointer into it**, at the copy:
    // the block leaves as memory of pointers, and the caller reads a pointer
    // made of part of something else (#832). See `Known::part_written` and
    // ADR-0087.
    ask_partial(
        PartialDoor::Exit,
        asked.iter().map(|&(site, _)| Loc::Heap(site)),
        findings,
        ask.analysis,
        known,
    );
    for (site, at) in asked {
        if matches!(known.state[site], SiteState::Freed { .. }) {
            continue;
        }
        let kind = if known.written[site].whole().is_unwritten() {
            Kind::StoredUnwritten
        } else if known.written[site].whole().may_be_unwritten() {
            Kind::StoredMaybeUnwritten
        } else {
            continue;
        };
        push_once(
            findings,
            Finding {
                function: ask.analysis.function,
                kind,
                conclusion: Conclusion::Unknown,
                at,
                local: None,
                freed: None,
                made: made_where_all_agree(&[site], known),
                unproven: None,
            },
        );
    }
}

/// Whether `ty` is a pointer whose type says nothing of what it points at,
/// a `void *`: a receiver of one is asked by what the block's own record
/// says it was held as (ADR-0086), where a receiver of any other pointer
/// reads what its type says. One answer for a `return` and a call's
/// argument (#914).
pub(super) fn says_nothing_of_its_pointee(analysis: &Allocations<'_>, ty: TyId) -> bool {
    match analysis.unit.ty(ty) {
        Ty::Pointer(pointee) => matches!(analysis.unit.ty(pointee), Ty::Void),
        Ty::Int | Ty::Char | Ty::Void | Ty::Array { .. } => false,
    }
}

/// The doors that ask whether a write left part of a pointer in what they
/// reach (ADR-0087, #914): each passes the places it reaches to
/// [`ask_partial`], which reads the record the door names here, so that
/// which record a door reads is said in one place. A `match` with no
/// wildcard, so a door added answers which it reads (`error[E0004]`).
#[derive(Clone, Copy)]
pub(super) enum PartialDoor {
    /// What left the function, asked at its exit.
    Exit,
    /// What a `return` hands the caller.
    Return,
    /// What a call is handed.
    Call,
    /// A pointer read out of memory.
    Load,
    /// A library copy out of memory.
    CopyOut,
}

impl PartialDoor {
    /// What this door reads of `at`'s record: **kept across a site made
    /// again where what is asked leaves**, since the caller or the callee
    /// may hold the block the site named before; **made since, where it is
    /// read here**, since a read reads the block the site names now
    /// (ADR-0089).
    ///
    /// Mutation: a read asks `ever`, and
    /// `a_pointer_read_out_of_a_block_each_turn_makes_before_a_partial_copy_into_it_is_not_reported`
    /// is reported. **No case holds the doors where it leaves reading
    /// `ever`, measured**, nor the exit's `Known::stored_out` doing so: a
    /// site made again in a loop meets the turn that wrote it at the loop's
    /// head, so `now` still holds the write wherever the block leaves.
    fn record(self, known: &Known, at: Loc) -> Option<Span> {
        match self {
            PartialDoor::Exit | PartialDoor::Return | PartialDoor::Call => {
                known.part_written[at].ever()
            }
            PartialDoor::Load | PartialDoor::CopyOut => known.part_written[at].now(),
        }
    }

    /// Where the report says the memory was asked (#935).
    fn asked(self) -> PartAsked {
        match self {
            PartialDoor::Exit | PartialDoor::Return | PartialDoor::Call => PartAsked::Leaves,
            PartialDoor::Load | PartialDoor::CopyOut => PartAsked::Read,
        }
    }
}

/// Report, at the write, each of `places` whose record `door` reads says a
/// write left part of a pointer there.
pub(super) fn ask_partial(
    door: PartialDoor,
    places: impl IntoIterator<Item = Loc>,
    findings: &mut Vec<Finding>,
    analysis: &Allocations<'_>,
    known: &Known,
) {
    for at in places {
        if let Some(copy) = door.record(known, at) {
            push_part_of_a_pointer(findings, analysis, copy, door.asked());
        }
    }
}

/// Report the copy at `copy`, which wrote part of a pointer into a block
/// now held as memory of pointers, unless the copy already stands reported
/// as a store departure, which says the same thing knowing the type: once
/// per copy, whichever door asks first, in the words of that door. See
/// `Known::part_written` and ADR-0087.
///
/// Mutation: leave `Kind::PartOfAPointer` out of what has been said, and
/// `a_partial_pointer_copied_into_a_block_handed_twice_is_reported_once`
/// reports its copy twice, among six cases.
fn push_part_of_a_pointer(
    findings: &mut Vec<Finding>,
    analysis: &Allocations<'_>,
    copy: Span,
    asked: PartAsked,
) {
    let said = findings.iter().any(|said| {
        said.at == copy
            && matches!(
                said.kind,
                Kind::HandedOnNarrow {
                    how: Departure::Store,
                    ..
                } | Kind::PartOfAPointer { .. }
            )
    });
    if !said {
        findings.push(part_of_a_pointer_written(analysis, copy, asked));
    }
}

/// Whether `said` is the deferred ask's finding at `at`, which a store
/// departure at the same caret takes the place of.
pub(super) fn is_part_of_a_pointer_written(said: &Finding, at: Span) -> bool {
    said.at == at && matches!(said.kind, Kind::PartOfAPointer { .. })
}

/// A write at `copy` of part of a pointer into a block later held as memory
/// of pointers, asked where `asked` says. See `Known::part_written` and
/// ADR-0087.
fn part_of_a_pointer_written(analysis: &Allocations<'_>, copy: Span, asked: PartAsked) -> Finding {
    Finding {
        function: analysis.function,
        kind: Kind::PartOfAPointer { asked },
        conclusion: Conclusion::Unknown,
        at: copy,
        local: None,
        freed: None,
        made: None,
        unproven: Some(Unproven::Extent),
    }
}

/// Push `finding` unless one answering its question already stands at its
/// caret: the every-turn walk and a loop's first turn may both reach one
/// `return` or one copy, and one defect is reported once (#838). Mutation:
/// push always; `a_char_buffer_written_in_one_turn_is_unwritten_again_in_the_next`
/// and `a_block_an_inner_loop_makes_unwritten_again_is_doubted_where_read_after_both`
/// report a caret twice.
///
/// **A returned block certainly unwritten and one possibly so answer one
/// question**, and the certain answer is the one kept: the first turn's arm
/// says possibly where the every-turn walk said certainly, and both stood at
/// one `return`, found by review. Mutation: compare kinds alone;
/// `a_return_reached_by_both_walks_is_reported_once` reports its first
/// `return` twice.
fn push_once(findings: &mut Vec<Finding>, finding: Finding) {
    // Each pair, certain then possible, answers one question.
    let pair = |kind: &Kind| match kind {
        Kind::ReturnedUnwritten | Kind::ReturnedMaybeUnwritten => Some(0),
        Kind::StoredUnwritten | Kind::StoredMaybeUnwritten => Some(1),
        _ => None,
    };
    let certain = |kind: &Kind| matches!(kind, Kind::ReturnedUnwritten | Kind::StoredUnwritten);
    let same = |said: &Finding| {
        said.at == finding.at
            && (said.kind == finding.kind
                || (pair(&said.kind).is_some() && pair(&said.kind) == pair(&finding.kind)))
    };
    match findings.iter().position(same) {
        None => findings.push(finding),
        // The certain answer takes the place of the possible one.
        Some(index) if certain(&finding.kind) && !certain(&findings[index].kind) => {
            findings[index] = finding;
        }
        Some(_) => {}
    }
}

/// Report a `return` of a pointer that may name a block **certainly** or
/// **possibly** unwritten, where the return type points at memory holding
/// pointers: the return-side counterpart of `loaded_indeterminate`, which
/// reports a read of such a block inside this function. Only a `return`,
/// since a block handed to a call is how a callee is given memory to fill
/// (ADR-0073 rule 3). Possibly wherever it is: `if (m) *m = x; return m;`
/// joins a path with no block at all, and that path's block is written
/// vacuously on the null edge, so it adds no doubt. See ADR-0080.
///
/// **A value read through dereferences is asked of what a load that deep may
/// name**, `Known::levels_below`, the one walk every reader of a load takes
/// (ADR-0045), so `return *box;` answers as `t = *box; return t;` does.
/// Asked of a local by name alone, `return *box;`, `return box[0];` and
/// `return **bb;` were silent where their spelling through `t` was reported
/// (#876). Such a value is named by no local, so the finding names none: the
/// local it was read through is not what points at the memory. It inherits
/// what a load names: memory keeps what an earlier store put there beside
/// what a later one did, so `*box = b; *box = d; return *box;` with `b` unwritten
/// says nothing wrote it, as `t = *box; return t;` does, a report the reader
/// sees.
fn report_returned_unwritten(
    ask: &UnwrittenAsk<'_, '_>,
    findings: &mut Vec<Finding>,
    operation: &Operation,
    known: &Known,
) {
    let UnwrittenAsk {
        analysis,
        function,
        doubt_only,
        ..
    } = *ask;
    let Rvalue::Use(Operand::Copy(place)) = &operation.value else {
        return;
    };
    // **What the returned value may name**, or `None` where it is read
    // through a projection with no dereference, memory this check does not
    // follow: that is doubted, as a copy's source is
    // (`source_may_be_unwritten`), since what is not proven is `Unknown`. No
    // C reaches it until structs do, so nothing guards that arm.
    let sites: Option<Vec<Site>> = if place.projection.is_empty() {
        Some(known.sites_of(place.local).collect())
    } else {
        match derefs(place) {
            0 => None,
            depth => Some(
                known
                    .levels_below(place.local, depth)
                    .0
                    .into_iter()
                    .collect(),
            ),
        }
    };
    let named = place.projection.is_empty().then_some(place.local);
    // **Whether the caller reads pointers out of it**, by `pointee_width`,
    // which answers only for a pointer, `void *` as `0`; through an array to
    // its element, as `int *(*)[2]` holds pointers too. Found by review.
    let returns_pointers = analysis
        .unit
        .place_ty(function, &Place::local(function.return_place()))
        .and_then(|ty| pointee(analysis, ty))
        .and_then(|ty| stored_pointer(analysis.unit, ty))
        .and_then(|ty| pointee_width(analysis, ty))
        .is_some();
    // **Or where what it names was held as pointers**, for a return type
    // that says nothing there, a `void *`: `int **b = malloc(8); return b;`
    // as a `void *` hands the caller memory it reads pointers out of once it
    // converts the result back (#902). Only a pointer returned: an `int`
    // read out of such a block was asked too, and 26 cases moved, measured.
    // See `Known::held_levels` and ADR-0086.
    let returns_a_pointer = analysis
        .unit
        .place_ty(function, &Place::local(function.return_place()))
        .is_some_and(|ty| says_nothing_of_its_pointee(analysis, ty));
    let held_as_pointers = sites
        .iter()
        .flatten()
        .any(|&site| known.held_levels[site] > 0);
    if !returns_pointers && !(returns_a_pointer && held_as_pointers) {
        return;
    }
    // **And the blocks those hold**, as far down as the return type reads
    // pointers: `*box = b; return box;` hands the caller `b`'s block, and
    // was silent with `b` unwritten (#900). The walk a store and a copy that
    // leave are marked by, so the three doors follow the same levels. See
    // ADR-0085.
    let levels = analysis
        .unit
        .place_ty(function, &Place::local(function.return_place()))
        .map_or(0, |ty| analysis.levels_holding_pointers(ty));
    let below: Vec<Site> = sites.as_ref().map_or_else(Vec::new, |own| {
        known
            .held_below(own.clone(), levels)
            .into_iter()
            .filter(|site| !own.contains(site))
            .collect()
    });
    // **And whether a copy wrote part of a pointer into any of them**, at the
    // copy: the caller reads pointers out of what is returned, found by
    // review. See `Known::part_written` and ADR-0087.
    ask_partial(
        PartialDoor::Return,
        sites
            .iter()
            .flatten()
            .chain(&below)
            .map(|&site| Loc::Heap(site)),
        findings,
        analysis,
        known,
    );
    // **Any block it may name**, not every: one written elsewhere does not
    // make the other one written, and `r = c ? a : b;` with only `a` written
    // was silent. The path where an allocation failed names no block, so it
    // adds nothing here. Found by review.
    let certainly_own = sites
        .iter()
        .flatten()
        .any(|&site| known.written[site].whole().is_unwritten());
    // **Possibly, wherever it is**: the path where an allocation failed adds
    // no doubt, because its null edge writes the block it did not make. See
    // ADR-0080.
    let possibly_own = sites.as_ref().is_none_or(|sites| {
        sites
            .iter()
            .any(|&site| known.written[site].whole().may_be_unwritten())
    });
    let certainly = certainly_own
        || below
            .iter()
            .any(|&site| known.written[site].whole().is_unwritten());
    let possibly = possibly_own
        || below
            .iter()
            .any(|&site| known.written[site].whole().may_be_unwritten());
    // **On a first turn's arm, certainly unwritten is a doubt**: that turn
    // may not take the arm, and a later one finds the block written (#838).
    // Mutation: ignore `doubt_only`; the return in
    // `a_return_on_an_arm_of_a_loop_shown_to_run_before_its_block_is_written_is_doubted`
    // is said to name memory nothing wrote.
    let kind = if certainly && !doubt_only {
        Kind::ReturnedUnwritten
    } else if certainly || possibly {
        Kind::ReturnedMaybeUnwritten
    } else {
        return;
    };
    // **The returned local is named only where its own blocks bear out the
    // words said**: where only a block below them does, "`box` points at
    // memory nothing wrote" is false of a `box` that was written, and the
    // nameless words are said instead. Bear out, not merely are in doubt:
    // with `box` written on one path and `b` below it on none, the words are
    // certain and `box` is only possibly unwritten, found by review.
    let own_bears_out = match kind {
        Kind::ReturnedUnwritten => certainly_own,
        _ => possibly_own,
    };
    let named = named.filter(|_| own_bears_out);
    push_once(
        findings,
        Finding {
            function: analysis.function,
            kind,
            conclusion: Conclusion::Unknown,
            at: operation.origin.span(),
            local: named,
            freed: None,
            made: None,
            unproven: None,
        },
    );
}

/// Whether a library copy may copy bytes nothing wrote into memory that
/// leaves this function and is believed to hold a pointer. Copying them is
/// defined (C17 7.24.2.1, 7.24.2.2); a caller reading the pointer they leave is not,
/// and the copy is the last place this check sees it (ADR-0071 rule 5).
///
/// All three are asked, and each one alone keeps a copy silent:
///
/// - **the destination leaves**, as the store the copy is leaves
///   (`Allocations::store_at`): a copy into memory only this function reads
///   is asked at the read, as any read of it is;
/// - **what it lands in is believed to hold a pointer**, by the
///   destination's type or by what it points at ([`points_at_a_pointer`]);
/// - **what it copies may be unwritten** ([`source_may_be_unwritten`]).
///
/// **Leaving is the store's answer, and a library copy exposes what it is
/// handed**, so a second copy into a block this function made, a copy in a
/// loop among them, is asked as leaving: copying unwritten bytes into it is
/// doubted there as well as where it is read, and a block filled that way
/// and freed unread is doubted though nothing reads it. That is the
/// conservative side of the store's question, and answering it otherwise
/// here would answer one destination two ways (#737).
fn copied_unwritten(
    analysis: &Allocations<'_>,
    function: &Function,
    terminator: &Terminator,
    into_ty: Option<TyId>,
    known: &Known,
) -> bool {
    let Terminator::Call {
        callee, arguments, ..
    } = terminator
    else {
        return false;
    };
    let Some(store) = analysis.store_at(terminator, known) else {
        return false;
    };
    let (StoreKind::Copy, Written::Bytes(from)) = (store.kind, &store.from) else {
        return false;
    };
    if !store.leaves {
        return false;
    }
    let typed = into_ty
        .and_then(|ty| pointee(analysis, ty))
        .and_then(|ty| stored_pointer(analysis.unit, ty))
        .and_then(|ty| pointee_width(analysis, ty))
        .is_some();
    let holds = typed
        || arguments
            .first()
            .and_then(|into| points_at_a_pointer(analysis, function, into, known))
            .is_some();
    holds && source_may_be_unwritten(analysis, *callee, arguments, from, known)
}

/// Whether the bytes a library copy reads through `from` may not have been
/// written: an allocation or a local it points at that may be unwritten.
/// **At the slots the copy covers**, where `from` points at a known slot of
/// a place kept per slot and the length is a constant, so that `a[0] = &x;
/// memcpy(out, a, 8);` with `a` two pointers long is not doubted for `a[1]`;
/// at any slot otherwise, which doubts. Slots are counted in pointers, since
/// only memory of pointers is kept per slot (`Known::written`), and a copy
/// ending part way through one covers it, hence the rounding up. Any other
/// place is kept whole and asked as one.
///
/// **A source that holds a pointer read out of memory is asked by what it
/// holds**, as any source is, which is a doubt where a load through it is one
/// (`Allocations::reads_partly`).
fn source_may_be_unwritten(
    analysis: &Allocations<'_>,
    callee: FuncId,
    arguments: &[Operand],
    from: &Place,
    known: &Known,
) -> bool {
    // **A source spelled through dereferences**, `memcpy(out, *b, 8)`, is
    // handed to the call as the place itself, not loaded into a local
    // first, so what it copies is what a load one level further reads
    // (`Known::levels_below`): answering `false` for it said the bytes were
    // written whenever the source was spelled that way. A place of
    // anything but dereferences names memory this check does not follow,
    // and is doubted. Found by review. **Nothing guards that last arm**: an
    // indexed source, `a[0]` or `a[k]`, reaches the call as a dereference,
    // measured, and no C program found reaches it otherwise.
    if !from.projection.is_empty() {
        let depth = derefs(from);
        if depth == 0 {
            return true;
        }
        let (sites, locals) = known.levels_below(from.local, depth);
        return sites
            .into_iter()
            .any(|site| known.written[site].whole().may_be_unwritten())
            || locals.into_iter().any(|local| {
                known.certainly_unwritten(local, None) || known.may_be_unwritten(local, None, true)
            });
    }
    // **One rule for a block and a local, and the one the copy's transfer
    // reads its source by** (`Allocations::slots_read`,
    // `Allocations::slots_read_of_locals`): the slots the copy covers where
    // the place keeps slots, from the slot the pointer is known to point at.
    // A block's was asked whole, so `b[0] = x; memcpy(out, b, 8);` was
    // doubted for `b[1]` (#894); and read here apart from the transfer, a
    // local array was asked here slot by slot and there at any element, so
    // `a[0] = x; memcpy(out, a, 8); r = *out;` was doubted at `*out` and
    // asked nothing at the copy (the memory model rebuilt on #924, stage 2).
    let held = &known.points_to[from.local];
    let bytes = analysis.constant_argument(callee, arguments, 2);
    let site_slots = analysis.slots_read(held, bytes);
    let local_slots = analysis.slots_read_of_locals(held, bytes);
    held.sites()
        .any(|site| known.written[site].over(site_slots).may_be_unwritten())
        || held
            .locals()
            .any(|local| known.written[local].over(local_slots).may_be_unwritten())
}

/// Whether what `operand` points at holds a pointer, by what it points at
/// rather than by its type, and if so the width that pointer is believed to
/// point at. Asked of a library copy's operands, where their types said
/// nothing (#746) or where only the destination is asked (#764). See
/// ADR-0071 rules 4 and 5.
///
/// What an operand points at holds a pointer three ways, each asked here:
///
/// - **a local it holds the address of**, by the local's declared type, or
///   its elements' where it is an array, at the width that type points at;
/// - **the memory it was believed to point at when it arrived**, from the
///   type it arrived as, which a conversion to `void *` keeps: `void *d =
///   out;` with `out` an `int **` (`Held::contents_at_most`), or a
///   `void **`, which believes `0` there as an `int *` does and is told
///   apart by `Held::arrived_holds_pointer` (#818);
/// - **an allocation it points at**, where a pointer was stored in it, of a
///   width nothing records here, so `0`, as a `void *` is asked only whether
///   it starts an object.
///
/// Where several locals answer, the narrowest width is said, as the message
/// names a width a reader may believe; it decides no report. An operand
/// that points at none of these answers nothing.
pub(super) fn points_at_a_pointer(
    analysis: &Allocations<'_>,
    function: &Function,
    operand: &Operand,
    known: &Known,
) -> Option<u64> {
    let Operand::Copy(place) = operand else {
        return None;
    };
    if !place.projection.is_empty() {
        return None;
    }
    let named = known.written_through(place.local);
    let local = function
        .locals()
        .filter(|&local| named.contains(&LocalIdx::of(local)))
        .filter_map(|local| {
            let ty = innermost_element(analysis.unit, function.local(local));
            pointee_width(analysis, ty)
        })
        .min();
    // `0` alone is no belief that a pointer is held: `type_levels` says it
    // from the first level that holds none, so an `int *` arrives believing
    // `0` as a `void **` does, and counting it doubted every copy of an
    // `int` through an `int *` parameter. Whether the type it arrived as
    // holds a pointer there tells them apart (#818). See ADR-0073 rule 4.
    let held = &known.points_to[place.local];
    let arrived = held
        .contents_at_most()
        .at(0)
        .bound()
        .filter(|&width| width > 0)
        .or(held.arrived_holds_pointer.then_some(0));
    let stored = known.sites_of(place.local).any(|site| {
        let cell = &known.contents[site];
        cell.sites().next().is_some() || cell.locals().next().is_some()
    });
    local.or(arrived).or(stored.then_some(0))
}

/// Report every read at `at` of a pointer out of memory nothing wrote, which
/// is a use of an indeterminate value (C17 7.22.3.1 p2, 7.22.3.4 p2, Annex
/// J.2), once per place per caret. The later uses of what it read are not
/// reported again. See ADR-0057.
///
/// `doubt_only` where `known` is a loop's first turn at a read that turn may
/// not run, which says no more than a doubt (#670).
#[allow(clippy::too_many_arguments)]
pub(super) fn loaded_indeterminate(
    sources: &SourceMap,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    at: Option<(Span, Vec<Place>)>,
    known: &Known,
    doubt_only: bool,
) {
    let Some((at, read)) = at else {
        return;
    };
    for place in read {
        // A proof where nothing wrote what it reads, and a doubt where an
        // array it reads at its start was written only in part. See
        // `Known::written`.
        let certain = analysis.reads_fresh(function, &place, known);
        let maybe = !certain && analysis.reads_partly(function, &place, known);
        if !certain && !maybe {
            // **Or a pointer read out of a block a copy wrote part of a
            // pointer into**, at the copy: `void **b = m; void *s = *b;`
            // after `memcpy(m, &i, 4)` built silent (#907). Only where the
            // read is not said indeterminate, which is the same defect said
            // more strongly. By the record of the block the site names now,
            // since that is the block read (ADR-0089). See ADR-0088.
            //
            // **Nor through a pointer a store has written through since it
            // was given its value**, on every path (`Held::stored_through`):
            // the read is of the whole pointer just stored, the lowering's
            // own read of an assignment's value among them. Asked there,
            // `void **b = m; *b = x;` after the partial copy was doubted
            // with no read in the source at all, found by review.
            let rewritten = derefs(&place) == 1 && known.points_to[place.local].stored_through;
            if !rewritten && analysis.may_be_pointer(function, &place) {
                ask_partial(
                    PartialDoor::Load,
                    blocks_read(&place, known),
                    findings,
                    analysis,
                    known,
                );
            }
            continue;
        }
        // **Keyed by the pointer read through, not by `*l`.** A dereference
        // of `l` at the same caret is said at `*l` by `used`, and one key
        // keeps one finding: a read out of a block nothing wrote before it
        // was freed lost its use after free to this report, though both are
        // true of it. Mutation: key by `place`, and
        // `a_pointer_read_out_of_a_freed_table_and_returned` loses `SC0402`.
        let local = place.local;
        let through = &known.points_to[local];
        // **Below the first dereference, what the pointer gone through last
        // may name**, and no name: the slot is not `*l` but a level further,
        // and saying `*m` of `int *r = **m;` named the wrong slot (#768).
        let depth = derefs(&place);
        let (reaches_allocation, reaches_local, named) = if depth > 1 {
            let (sites, locals) = known.levels_below(local, depth - 1);
            (!sites.is_empty(), !locals.is_empty(), None)
        } else {
            (
                through.sites().next().is_some(),
                through.may_be_a_locals_address(),
                Some(local),
            )
        };
        // **Where the bytes came from, where a copy or a `realloc` carried
        // them**: asked of each place read that may be unwritten, and said
        // only where every one names the same call, since a read that may
        // reach two such places has no one source to name (#752).
        let mut origins = blocks_read(&place, known)
            .into_iter()
            .filter(|&at| known.written[at].whole() != Init::Written)
            .map(|at| known.written[at].carried());
        let carried = origins
            .next()
            .flatten()
            .filter(|first| origins.all(|other| other == Some(*first)));
        let kind = Kind::IndeterminateLoad {
            reaches_allocation,
            reaches_local,
            carried,
        };
        let root = Place::local(local);
        let conclusion = if maybe || doubt_only {
            Conclusion::Unknown
        } else {
            Conclusion::Unsafe
        };
        if !may_say(findings, said, at, &root, conclusion) {
            continue;
        }
        say(
            sources,
            findings,
            said,
            &root,
            Finding {
                function: analysis.function,
                kind,
                conclusion,
                at,
                local: named,
                freed: None,
                made: None,
                unproven: None,
            },
        );
    }
}

/// The blocks a load of `place` reads a pointer out of: what the pointer
/// read through names, and below the first dereference, what the pointer
/// gone through last may name, as [`loaded_indeterminate`] names the slot
/// (#768). **Locals as well as blocks**, since a local array keeps the
/// partial pointer record too (ADR-0090).
fn blocks_read(place: &Place, known: &Known) -> Vec<Loc> {
    let depth = derefs(place);
    let (sites, locals): (Vec<Site>, Vec<LocalIdx>) = if depth > 1 {
        let (sites, locals) = known.levels_below(place.local, depth - 1);
        (sites.into_iter().collect(), locals.into_iter().collect())
    } else {
        let held = &known.points_to[place.local];
        (held.sites().collect(), held.locals().collect())
    };
    sites
        .into_iter()
        .map(Loc::Heap)
        .chain(locals.into_iter().map(Loc::Local))
        .collect()
}

/// Whether a read reported indeterminate at `at` through `root` may be said:
/// where nothing stands there yet, or where what stands is a doubt and this
/// is a proof. **A proof gives way to nothing, and takes the place of a doubt**
/// (#670): a loop's first turn proves a read the every-turn walk only doubted,
/// `r = q; if (c) q = &x;` in a body, and skipping whatever stood left that
/// proof a warning `--allow-unknown` lowers. Found by review. Not handed to
/// [`say`] for every pair, whose `supersedes` takes two proofs to name a free.
fn may_say(
    findings: &[Finding],
    said: &[(Span, Place, usize)],
    at: Span,
    root: &Place,
    conclusion: Conclusion,
) -> bool {
    match said
        .iter()
        .find(|(said_at, said_place, _)| *said_at == at && said_place == root)
    {
        None => true,
        Some(&(_, _, index)) => {
            findings[index].conclusion == Conclusion::Unknown && conclusion == Conclusion::Unsafe
        }
    }
}

/// Report every read by name of a local that holds a pointer before anything
/// wrote it on any path: an indeterminate value used (C17 6.2.4 p6, 6.7.9
/// p10, Annex J.2), once per local per caret. `int *q; int **pq = &q; return
/// q == 0;` (#640).
///
/// `taken` is `assigned::address_taken`. A local whose address is taken is
/// asked here alone, since `memory::assigned` follows no write through an
/// address and this one does ([`Known::written`]). One whose address is never
/// taken is `memory::assigned`'s first, and is asked here only where it may
/// be unwritten and nothing was said of it at the caret: one rule for a
/// read some path left unassigned, whatever happens to the local's address.
/// See ADR-0057 and ADR-0075.
///
/// `doubt_only` as [`loaded_indeterminate`] takes it.
#[allow(clippy::too_many_arguments)]
pub(super) fn unwritten_read(
    sources: &SourceMap,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    reads: Vec<(LocalId, Span)>,
    taken: &Bits<LocalIdx>,
    known: &Known,
    doubt_only: bool,
) {
    let function = analysis.unit.function(analysis.function);
    for (local, at) in reads {
        // **A doubt where an element may still be unwritten**, a proof
        // where nothing wrote any of it. See `Known::written`. **Only a local
        // that holds a pointer**: every local starts unwritten, and an
        // integer read as itself is not followed (ADR-0073 rule 2).
        let init = known.written[local].whole();
        let partly = init == Init::Maybe;
        // **A local whose address is never taken is asked here only for the
        // doubt**, and only where `memory::assigned`, which runs first and
        // proves or doubts it by name, said nothing at this caret: one rule
        // for a read some path left unassigned, whatever happens to the
        // local's address. **Only `memory::assigned`'s findings count**: one
        // this check made of the same read, a free of it on a loop's later
        // turn, says nothing about whether it was assigned. See ADR-0075.
        let by_assigned = |said: &Finding| {
            matches!(
                said.kind,
                Kind::Unassigned | Kind::SometimesUnassigned { .. }
            )
        };
        let by_name = !taken.get(LocalIdx::of(local));
        if (by_name
            && (!(partly || doubt_only)
                || findings
                    .iter()
                    .any(|said| said.at == at && said.local == Some(local) && by_assigned(said))))
            || init == Init::Written
            || stored_pointer(analysis.unit, function.local(local)).is_none()
        {
            continue;
        }
        let root = Place::local(local);
        let conclusion = if partly || doubt_only {
            Conclusion::Unknown
        } else {
            Conclusion::Unsafe
        };
        if !may_say(findings, said, at, &root, conclusion) {
            continue;
        }
        say(
            sources,
            findings,
            said,
            &root,
            Finding {
                function: analysis.function,
                kind: Kind::IndeterminateLocal,
                conclusion,
                at,
                local: Some(local),
                freed: None,
                made: None,
                unproven: None,
            },
        );
    }
}
