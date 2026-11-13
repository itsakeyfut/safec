//! The report: what [`findings`](super::findings) asks at each free,
//! dereference, `return` and call once the walk has settled, and how what it is
//! asked becomes a [`Finding`] with a conclusion, a caret and the spans a reader
//! is shown.

use std::collections::BTreeSet;

use crate::analysis::Conclusion;
use crate::ir::{
    BlockId, Element, FuncId, Function, LocalId, Operand, Place, Projection, Rvalue, Terminator,
    TyId,
};
use crate::library::{Callee, Reach};
use crate::nullability::NullAtTerminators;
use crate::reads::derefs;
use crate::source::{SourceMap, Span, inside};

use super::allocations::{Allocations, handed_places, stored_pointer};
use super::built::named;
use super::fit::{
    Fit, fits, folded, only_if_certain, pointee, pointee_width, read_from_locals, slots_leaving,
    through_locals,
};
use super::index::{Idx, Loc, LocalIdx, Site};
use super::known::Known;
use super::parts::{At, Extent, Freeing, Levels, Reached, Read, SiteState, Width, earlier, same};
use super::store::{StoreKind, Written};
use super::transfer::CopiedPointers;
use super::unwritten::{
    PartialDoor, ask_partial, is_part_of_a_pointer_written, points_at_a_pointer,
    says_nothing_of_its_pointee,
};
use super::{Departure, Finding, Kind, LostReason, Unproven};

/// What a set of sites says about whatever touched them.
///
/// The fold from many sites to one conclusion, and the one place either kind
/// of finding makes it. There is one because a may-analysis proves nothing
/// from one member of its set: its join is forced to be right by the lattice,
/// and the code that
/// reads the answer is where the same rule gets lost.
struct Verdict {
    conclusion: Conclusion,
    /// The earliest free reaching here, where there is one to name.
    freed: Option<Span>,
    /// Where that allocation came from, where this check saw it happen.
    made: Option<Span>,
    /// Why this could not be proven, and `None` where it was. Carried through
    /// to [`Finding::unproven`] unchanged.
    unproven: Option<Unproven>,
}

/// The local a pointer read with no dereference is the own value of, for
/// [`Finding::local`]: the place's local where nothing is projected from it.
///
/// **Not the root of a projection.** `free(*pp)` frees what was stored in
/// `pp`'s allocation, which no local holds, and naming `pp` for it would name
/// a pointer that was not freed. A dereference is the other case and is not
/// asked here: `*p` reads `p`'s own value and goes through it, so [`used`]
/// names the local for its first level whatever the projection is.
fn own_local(place: &Place) -> Option<LocalId> {
    place.projection.is_empty().then_some(place.local)
}

/// [`Finding::local`] for a read carried forwards, by which read it is.
///
/// A dereference is carried at its first level only, by `Known::met`, so it is
/// about the local's own value whatever the projection; a pointer handed on is
/// [`own_local`]'s, as it is where the call is met. Naming the root for a
/// pointer handed on fails
/// `a_pointer_read_out_of_memory_and_handed_on_is_carried_to_a_later_free`,
/// which hands `*tab` to a call beside a free.
fn pending_local(read: Read, place: &Place) -> Option<LocalId> {
    match read {
        Read::Dereference => Some(place.local),
        Read::Argument => own_local(place),
    }
}

/// What these sites amount to, or nothing where they amount to no report.
///
/// The caller decides what reaching nothing means, by what it puts in
/// `reached`: a free hands a [`Reached::Lost`] for an argument it stopped
/// following, and a dereference hands an empty iterator. That asymmetry is the
/// design rather than an accident, and [`used`] says why.
fn verdict(
    sources: &SourceMap,
    kind: Kind,
    reached: impl IntoIterator<Item = Reached>,
    known: &Known,
) -> Option<Verdict> {
    // The earliest free reaching here, and whether C has sequenced **every**
    // free that reaches here. [`Freeing::joined`] is both rules, because
    // folding over the sites reached at one point wants exactly what folding
    // over two paths wants: the earlier span, and the conjunction.
    let mut earliest: Option<Freeing> = None;
    // Where the allocation came from, kept only while every freed site agrees.
    // **One level down**: proving a double free from one of several sites
    // is the may-set mistake the fold below is written to avoid, and
    // naming one of several allocations as *the* one is the same mistake about
    // a label. `if (c) p = malloc(); else p = malloc();` reaches both, and
    // pointing at either would be a caret on an allocation the value may not
    // hold.
    let mut made: Option<Span> = None;
    let mut any_freed = false;
    let mut live = false;
    let mut unknown = false;
    // Kept apart from `unknown` because they are unproven for opposite
    // reasons, and the words a reader is given turn on which. A site the paths
    // disagree about was freed on one of them, and a site an opaque call was
    // handed may have been freed by it; a pointer this check lost says nothing
    // about any free anywhere. Counting both as one flag is what put
    // `may free it again here` on a program with one free in it. Why it was
    // lost travels with it, one reason for all of them, and `Other` where two
    // disagree. See #213.
    let mut lost: Option<LostReason> = None;
    let mut partial = false;
    // Whether more than one free was folded in. The span below is the earliest
    // of them and the flag beside it is the conjunction, so where they differ
    // the span can be a free this check *has* seen sequenced while the flag is
    // false because another was not. Saying "the order is what is open" about
    // that pair would point two carets at two evaluations C does order.
    let mut several = false;

    for entry in reached {
        let site = match entry {
            Reached::Site(site) => site,
            // A proof about the set: freeing it again takes the same member,
            // whichever it was. It carries no `made`, because naming one of
            // several allocations as the one that was freed is the may-set
            // mistake this whole rule is against.
            Reached::SetFreed(freed) => {
                several |= any_freed;
                any_freed = true;
                earliest = Some(match earliest {
                    Some(already) => already.joined(freed, sources),
                    None => freed,
                });
                continue;
            }
            // **Not the same as proving it live.** This is the check having
            // lost the pointer, and answering nothing about it is the failure
            // `docs/safety-model.md` is written to prevent rather than the one
            // it tolerates.
            Reached::Lost(reason) => {
                lost = Some(lost.map_or(reason, |already| already.joined(reason)));
                continue;
            }
            Reached::Partial => {
                partial = true;
                continue;
            }
        };

        match known.state[site] {
            SiteState::Live(_) => live = true,
            SiteState::Freed {
                made: from,
                freed: before,
            } => {
                made = if any_freed { same(made, from) } else { from };
                several |= any_freed;
                any_freed = true;
                earliest = Some(match earliest {
                    Some(already) => already.joined(before, sources),
                    None => before,
                });
            }
            SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => unknown = true,
        }
    }

    // **`points_to` is a may-set, so one freed site among several is not a
    // proof.** `SiteState::joined` already answers `Unknown` where one path
    // freed a site and another did not; this is the same question across two
    // sites rather than across two paths, and answering it differently let the
    // spelling of a program decide whether it was a warning or an error. A
    // proof needs every site reached to have been freed, and nothing about it
    // to have been lost.
    // Every site reached was freed and nothing about them was lost. That is
    // the whole of what this check can work out from the states; whether C has
    // put the free first is a separate question with a separate answer, and
    // running them together is a known mistake: one test meaning "proved" and
    // "gave up" at once reports the second as the first.
    // A set that may be missing members proves nothing. See ADR-0045.
    let settled = !live && !unknown && lost.is_none() && !partial;

    // **A double free does not turn on which ran first.** Two frees of one
    // allocation are a double free in either order, so there is nothing for a
    // sequence point to settle and asking for one turned `(free(p), 0) +
    // (free(p), 0)` from an error into a warning. Issue #142 said so in as many
    // words and this check did it anyway until review measured it. A use is the
    // other way round: one allowed order reads freed storage and another does
    // not, which is the whole of what this field is for.
    let ordered = match kind {
        Kind::DoubleFree => true,
        Kind::UseAfterFree => earliest.is_some_and(|freed| freed.sequenced),
        // Never asked: `interior` answers that one without folding frees at
        // all. `true` because a pointer that is not the start of an
        // allocation is the wrong thing to hand `free` whatever ran first.
        Kind::InteriorFree => true,
        // A `return` leaves the function after its whole expression, so a free
        // anywhere in it has run by then, and what leaves is freed in every
        // order C allows. See ADR-0041.
        Kind::ReturnAfterFree => true,
        // A store is asked of what the value held as it ran, as a return
        // is: what leaves leaves then. A free later in the same full
        // expression frees what was stored live, which is #650's case and
        // not this one. See ADR-0041.
        Kind::StoreAfterFree => true,
        // **Never asked, since neither is ever settled**: `handed_below`
        // always answers `Reached::Partial` beside what it finds, so these
        // are `Unknown` and the order decides nothing. `true` for the reasons
        // the two rows above give, should that change: what leaves leaves
        // after its expression, or is asked as the store runs.
        Kind::FreedBehindReturn | Kind::FreedBehindStore => true,
        // Never asked: each is built where it is said, at the free in
        // `reported` and at the call in `handed_stored_out`, not out of a
        // verdict, and is always a doubt. See `Known::stored_out`.
        Kind::FreedAfterStore | Kind::HandedAfterStore => true,
        // **Not a `return`'s answer.** A call is not the end of its full
        // expression, so `(free(a), 0) + use(a)` may call `use` before the
        // free, and answering `true` proved a program C defines on that order.
        // See ADR-0042.
        Kind::ArgumentAfterFree => earliest.is_some_and(|freed| freed.sequenced),
        // The same question, about what it points at. No proof reaches it, because
        // `handed_below` always answers `Reached::Partial` beside its sites.
        Kind::FreedBehindArgument => earliest.is_some_and(|freed| freed.sequenced),
        // Never asked: it is built from its element in `memory.rs`, not out of
        // a verdict, and it is about no free. See ADR-0052.
        Kind::UnsequencedAccess { .. } => true,
        // Never asked: it is built before any value exists, and it is about
        // no free.
        Kind::TooLarge { .. } | Kind::TooLong { .. } => true,
        // Never asked: it is built by `memory::assigned` from where a local
        // was assigned, and it is about no free.
        Kind::Unassigned | Kind::SometimesUnassigned { .. } => true,
        // Never asked: each is said by `loaded_indeterminate` or
        // `unwritten_read`, about no free.
        Kind::IndeterminateLoad { .. } | Kind::IndeterminateLocal => true,
        // Never asked: it is said by `may_not_fit`, about no free.
        Kind::OutsideItsObject { .. } => true,
        // Never asked: it is said where a pointer leaves, about no free.
        Kind::HandedOnNarrow { .. } => true,
        // Never asked: it is said where a copy leaves, about no free.
        Kind::CopiedUnwritten => true,
        // Never asked: it is said where a block is returned, about no free.
        Kind::ReturnedUnwritten => true,
        // Never asked, as the one above.
        Kind::ReturnedMaybeUnwritten => true,
        // Never asked: it is said where a block leaves by a store, about no
        // free.
        Kind::StoredUnwritten | Kind::StoredMaybeUnwritten => true,
        // Never asked: it is said at a write of part of a pointer, about no
        // free.
        Kind::PartOfAPointer { .. } => true,
    };

    match earliest {
        // **A null nothing here established does not weaken this.** The site a
        // parameter stands for carries no allocation of its own, so asking for one
        // before answering `Unsafe` would drop the proof on the commonest double
        // free there is, and C17 7.22.3.3 p2 exempts a failed allocation by the
        // same sentence it exempts a null caller passes. What this asserts is that
        // some execution of this function is undefined, which is ADR-0027.
        Some(freed) if settled && ordered => Some(Verdict {
            conclusion: Conclusion::Unsafe,
            freed: Some(freed.at),
            made,
            unproven: None,
        }),
        // **Unproven, and the free is still named.** What is open here is only
        // the order: the sites agree, nothing was lost, and there is exactly
        // one free to point at. A reader given two carets and the note beside
        // them can see the shape of it. See ADR-0022.
        Some(freed) if settled && !several => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: Some(freed.at),
            made,
            unproven: Some(Unproven::Unsequenced),
        }),
        // Neither span is carried. What makes this unproven is that the sites
        // or the paths disagree, so there is no one free that every execution
        // reaching here went through, and no one allocation to name beside it.
        Some(_) => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Disagreement),
        }),
        // **Nothing established a free at all**, so nothing here may say one
        // happened. This check lost the pointer and no site it reached says
        // otherwise, which is what `!unknown` is doing: a site the paths
        // disagree about, and a site an opaque call was handed, are each a
        // free worth suspecting, and the arm below is right about them.
        None if lost.is_some() && !unknown => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: lost.map(Unproven::Lost),
        }),
        None if unknown => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Disagreement),
        }),
        None => None,
    }
}

/// What this terminator is worth reporting as a free, if anything.
///
/// One finding per call and question rather than one per site: a local may
/// point at several allocations where a branch put them there, and two carets
/// on one `free` say one thing twice. **Three questions, though, and one caret
/// can carry them all**: in `free(p); free(p + 1);` the second call frees an
/// allocation already freed, and through a pointer that is not its start; and
/// a free of what was stored where code outside reads it is doubted as that
/// beside either (ADR-0071 rule 3).
///
/// Pushed in that order, the double free, then the stored-out doubt, then the
/// interior free, so that a caret carrying them reads `SC0401`, `SC0414`,
/// `SC0404` from the top: the sort at the end of [`findings`](super::findings)
/// is stable and they share a span. See ADR-0036.
pub(super) fn reported(
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
    null: &NullAtTerminators,
    block: BlockId,
) -> Vec<Finding> {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return Vec::new();
    };

    // What is handed to be freed: every argument of `free`, and the first of
    // `realloc`, which C17 7.22.3.5 p3 holds to what `free` is held to. A
    // `match` so that a new kind of callee answers here. See ADR-0039.
    let arguments = match analysis.callee(*callee) {
        Callee::Frees => &arguments[..],
        Callee::Reallocates => &arguments[..arguments.len().min(1)],
        Callee::Allocates
        | Callee::ReturnsFirst
        | Callee::Copies
        | Callee::Ends
        | Callee::Opaque => {
            return Vec::new();
        }
    };

    // **One answer to what the arguments reached feeds both questions**,
    // because the two have to agree about it, and two walks deciding it would
    // drift apart. The offset is read beside it from the same `asked`, so an
    // argument established null is left out of both or of neither.
    let reached = Allocations::touching(asked(arguments, null, block), known);
    let offset = asked(arguments, null, block)
        .filter_map(|argument| match argument {
            Operand::Copy(place) if place.projection.is_empty() => {
                Some(known.points_to[place.local].offset)
            }
            // A constant frees nothing and a projection is already a
            // `Reached::Lost` above, which `interior` answers nothing for.
            Operand::Copy(_) | Operand::Constant(_) => None,
        })
        .reduce(At::joined)
        .unwrap_or(At::Bytes(0));

    // One pointer handed to be freed, read as itself, is the value both
    // questions are about. Every free asks one, since `free` takes one argument
    // and `realloc` is asked about its first, so `None` here is a pointer read
    // out of memory or a constant rather than a call naming two.
    let local = match arguments {
        [Operand::Copy(place)] => own_local(place),
        [..] => None,
    };

    let finding = |kind, verdict: Verdict| Finding {
        function: analysis.function,
        kind,
        conclusion: verdict.conclusion,
        at: origin.span(),
        local,
        freed: verdict.freed,
        made: verdict.made,
        unproven: verdict.unproven,
    };

    // Asked first because `verdict` takes what was reached by value; pushed
    // last, for the order the doc comment gives.
    let inside = interior(&reached, offset, known);

    // What it frees that was stored where code outside may read it, asked
    // before `verdict` takes what was reached. See `Known::stored_out`.
    let stored_out: Vec<Site> = reached
        .iter()
        .filter_map(|reached| match reached {
            Reached::Site(site) if known.stored_out[*site].now().is_some() => Some(*site),
            _ => None,
        })
        .collect();

    let mut found = Vec::new();
    if let Some(verdict) = verdict(analysis.sources, Kind::DoubleFree, reached, known) {
        found.push(finding(Kind::DoubleFree, verdict));
    }
    if !stored_out.is_empty() {
        // **Beside any double free, not only where there is none**: the
        // double-free doubt may come from no earlier free at all, but from an
        // escape, `memcpy(pp, &x, 8)` taking `&x`, or a call that may free
        // what it was handed, and then the stored-out doubt is the reason a
        // reader can act on (#684). Second,
        // so a caret carrying both reads `SC0401` above `SC0414`. A doubt,
        // since whoever reads that memory may only write it (#650). See
        // ADR-0071 rule 3.
        let made = made_where_all_agree(&stored_out, known);
        found.push(Finding {
            function: analysis.function,
            kind: Kind::FreedAfterStore,
            conclusion: Conclusion::Unknown,
            at: origin.span(),
            local,
            freed: None,
            made,
            unproven: None,
        });
    }
    if let Some(verdict) = inside {
        found.push(finding(Kind::InteriorFree, verdict));
    }
    found
}

/// Whether a free hands `free` something other than the start of what it
/// reached.
///
/// C17 7.22.3.3 p2 makes a `free` of anything but a pointer an allocation
/// function returned undefined, and `p + 1` reaches the allocation `p` does,
/// so [`verdict`] cannot see this: the sites are the same and so are their
/// states. What differs is [`Held::offset`](super::parts::Held::offset).
///
/// Nothing, where any of these holds:
///
/// * a [`Reached::Lost`] is among them. ADR-0017 makes [`Known::reached_by`]
///   answer it for an escaped local that holds sites, so `int **pp = &p; *pp =
///   p + 1; free(p);` never reaches the offset at all, and neither does
///   anything a call this check cannot read may have written. The offset is a
///   positive claim about a local and the address-taken set is what stops it
///   being believed, which any analysis proving something positive about a
///   local owes. **That rule is what covers this line**, and the day something
///   narrows what an escaped local is reported as, this line stops being
///   covered: a field a stronger rule upstream answers for has no guard of
///   its own.
/// * a [`Reached::SetFreed`] is. The sites are gone from `reached` by then, so
///   there is nothing left to be an offset into.
/// * no site at all. A local that reaches nothing is one this check never
///   followed, and [`verdict`] already answers [`Unproven::Lost`] for the same
///   call. [`Allocations::touching`] pushes a `Reached::Lost` for an argument
///   whose local reaches no site, so this rule is reached with nothing in hand
///   in two ways only. One is after a `SetFreed`, which the arm above answers
///   too: **the two hold each other**, so removing either leaves the whole
///   workspace green, and removing both fails
///   `a_free_after_an_offset_that_kept_the_set_is_proved`. The other is a
///   constant argument, `free(0)`, which `touching` skips and so hands this
///   nothing at all; [`reported`] then hands this `At::Bytes(0)`, which says
///   nothing either. See ADR-0036.
///
/// **A parameter is proved on, and that is sound.** `void f(int *p) { free(p +
/// 1); }` can only free the start of an object if a caller passed a `p` one
/// element before the start of one, which C17 6.5.6 p8 already makes
/// undefined, so every conforming caller leaves this call undefined. ADR-0027
/// is the record that says a conclusion is about an execution this function
/// has.
///
/// `freed` is `None` whatever the answer: what this reports is not about a free
/// that already happened.
fn interior(reached: &[Reached], offset: At, known: &Known) -> Option<Verdict> {
    let mut sites = Vec::new();
    for entry in reached {
        match entry {
            Reached::Site(site) => sites.push(*site),
            // `SetFreed` is answered twice: `Known::reached_by` clears the
            // sites whenever it answers it, so the rule below this loop says
            // the same. The doc comment above says what holds the pair.
            Reached::SetFreed(_) | Reached::Lost(_) | Reached::Partial => return None,
        }
    }

    let (&first, rest) = sites.split_first()?;

    // The allocation, where every site agrees about it, because naming
    // one of several as the one freed is a caret on an allocation the value
    // may not hold.
    let made_of = |site: Site| match known.state[site] {
        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
        SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
    };
    let made = rest
        .iter()
        .fold(made_of(first), |made, &site| same(made, made_of(site)));

    let (conclusion, unproven) = match offset {
        At::Bytes(0) => return None,
        At::Moved | At::Bytes(1..) => (Conclusion::Unsafe, None),
        At::Unknown => (Conclusion::Unknown, Some(Unproven::Offset)),
    };

    Some(Verdict {
        conclusion,
        freed: None,
        made,
        unproven,
    })
}

/// The arguments of a `free` this report asks about, which is every one except
/// a pointer this compiler established is null.
///
/// C17 7.22.3.3 p2: if the argument is a null pointer, no action occurs. So a
/// call whose only argument is such a pointer frees nothing, and there is
/// nothing here for a double free to be about. `Allocations::touching` already
/// applies this to an argument *written* as a constant, with the clause quoted;
/// this is the same rule reaching a local the nullability check proved. See
/// ADR-0027, which is also why nothing weaker exempts one: a pointer nobody
/// established anything about still weakens no proof.
///
/// **Only the report, never the transfer.** This is called from the walk that
/// reports and `Analysis::terminator` cannot reach it, so a free of a pointer
/// established null still marks its sites freed and still leaves the local
/// holding them. Clearing them instead would make the local reach no site,
/// which is how this check spells having lost a pointer, and the reader would
/// get a warning about the wrong thing. That is the third of ADR-0027's three
/// conditions and it is held by where this function is called from.
///
/// **The result being empty is not [`Reached::Lost`]**, and the two are worth
/// keeping apart here because this check has two emptinesses, and an empty
/// may-set means opposite things to its two readers. `verdict` answers `None`
/// for no sites and nothing
/// lost, which is silence, and that is the right answer to a call C says does
/// nothing. What is lost still arrives as a `Reached::Lost` from the arguments
/// that were asked about.
fn asked<'o>(
    arguments: &'o [Operand],
    null: &'o NullAtTerminators,
    block: BlockId,
) -> impl Iterator<Item = &'o Operand> + 'o {
    arguments
        .iter()
        .filter(move |argument| !established_null(argument, null, block))
}

/// Whether this argument is a pointer this compiler established is null where
/// the call runs.
///
/// **A projection answers `false`.** `free(*pp)` asks what a *place* holds, and
/// the nullability lattice is keyed by the local, which its own doc comment
/// says it pays for. Answering anything else here would be reading a claim
/// about `pp` as a claim about what `pp` points at. Mutation: drop the
/// `projection.is_empty()` guard. `a_free_read_out_of_a_pointer_proved_null`
/// loses its `error[SC0401]` and fails, and nothing else in the suite moves.
///
/// A constant answers `false` as well, although `free(0)` is exempt: it is
/// exempt in `Allocations::touching`, where the operand is read, and two rules
/// for one argument is one of them being wrong. **That arm is held by nothing
/// and cannot be**: answering `true` for a constant changes no program,
/// because `touching` has already skipped every one of them before this is
/// asked. It is written out so the arm says which rule owns it.
fn established_null(argument: &Operand, null: &NullAtTerminators, block: BlockId) -> bool {
    match argument {
        Operand::Copy(place) if place.projection.is_empty() => {
            null.established_null(block, place.local)
        }
        Operand::Copy(_) | Operand::Constant(_) => false,
    }
}

/// Report every dereference at `at` of something that was freed.
///
/// **A place this check follows no allocation for says nothing**, which is the
/// opposite of what a free of one says, and the asymmetry is deliberate. A free
/// acts on an allocation, so freeing something the check stopped following may
/// be a second free. A dereference only reads one, and a pointer with no
/// allocation behind it is an uninitialised pointer or one into storage that is
/// not the heap: different defects, with checks of their own that do not exist
/// yet. Answering `Unknown` here would warn on every `*p` whose pointer came
/// from anywhere this does not follow, which is most of them.
///
/// **What that silence covers is a boundary rather than a rule.** A pointer
/// written behind this check's back arrives here as `SiteState::Unknown`
/// rather than as no site at all, because taking a local's address is what
/// makes its sites unknown, and `Known::escaped` is what keeps them that way
/// for the rest of the function: `int **pp = &p; p = malloc(8); *pp = q;` used
/// to hand `p` a fresh site nothing had lost, and reading through it was exit
/// 0 on a freed pointer. Pointer arithmetic is followed for the same reason,
/// and so is a controlling expression, which needed `Terminator::Branch` to
/// carry a span before it could be.
///
/// A place whose value is thrown away is read too, because
/// [`Element::Evaluate`] exists to say that it was evaluated: `*p;` on its own
/// used to leave no element at all, so there was nothing here to look at.
///
/// **The rule, rather than a list of what falls outside it: a place whose root
/// reaches no site says nothing, however it came to reach none.** Two ways are
/// known, and the third was closed by making the lowering apply C17 6.5.3.2
/// p3, so `int *r = &*p;` now copies the pointer rather than taking an address
/// of what it reaches. A pointer read out of memory nothing recorded a store
/// into, `int *p = *pp;` with `pp` a parameter, has no site; one read out of
/// the function's own memory holds what was stored there (ADR-0045). And a bare name is never given an element at all, so
/// `free(p); p;` reads nothing here. A freed value used by a comparison or a
/// branch is [`used_by_value`]'s, a `return` of one and an argument of a call
/// are [`returned`]'s and [`handed`]'s; a plain copy and a value that may
/// have been freed are asked by none of them. See ADR-0084.
/// `docs/diagnostics.md` says what exit 0 does not mean here, because a
/// boundary that lives only in a comment is one no user can find.
///
/// [`Element::Evaluate`]: crate::ir::Element::Evaluate
pub(super) fn used(
    sources: &SourceMap,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    at: Option<(Span, Vec<&Place>)>,
    known: &Known,
    function: FuncId,
) {
    let Some((at, dereferenced)) = at else {
        return;
    };

    for place in dereferenced {
        // **One place at one span said once.** `*p = 42;` lowers to two
        // operations that both read through `p`, because an assignment is an
        // expression with a value and the lowering reads the place back into a
        // temporary. Both are genuine dereferences of one thing and two carets
        // on one line would say it twice.
        //
        // The place and not the finding. Deduplicating finished findings was
        // wrong in both directions at once, measured: two unproven uses on one
        // line carry no spans at all, so they were field-identical and one was
        // thrown away, while `*p = *q;` produced `p`, `q`, `p` in that order
        // and the pair that should have collapsed was not adjacent for
        // `Vec::dedup` to see. What decides whether two reports are one report
        // is which place was dereferenced, and only this knows it.
        //
        // **Each level before the next**, as C reads them: `***t3` reads
        // `t3`'s own allocation, then what was stored in it, then what was
        // stored in that, so a freed level nearer the root is the earlier
        // defect and the one that is said. A deeper level is asked only when
        // every level above it says nothing. Below the first level the order
        // is not visible in what is printed, measured: every such level is
        // unproven and says the same words, so only the first level's place
        // ahead of them is guarded. See ADR-0045.
        //
        // **Only the first level is about the local's own value**, so only
        // it names one: below it the pointer gone through was loaded from
        // memory, and `free(*pp); return **pp;` is about what `pp`'s
        // allocation held rather than about `pp`.
        let Some((verdict, local)) = verdict(
            sources,
            Kind::UseAfterFree,
            known.reached_by(place.local),
            known,
        )
        .map(|verdict| (verdict, Some(place.local)))
        .or_else(|| {
            (1..derefs(place)).find_map(|depth| {
                verdict(
                    sources,
                    Kind::UseAfterFree,
                    known.reached_below(place.local, depth),
                    known,
                )
                .map(|verdict| (verdict, None))
            })
        }) else {
            continue;
        };

        say(
            sources,
            findings,
            said,
            place,
            Finding {
                function,
                kind: Kind::UseAfterFree,
                conclusion: verdict.conclusion,
                at,
                local,
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }
}

/// Report every local at `at` read by value by an operator or a branch,
/// `free(p); return p != 0;`, **where it was certainly freed**: `SC0402`,
/// `Unsafe`. A value that may have been freed says nothing here, a silence
/// `docs/safety-model.md` lists: asked as a doubt, it was asked again at every
/// comparison of every pointer this check stopped following, 233 cases of
/// the corpus, measured. A plain copy is not asked, since the copy carries
/// the value to where it is used. See ADR-0084.
///
/// Through [`say`], keyed with [`used`], so a place both dereferenced and
/// compared at one caret is said once; and not where a use after free
/// already stands at the caret, whatever place it names.
pub(super) fn used_by_value(
    sources: &SourceMap,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    at: Option<(Span, Vec<&Place>)>,
    known: &Known,
    function: FuncId,
    is_pointer: &dyn Fn(LocalId) -> bool,
) {
    let Some((at, operated)) = at else {
        return;
    };
    // **A pointer's value, not a value computed from one**: an operator's
    // result keeps its operands' sites, so `int b = p != 0;` before the free
    // reached the block and `b == 1` after it was said to use a freed value,
    // found by review. An `int` holds no pointer this frontend can make one
    // of, since a cast is refused.
    for place in operated.into_iter().filter(|place| is_pointer(place.local)) {
        // **One use after free at one caret said once, whichever place it
        // names**: `if (p == 0)` compares `p` and then branches on the
        // comparison's temporary, which carries the freed state on, and the
        // two were said twice at one caret, found by review.
        if findings
            .iter()
            .any(|said| said.kind == Kind::UseAfterFree && said.at == at)
        {
            continue;
        }
        let Some(verdict) = verdict(
            sources,
            Kind::UseAfterFree,
            known.reached_by(place.local),
            known,
        ) else {
            continue;
        };
        if verdict.conclusion != Conclusion::Unsafe {
            continue;
        }
        say(
            sources,
            findings,
            said,
            place,
            Finding {
                function,
                kind: Kind::UseAfterFree,
                conclusion: verdict.conclusion,
                at,
                local: Some(place.local),
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }
}

/// Report every access at `at` that this check cannot show fits the object
/// its pointer reaches, once per pointer per caret.
///
/// **Asked of the pointer the place goes through first, and of a deeper one
/// where it may be a local's value.** A deeper level goes through a pointer
/// read out of memory, which is believed to be one whole object of the type it
/// was read as: what makes that belief worth anything is that the store that
/// put the pointer there is asked, by [`departed_in_element`], and that
/// [`fits`] bounds it by what the memory is recorded to hold. A local is not
/// memory with a door: `v = p + 4;` after `pi = &v;` is an assignment, asked
/// nowhere, and `**pi = 1` believed what `v` held. So a deeper level that may
/// go through a local asks that local's value, as it stands at the access, as
/// [`contents_leaving`] asks it where an address leaves; the order of the
/// assignment and the address then does not matter. A pointer read out of a
/// local into another first is asked the same way where it is read, by
/// `Allocations::read_through`. See ADR-0060.
///
/// **Not through [`say`], and not keyed with it.** A use after free and an
/// access outside its object are two questions about one place, and both can
/// be true: keyed together, one would replace the other. See ADR-0060.
pub(super) fn may_not_fit(
    findings: &mut Vec<Finding>,
    asked: &mut Vec<(Span, LocalId, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    at: Option<(Span, Vec<&Place>)>,
    known: &Known,
) {
    let Some((at, places)) = at else {
        return;
    };
    for place in places {
        let local = place.local;
        // **An index is not read**, and is unknown rather than silent: no C
        // reaches one, since the lowering writes `p[i]` as `*(p + i)`, and a
        // frontend that builds one has to be answered for.
        if place
            .projection
            .iter()
            .any(|step| matches!(step, Projection::Index(_)))
        {
            say_fit(
                findings,
                asked,
                analysis,
                (at, local, Some(local)),
                0,
                Some(Fit::Unknown),
            );
            continue;
        }
        // Each level of dereference: the first asked of the place's own local,
        // and every deeper one of each local the pointer it goes through may
        // have been read from.
        for level in 1..=place.projection.len() {
            let through = Place {
                local,
                projection: vec![Projection::Deref; level],
            };
            // A `void` place has no size, and is no access this asks about.
            // Nothing below it is either: the C frontend builds a place of
            // dereferences only, so every level below a `void` one is
            // `void` too, and stopping says nothing a later level would
            // have. A projection that narrows a place, a field, would make
            // a later level sized again, and this would then be silent
            // about it.
            let Some(width) = analysis
                .unit
                .place_ty(function, &through)
                .and_then(|ty| analysis.unit.size_of(ty))
            else {
                break;
            };
            if level == 1 {
                // An access through the local itself, which doubts what may
                // have written it unseen.
                let fit = fits(analysis, function, local, width, known, true);
                say_fit(
                    findings,
                    asked,
                    analysis,
                    (at, local, Some(local)),
                    width,
                    fit,
                );
                continue;
            }
            // **The memory it goes through**, no wider than that memory is
            // believed to hold ([`Allocations::believed_read`]): `**pi = 1`
            // with `pi` a `void **` parameter converted to `int **` wrote a
            // whole `int` through a pointer nobody was asked the width of
            // (#632). It names no local, since the pointer was read out of
            // memory, and it is keyed by the place's own local, so it says
            // nothing where that local's answer already stands at this caret.
            let above = Place {
                local,
                projection: vec![Projection::Deref; level - 1],
            };
            if analysis.believed_read(function, &above, known) < width {
                say_fit(
                    findings,
                    asked,
                    analysis,
                    (at, local, None),
                    width,
                    Some(Fit::Unknown),
                );
            }
            // And each local the pointer may have been read from, which its
            // address has escaped, so the escape is not doubted: the writes
            // it stands for are marked and asked where they happened
            // (ADR-0060). Lost and foreign stand for the same writes and are
            // not doubted either, as [`fits`] says.
            let (locals, certain) = read_from_locals(function, known, local, level - 1);
            for pointer in locals {
                let fit = fits(analysis, function, pointer, width, known, false)
                    .map(|fit| only_if_certain(fit, certain));
                say_fit(
                    findings,
                    asked,
                    analysis,
                    (at, pointer, Some(pointer)),
                    width,
                    fit,
                );
            }
        }
    }
}

/// Push what [`fits`] answered about an access through `local`'s value, once
/// per local per caret, and the proof where one level proves what another
/// only doubted. `at` holds three things: the caret; the local the answer
/// is keyed by, so that one local is said once there; and the local the
/// words name, which is `None` for memory a deeper level goes through and
/// otherwise the same local.
///
/// **The worse answer stands, whichever level is asked first.** One pointer
/// can be asked twice at one caret, through itself and through its address:
/// `*v = **pi;` with `pi = &v;` asks `v` at both, and the first doubts its
/// escape while the second does not. Keeping the first answer let the order
/// the places were listed in decide whether a proof was said. Found by
/// review.
fn say_fit(
    findings: &mut Vec<Finding>,
    asked: &mut Vec<(Span, LocalId, usize)>,
    analysis: &Allocations<'_>,
    at: (Span, LocalId, Option<LocalId>),
    width: u64,
    fit: Option<Fit>,
) {
    let (at, local, named) = at;
    let (conclusion, reach, extent, made, unproven) = match fit {
        None | Some(Fit::Inside) => return,
        Some(Fit::Outside {
            reach,
            extent,
            made,
        }) => (Conclusion::Unsafe, reach, extent, made, None),
        Some(Fit::Unknown) => (
            Conclusion::Unknown,
            width,
            None,
            None,
            Some(Unproven::Extent),
        ),
    };
    let finding = Finding {
        function: analysis.function,
        kind: Kind::OutsideItsObject {
            width: reach,
            extent,
        },
        conclusion,
        at,
        local: named,
        freed: None,
        made,
        unproven,
    };
    let standing = asked
        .iter()
        .find(|(asked_at, asked_local, _)| *asked_at == at && *asked_local == local);
    match standing {
        Some(&(_, _, index)) => {
            if conclusion == Conclusion::Unsafe && findings[index].conclusion != Conclusion::Unsafe
            {
                findings[index] = finding;
            }
        }
        None => {
            asked.push((at, local, findings.len()));
            findings.push(finding);
        }
    }
}

/// Report every library call at this terminator that reads or writes through
/// a pointer it is handed further than this check can show fits.
///
/// **What the call is asked to touch is the access**, as [`LIBRARY`](crate::library::LIBRARY) says per
/// argument: a [`Reach::Length`] is an access of as many bytes as that
/// argument holds, read as the parameter receives it, and any length this
/// check cannot read is unknown; a [`Reach::String`] runs as far as a string
/// does, which nothing here reads, so it is unknown wherever the pointer
/// holds anything. A null holds nothing and is the nullability check's.
/// See ADR-0060.
pub(super) fn reached_by_library(
    findings: &mut Vec<Finding>,
    touched: &mut Vec<(Span, LocalId, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    terminator: &Terminator,
    known: &Known,
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
    let Some(row) = analysis.library(*callee) else {
        return;
    };
    let at = origin.span();
    for &(index, reach) in row.reaches {
        let Some(Operand::Copy(place)) = arguments.get(index) else {
            continue;
        };
        let local = place.local;
        let width = match reach {
            Reach::Length(length) => analysis.constant_argument(*callee, arguments, length),
            Reach::String => None,
        };
        // The call reaches through the local itself, which doubts what may
        // have written it unseen, as an access does.
        let fit = if place.projection.is_empty() {
            match width {
                Some(width) => fits(analysis, function, local, width, known, true),
                // Anything that holds something is unknown here.
                None => fits(analysis, function, local, 0, known, true).map(|_| Fit::Unknown),
            }
        } else {
            // **A pointer read out of memory and handed straight over** is
            // believed as `Allocations::believed_read` says, as one assigned to a local
            // first is, unless it may have been read from a local, which is
            // asked instead ([`through_locals`]), and alone where it
            // certainly was.
            let believed = analysis.believed_read(function, place, known);
            match width {
                Some(width) => {
                    let belief = if believed >= width {
                        Fit::Inside
                    } else {
                        Fit::Unknown
                    };
                    Some(
                        match through_locals(analysis, function, place, width, known) {
                            Some((fit, Some(_))) => fit,
                            Some((Fit::Inside, None)) | None => belief,
                            Some((fit, None)) => fit,
                        },
                    )
                }
                None => Some(Fit::Unknown),
            }
        };
        let (conclusion, reach, extent, made, unproven) = match fit {
            None | Some(Fit::Inside) => continue,
            Some(Fit::Outside {
                reach,
                extent,
                made,
            }) => (Conclusion::Unsafe, Some(reach), extent, made, None),
            Some(Fit::Unknown) => (
                Conclusion::Unknown,
                width,
                None,
                None,
                Some(Unproven::Extent),
            ),
        };
        let finding = Finding {
            function: analysis.function,
            // No width where none was read: it is printed only beside a
            // proof, and a proof needs one.
            kind: Kind::OutsideItsObject {
                width: reach.unwrap_or(0),
                extent,
            },
            conclusion,
            at,
            local: place.projection.is_empty().then_some(local),
            freed: None,
            made,
            unproven,
        };
        // **One report per pointer per call, and a proof stands over a
        // doubt**, as [`say_fit`] keeps it: `memcpy(*pp, pp, 16)` asks `pp`
        // twice, and the doubt about `*pp` asked first stood where the
        // proof that the source runs past `pp`'s eight bytes should have,
        // so `--allow-unknown` built the overflow. Found by review of #735,
        // where a load believing nothing made that doubt the common case.
        let standing = touched
            .iter()
            .find(|(touched_at, touched_local, _)| *touched_at == at && *touched_local == local);
        match standing {
            Some(&(_, _, index)) => {
                if conclusion == Conclusion::Unsafe
                    && findings[index].conclusion != Conclusion::Unsafe
                {
                    findings[index] = finding;
                }
            }
            None => {
                touched.push((at, local, findings.len()));
                findings.push(finding);
            }
        }
    }
}

/// Whether a value leaving as a pointer to `width` bytes points at the
/// start of a whole object that wide, and the local it is the value of, or
/// `None` where it is a constant and so a null, which leaves nothing to ask.
///
/// **Each shape a value can arrive in, written out**, so that a kind of
/// rvalue added later answers here: a local's own value is asked as
/// [`fits`] asks an access; a pointer read out of memory and handed straight
/// on is believed by the type it is read as, as a load assigned first is; a
/// local's address is that local's size; and anything built here without a
/// temporary, which the C frontend does not do, is unknown. See ADR-0060.
fn leaving(
    analysis: &Allocations<'_>,
    function: &Function,
    value: &Rvalue,
    width: u64,
    known: &Known,
) -> Option<(Fit, Option<LocalId>)> {
    match value {
        Rvalue::Use(Operand::Constant(_)) => None,
        // A value leaving doubts nothing unseen, as [`fits`] says. Off its
        // start, unknown: see [`departing_fit`].
        Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => {
            departing_fit(analysis, function, place.local, width, known)
                .map(|fit| (fit, Some(place.local)))
        }
        // A belief narrower than the width is unknown and not narrower,
        // since a belief is a floor and the object may be wider.
        // Unless it may have been read from a local, which is asked instead
        // ([`through_locals`]), and alone where it certainly was.
        Rvalue::Use(Operand::Copy(place)) if derefs(place) > 0 => {
            let believed = analysis.believed_read(function, place, known);
            let belief = if believed >= width {
                Fit::Inside
            } else {
                Fit::Unknown
            };
            Some(
                match through_locals(analysis, function, place, width, known) {
                    Some((fit, Some(local))) => (fit, Some(local)),
                    Some((Fit::Inside, None)) | None => (belief, None),
                    Some((fit, None)) => (fit, None),
                },
            )
        }
        Rvalue::Use(Operand::Copy(_)) => Some((Fit::Unknown, None)),
        Rvalue::Address(place) if place.projection.is_empty() => {
            let fit = match analysis.unit.size_of(function.local(place.local)) {
                Some(size) if size >= width => Fit::Inside,
                Some(size) => Fit::Outside {
                    reach: width,
                    extent: Some(size),
                    made: None,
                },
                None => Fit::Unknown,
            };
            Some((fit, None))
        }
        Rvalue::Address(_) | Rvalue::Unary { .. } | Rvalue::Binary { .. } => {
            Some((Fit::Unknown, None))
        }
    }
}

/// [`fits`] for a pointer that leaves the function, which doubts nothing
/// unseen, **and is unknown wherever it points off its start**, by a
/// distance counted or not, as it was before distances were counted.
///
/// **A departure is asked whether the receiver may take a whole object of
/// its type there**, and for a `void *` whether it starts one (ADR-0065);
/// both are worded about the start, and a pointer some distance in was
/// always a doubt. Counting the distance (#703) is for an access, which
/// says how far it reaches: answered here by it, `return p + 5;` out of
/// `malloc(4)` was a proof worded as `a pointer to 4 bytes` where `4 bytes
/// are believed`, which the reader cannot reconcile, and a `void *` some
/// way into an object was answered whether a byte fits there rather than
/// whether it starts it.
///
/// Mutation: answer `off_start` `false`;
/// `a_void_pointer_off_its_start_leaving_is_asked_whether_it_starts_an_object`
/// fails, among many.
fn departing_fit(
    analysis: &Allocations<'_>,
    function: &Function,
    local: LocalId,
    width: u64,
    known: &Known,
) -> Option<Fit> {
    let off_start = matches!(known.points_to[local].offset, At::Moved | At::Bytes(1..));
    fits(analysis, function, local, width, known, false)
        .map(|fit| if off_start { Fit::Unknown } else { fit })
}

/// Push what [`leaving`] answered, once per value per caret and door.
fn say_leaving(
    findings: &mut Vec<Finding>,
    departed: &mut Vec<(Span, Departure, Option<LocalId>, usize)>,
    analysis: &Allocations<'_>,
    left: (Span, Departure, u64),
    answer: Option<(Fit, Option<LocalId>)>,
    below: bool,
) {
    let (at, how, width) = left;
    let Some((fit, local)) = answer else {
        return;
    };
    // **Unknown either way**, for the reason `Kind::HandedOnNarrow` gives;
    // what was shown is carried into the words.
    let (narrower, extent, made) = match fit {
        Fit::Inside => return,
        Fit::Outside { extent, made, .. } => (true, extent, made),
        Fit::Unknown => (false, None, None),
    };
    let finding = Finding {
        function: analysis.function,
        kind: Kind::HandedOnNarrow {
            how,
            width,
            narrower,
            extent,
            below,
        },
        conclusion: Conclusion::Unknown,
        at,
        local,
        freed: None,
        made,
        unproven: Some(Unproven::Extent),
    };
    // Once per local per caret and door: a second argument of another width
    // at the same call says the same thing about the same pointer, and every
    // one is unknown, so which width is named changes nothing a reader can
    // act on. **What was shown narrower stands over the doubt**, whichever
    // was asked first: one local is asked once per type it is read as, and
    // the order the walk reaches them in chose the words. Found by review.
    if local.is_some() {
        let standing = departed
            .iter()
            .find(|(said_at, said_how, said_local, _)| {
                *said_at == at && *said_how == how && *said_local == local
            })
            .map(|&(.., index)| index);
        if let Some(index) = standing {
            let shown = matches!(
                findings[index].kind,
                Kind::HandedOnNarrow { narrower: true, .. }
            );
            // **And a doubt about the pointer's own level stands over one
            // about the memory below it**, whichever was asked first, since
            // it is the one to fix first (#636).
            let standing_below = matches!(
                findings[index].kind,
                Kind::HandedOnNarrow { below: true, .. }
            );
            if (narrower && !shown) || (standing_below && !below && !shown) {
                findings[index] = finding;
            }
            return;
        }
    // A value with no local, a load, a constant's address or what memory
    // holds, has no identity to key on, and is said unless the same words
    // already stand at this caret: two objects in memory both too narrow,
    // or a local and the memory beside it, printed one report twice. Found
    // by review.
    } else if findings
        .iter()
        .any(|said| said.at == at && said.kind == finding.kind)
    {
        return;
    }
    // **A store departure stands over the deferred ask of a partial copy**
    // at the same caret, which says the same copy in words that know no
    // type: `memcpy(q, &i, 4)` with `q` an `int **` printed both, found by
    // review. In place, so the indices `departed` holds stay right. See
    // `push_part_of_a_pointer`.
    let deferred = if how == Departure::Store {
        findings
            .iter()
            .position(|said| is_part_of_a_pointer_written(said, at))
    } else {
        None
    };
    if let Some(index) = deferred {
        departed.push((at, how, local, index));
        findings[index] = finding;
        return;
    }
    departed.push((at, how, local, findings.len()));
    findings.push(finding);
}

/// Ask every pointer a value leaving as a `ty` lets leave with it: what each
/// local it may point at holds, believed by the receiver as the type `ty`
/// points to, and so on down while that is a pointer too.
///
/// **Handing on a pointer to memory hands on the pointers stored there.**
/// `int *q = v; h(&q);` with `v` a `char`'s address lets `h` read `*pp` and
/// believe it a whole `int`, and nothing asked `q`, since an assignment to a
/// local is no door. Asking the local's own value where its address leaves
/// is the door, with the state at that point, so an assignment after the
/// address was taken is seen too. Found by review.
///
/// **Each local at the wider of its own type and the receiver's.** Whoever
/// converts a `void *` back and reads the local reads a pointer of the
/// local's type, so a `void *` leaving still asks each local it reaches at
/// that type, and a `void *` local its start; and a receiver that reads the
/// local as a wider type believes that: with `void *v = &c; void *pv =
/// &v;` and `use` taking `int **`, `use(pv)` asks `v` four bytes wide
/// (#627).
///
/// **Memory by its record, where the receiver believes a width.** Its
/// stores were asked at the type they stored, and the receiver may read a
/// wider one: `*m = c; use(pm);` with `m` a `void **` and `c` one `char` is
/// asked what `m` holds, the objects the pointers stored there point at
/// (a cell's `names`) by their size. What the
/// record does not hold is not asked by its record, since asking it would
/// doubt every pointer to a parameter's memory handed on (#627); it is
/// asked by what it is believed to hold instead, from the type the
/// departing value arrived as and from what was stored there
/// ([`Held::contents_at_most`](super::parts::Held::contents_at_most),
/// a cell's `stored_levels`, #632). See ADR-0060.
fn contents_leaving(
    findings: &mut Vec<Finding>,
    departed: &mut Vec<(Span, Departure, Option<LocalId>, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    left: (Span, Departure, TyId),
    value: &Rvalue,
    known: &Known,
) {
    let (at, how, ty) = left;
    // **Only a pointer lets anything leave with it.** An integer can hold
    // edges, since a call this check cannot read may return any escaped
    // local's address and its result is followed whatever its type, and
    // asking through one doubted `return use2(&a);` about `a`.
    let Some(read_as) = pointee(analysis, ty) else {
        return;
    };
    // **What the departing value's own memory is believed to hold**, from
    // the type it arrived as, is asked against what the receiver reads
    // there: in `void run(void **m) { void *pm = m; use(pm); }` with `use`
    // taking `int **`, the memory handed on holds pointers nobody was asked
    // the width of (#632). A pointer read out of memory and handed straight
    // on arrives as the type it is read as. A local's address and an
    // allocation are asked by the local and the record below. See
    // `Held::contents_at_most`.
    //
    // **At every level the receiver's type has**: `use(pm)` with `pm` a
    // `void ***` converted whole and `use` taking `int ***` hands on memory
    // whose pointers point at pointers nobody was asked the width of, which
    // asking only the first level let leave (#634). The width said is the
    // receiver's at the first level, as `fits` says one for an access.
    //
    // Mutation: ask level 0 alone, and
    // `a_void_pointer_three_levels_deep_handed_on_whole_is_asked_below_its_first_level`
    // goes silent.
    let arrived = match value {
        Rvalue::Use(Operand::Copy(place)) => analysis.contents_read(function, place, known),
        Rvalue::Use(_) | Rvalue::Address(_) | Rvalue::Unary { .. } | Rvalue::Binary { .. } => {
            Levels::default()
        }
    };
    if let (Some(receiver), Some(width)) = (
        analysis.type_levels(ty),
        stored_pointer(analysis.unit, read_as).and_then(|stored| pointee_width(analysis, stored)),
    ) {
        if arrived.narrower_than(&receiver) {
            // **Named by the local that leaves, where one does**, so that
            // this and a doubt about the local's own level at the same
            // call are one report, `use2(v)` after `put_any(&v)`, rather
            // than two (#636).
            let leaving = match value {
                Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => {
                    Some(place.local)
                }
                Rvalue::Use(_)
                | Rvalue::Address(_)
                | Rvalue::Unary { .. }
                | Rvalue::Binary { .. } => None,
            };
            say_leaving(
                findings,
                departed,
                analysis,
                (at, how, width),
                Some((Fit::Unknown, leaving)),
                true,
            );
        }
    }
    let (locals, sites, _) = slots_leaving(analysis, function, read_as, value, known);
    for (local, read_as) in locals {
        // A local that holds no pointer, read as none, has nothing a reader
        // believes of it; a `void *` is asked whether it starts an object.
        let own = pointee_width(analysis, function.local(local));
        let receiver = read_as
            .and_then(|ty| stored_pointer(analysis.unit, ty))
            .and_then(|ty| pointee_width(analysis, ty));
        let Some(width) = own.max(receiver) else {
            continue;
        };
        // Reached through its address, so its escape is not doubted, as
        // [`fits`] says. Off its start, unknown: see [`departing_fit`].
        let fit = departing_fit(analysis, function, local, width, known);
        // **And below its own level, by what its value believes of its
        // memory**, against what the receiver reads there: `use(&x)` with
        // `x` a `void **` converted to `int **` and `use` taking `int ***`
        // hands on memory whose pointers nobody was asked the width of,
        // which the local's own fit cannot see (#634). Narrower is unknown,
        // and a fit shown outside stays the answer, since it says more.
        let narrower = read_as
            .and_then(|ty| analysis.type_levels(ty))
            .is_some_and(|receiver| {
                known.points_to[local]
                    .contents_at_most()
                    .narrower_than(&receiver)
            });
        let fit = match fit {
            Some(Fit::Outside { .. }) => fit,
            _ if narrower => Some(Fit::Unknown),
            _ => fit,
        };
        let answer = fit.map(|fit| (fit, Some(local)));
        say_leaving(
            findings,
            departed,
            analysis,
            (at, how, width),
            answer,
            false,
        );
    }
    for (site, read_as) in sites {
        // A `void *` receiver believes only a start, which the store that put
        // each pointer here asked; and memory recorded to hold nothing says
        // nothing.
        let Some(width) = stored_pointer(analysis.unit, read_as)
            .and_then(|ty| pointee_width(analysis, ty))
            .filter(|&width| width > 0)
        else {
            continue;
        };
        let extents: Vec<Extent> = known.contents[site]
            .sites()
            .map(|held| known.extent[held].unwrap_or(Extent::AtLeast(0)))
            .chain(
                function
                    .locals()
                    .filter(|local| known.contents[site].names_local(LocalIdx::of(local)))
                    .map(|held| {
                        analysis
                            .unit
                            .size_of(function.local(held))
                            .map_or(Extent::AtLeast(0), Extent::Exactly)
                    }),
            )
            .collect();
        // And what was stored there that the record does not name, by what it
        // was believed (a cell's `stored_levels`, #632).
        // A narrower stored bound is unknown even beside a record that
        // proves: the record names the objects the pointers it holds point
        // at, and the bound stands for a pointer it holds that names none,
        // so a whole object on one path proves nothing of the other. With
        // nothing recorded and no bound there is nothing to ask.
        //
        // **And at every level below**, which is what each pointer stored
        // there believed of its own memory, whatever the record names: `*box
        // = m; use(box);` with `m` a `void **` parameter and `use` taking
        // `int ***` (#634).
        let receiver = analysis
            .type_levels(read_as)
            .unwrap_or_default()
            .stored_under(Width::AtMost(width), analysis.deepest_pointer);
        // **And where a store there recorded no width**, which no other
        // record may stand in for where the receiver reads a pointer out of
        // it: `*ib = 7; use(box);` with `use` taking `int **` handed on an
        // `int` read as a pointer, in silence, and so did the same on one
        // arm beside `*box = malloc(4);` on the other (#767). Such a store
        // is `Nothing` at the cell's level 0, which `Levels::narrower_than`
        // reads as narrower than any width the receiver reads there.
        let stored_narrower = known.contents[site]
            .stored_levels()
            .narrower_than(&receiver);
        let fit = match (extents.is_empty(), stored_narrower) {
            (true, false) => continue,
            (true, true) => Fit::Unknown,
            (false, true) => match folded(&extents, width) {
                Fit::Inside => Fit::Unknown,
                fit => fit,
            },
            (false, false) => folded(&extents, width),
        };
        say_leaving(
            findings,
            departed,
            analysis,
            (at, how, width),
            Some((fit, None)),
            false,
        );
    }
}

/// The width a pointer stored by this library copy is believed to point at,
/// where the copy writes only part of one: `None` where the operand typed
/// `ty` holds no pointer by its type, or the copy writes whole ones. See
/// `CopiedPointers`.
fn part_of_a_pointer(
    analysis: &Allocations<'_>,
    callee: FuncId,
    arguments: &[Operand],
    ty: Option<TyId>,
) -> Option<u64> {
    let width = pointee_width(
        analysis,
        stored_pointer(analysis.unit, pointee(analysis, ty?)?)?,
    )?;
    match analysis.copied_pointers(callee, arguments) {
        CopiedPointers::Whole => None,
        CopiedPointers::LastInPart | CopiedPointers::NoneWhole => Some(width),
    }
}

/// [`part_of_a_pointer`] asked of what each operand points at rather than of
/// its type: the width a pointer held there is believed to point at, where
/// the copy writes only part of one. Reached only where both typed asks
/// answered nothing, and asked whatever the operand's type, since a `void *`
/// says nothing and a `char *` made from one, `void *vv = &v; char *s = vv;`,
/// says bytes, and both built silent where the same copy through an `int **`
/// is asked (#746). See ADR-0071 rule 4.
///
/// The destination first, then the source, as the typed asks are, each by
/// [`points_at_a_pointer`], and the first answer is the report: the width is
/// all it carries. A copy of an `int`'s bytes into a local that holds no
/// pointer stays silent however its operands are typed.
fn part_of_what_operands_point_at(
    analysis: &Allocations<'_>,
    function: &Function,
    callee: FuncId,
    arguments: &[Operand],
    known: &Known,
) -> Option<u64> {
    match analysis.copied_pointers(callee, arguments) {
        CopiedPointers::Whole => return None,
        CopiedPointers::LastInPart | CopiedPointers::NoneWhole => {}
    }
    arguments
        .iter()
        .take(2)
        .find_map(|operand| points_at_a_pointer(analysis, function, operand, known))
}

/// Report a pointer this element hands back by a `return`, stores through
/// a pointer, or writes by name into a local code this check cannot read
/// may read (#690), where this check cannot show it points at one whole
/// object of the type it leaves as. And a `return` of a block nothing wrote,
/// `unwritten::report_returned_unwritten`.
///
/// **The departure half of ADR-0060.** A caller believes what it is handed
/// back, and a later load believes what it reads, so this is where either
/// belief is asked. A `void *` is asked only whether it starts an object, and
/// a constant is a null, which the nullability check answers.
#[allow(clippy::too_many_arguments)]
pub(super) fn departed_in_element(
    findings: &mut Vec<Finding>,
    departed: &mut Vec<(Span, Departure, Option<LocalId>, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    element: &Element,
    known: &Known,
) {
    let Element::Assign(operation) = element else {
        return;
    };
    let Some(store) = analysis.store_in(element, known) else {
        return;
    };
    // Whether the block it returns was written is asked by
    // [`asked_unwritten_in_element`], which both report walks call (#838).
    let how = if store.into == Place::local(function.return_place()) {
        Departure::Return
    } else {
        match store.kind {
            StoreKind::Through { .. } if store.leaves => Departure::Store,
            // **Into memory nothing outside this function reads, only
            // whether the pointer is off its start.** What it points at and
            // how wide is asked by the load that reads it back (ADR-0061) and
            // by the door the memory leaves by, as a write into a local is;
            // where it starts is not, since no distance is carried
            // (ADR-0036), and `*box = p + 5;` read by nothing was reported
            // only here. See ADR-0065.
            StoreKind::Through { .. } => {
                if let Some(width) = analysis
                    .unit
                    .place_ty(function, &operation.place)
                    .and_then(|ty| pointee_width(analysis, ty))
                {
                    say_leaving(
                        findings,
                        departed,
                        analysis,
                        (operation.origin.span(), Departure::Store, width),
                        at_its_start(analysis, function, &operation.value, known),
                        false,
                    );
                }
                return;
            }
            // A write to a local by name is asked here where code this check
            // cannot read may read the local, which is what `leaves` says of
            // a direct write: `keep(&slot); slot = v;` is a store a later
            // read through the kept address believes, as `*ps = v;` is. Any
            // other is asked where the local's value or its address leaves
            // (#690). See ADR-0060.
            StoreKind::Direct if store.leaves => Departure::Store,
            StoreKind::Direct => return,
            // `store_in` makes no copy; listed so that a kind added is
            // answered here.
            StoreKind::Copy => return,
        }
    };
    let Some(ty) = analysis.unit.place_ty(function, &operation.place) else {
        return;
    };
    let at = operation.origin.span();
    // What the value points to first, then the value, whatever it leaves as.
    contents_leaving(
        findings,
        departed,
        analysis,
        function,
        (at, how, ty),
        &operation.value,
        known,
    );
    let Some(width) = pointee_width(analysis, ty) else {
        return;
    };
    let answer = leaving(analysis, function, &operation.value, width, known);
    say_leaving(
        findings,
        departed,
        analysis,
        (at, how, width),
        answer,
        false,
    );
}

/// What a store into memory nothing outside reads asks of the pointer it
/// stores: whether it starts an object, as [`leaving`] answers for one byte,
/// so that where it starts is asked and how wide it is is not. A pointer off
/// its start, or one this check cannot say starts one, is `Unknown`; one
/// read out of memory is believed as its type, as `fits` believes it. See
/// ADR-0065.
fn at_its_start(
    analysis: &Allocations<'_>,
    function: &Function,
    value: &Rvalue,
    known: &Known,
) -> Option<(Fit, Option<LocalId>)> {
    leaving(analysis, function, value, 1, known).map(|(fit, local)| match fit {
        // One byte outside is no object at all, which is a width and
        // not a start; asked by the load and the door instead.
        Fit::Outside { .. } => (Fit::Inside, local),
        Fit::Inside | Fit::Unknown => (fit, local),
    })
}

/// Report a pointer this call is handed where this check cannot show it
/// points at one whole object of the type the callee believes it is.
///
/// **What the callee believes is its parameter's type**, so the callee's
/// parameters are walked as well as the call's arguments: `void g();` names
/// none, and a call through it passing fewer arguments than the definition
/// takes would ask nothing about the rest while the body believed them. A
/// pointer parameter no argument fills is unknown, and an argument past every
/// parameter is asked as its own type, which is what a variadic or an
/// unprototyped callee receives.
///
/// **Which callees ask, by what `Callee` says of them**, unless the callee
/// is defined in this unit, which is asked as any function whatever its
/// name. An opaque call asks every argument, but one its `LIBRARY` row
/// reaches, which [`reached_by_library`] asks instead. `memcpy` and `memmove`
/// ask what their source points at, as a store into the destination. `free`,
/// an allocator, `realloc`, a call that does not return, and a library
/// function that returns its first argument ask nothing here: the first four
/// read nothing they are handed as an object, and the last is asked how far
/// it reaches. See ADR-0060.
pub(super) fn departed_at_call(
    findings: &mut Vec<Finding>,
    departed: &mut Vec<(Span, Departure, Option<LocalId>, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    terminator: &Terminator,
    known: &Known,
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
    let at = origin.span();
    let declared = analysis.unit.function(*callee);
    // **A body here is what runs, whatever it is called**, so a definition of
    // `memcmp` taking `int *` is asked as any function is: `callee` answers
    // `Callee::Opaque` for every body this unit defines (ADR-0071).
    match analysis.callee(*callee) {
        Callee::Opaque => {}
        // **`memcpy` and `memmove` store what they copy**, so the pointers
        // their source reaches leave into the destination as a store would
        // put them there. Found by review.
        Callee::Copies => {
            // **What the source points at, whatever the destination is**,
            // and so not through `store_at`, which makes a `Store` only where
            // both are places: what this asks is how wide what the source
            // points at is, and `memcpy(0, b, 8)`, undefined for its
            // destination, asked nothing and built where main refused it.
            // Found by review. The destination is read for its type alone,
            // below, where it is a place.
            //
            // **Its width only where it may leave**, as the store door asks
            // it (`departed_in_element`): a copy into memory nothing outside
            // this function reads is a store there, whose width a load that
            // reads it back asks. Asked regardless, `memcpy(b, &v, 8)` into
            // this function's own block said a pointer to one byte left
            // where `*b = v;` said nothing (the memory model rebuilt on #924,
            // stage 2). Part of a pointer is still asked here, below, as
            // ADR-0073 rule 4 asks it; where no `Store` is made, as above,
            // the width is asked too.
            let stays = analysis
                .store_at(terminator, known)
                .is_some_and(|store| !store.leaves);
            let source_ty = match arguments.get(1) {
                Some(Operand::Copy(place)) => analysis.unit.place_ty(function, place),
                Some(Operand::Constant(_)) | None => None,
            };
            if let (Some(source), Some(source_ty), false) = (arguments.get(1), source_ty, stays) {
                contents_leaving(
                    findings,
                    departed,
                    analysis,
                    function,
                    (at, Departure::Store, source_ty),
                    &Rvalue::Use(source.clone()),
                    known,
                );
            }
            // **And at the destination's type**, which is what a later read
            // of the destination believes: `memcpy(out, &v, 8)` with `v` a
            // `void *` and `out` an `int **` let a one-byte allocation leave
            // as a whole `int`'s, where `*out = v;` is asked at `int *`
            // (#713). After the source's ask, which stays, so a destination
            // narrower than the source asks no less than before. A `void **`
            // destination asks a `void *`'s start, as the source's type
            // does. See ADR-0060.
            let into_ty = match arguments.first() {
                Some(Operand::Copy(place)) => analysis.unit.place_ty(function, place),
                Some(Operand::Constant(_)) | None => None,
            };
            if let (Some(source), Some(into_ty), false) = (arguments.get(1), into_ty, stays) {
                contents_leaving(
                    findings,
                    departed,
                    analysis,
                    function,
                    (at, Departure::Store, into_ty),
                    &Rvalue::Use(source.clone()),
                    known,
                );
            }
            // **And whether it stores a pointer whole**: `memcpy(out, &v, 4)`
            // with `out` an `int **` writes four of a pointer's eight bytes,
            // and a reader of `*out` reads a pointer nobody wrote, which was
            // asked nothing (#717). Unknown, as every departure is. **By the
            // source's type too**, as the asks above are: `void *d = out;
            // memcpy(d, &v, 4);` hides the destination's and not the
            // source's, and was asked nothing. Found by review. **And by what
            // the operands point at**, where neither type says: `void *d =
            // out; void *s = &v; memcpy(d, s, 4);` hid both (#746). See
            // ADR-0071 rule 4.
            if let Some(width) = part_of_a_pointer(analysis, *callee, arguments, into_ty)
                .or_else(|| part_of_a_pointer(analysis, *callee, arguments, source_ty))
                .or_else(|| {
                    part_of_what_operands_point_at(analysis, function, *callee, arguments, known)
                })
            {
                say_leaving(
                    findings,
                    departed,
                    analysis,
                    (at, Departure::Store, width),
                    Some((Fit::Unknown, None)),
                    false,
                );
            }
            // Whether what it copies was written is asked by
            // [`asked_unwritten_in_terminator`], which both report walks
            // call (#838).
            return;
        }
        // `free`, an allocator, `realloc` and a call that does not return
        // read nothing they are handed as an object, and a library function
        // that returns its first argument is asked how far it reaches by
        // `reached_by_library` instead.
        Callee::Frees
        | Callee::Allocates
        | Callee::Reallocates
        | Callee::ReturnsFirst
        | Callee::Ends => return,
    }
    let reached: Vec<usize> = analysis
        .library(*callee)
        .map(|row| row.reaches.iter().map(|&(index, _)| index).collect())
        .unwrap_or_default();
    let parameters: Vec<LocalId> = declared.parameters().collect();
    for index in 0..parameters.len().max(arguments.len()) {
        if reached.contains(&index) {
            continue;
        }
        let ty = match (parameters.get(index), arguments.get(index)) {
            (Some(&parameter), _) => declared.local(parameter),
            (None, Some(Operand::Copy(place))) => {
                let Some(ty) = analysis.unit.place_ty(function, place) else {
                    continue;
                };
                ty
            }
            (None, Some(Operand::Constant(_)) | None) => continue,
        };
        if let Some(argument) = arguments.get(index) {
            contents_leaving(
                findings,
                departed,
                analysis,
                function,
                (at, Departure::Argument, ty),
                &Rvalue::Use(argument.clone()),
                known,
            );
        }
        // **A block handed to a parameter that holds pointers is asked
        // whether a copy wrote part of one into it**, at the copy: the
        // callee reads pointers out of it (#832). See `Known::part_written`
        // and ADR-0087. **Or one handed as a pointer whose own record says
        // it was held as pointers**, as a `return` asks it (ADR-0086): `g(m)`
        // with `g` taking a `void *` hands on what `void **h = m;` held as
        // pointers, and was silent where `return m;` was reported (#914,
        // ADR-0092 rule 6).
        let held_as_pointers = |place: &Place| {
            says_nothing_of_its_pointee(analysis, ty)
                && place.projection.is_empty()
                && known
                    .sites_of(place.local)
                    .any(|site| known.held_levels[site] > 0)
        };
        let handed = match arguments.get(index) {
            Some(Operand::Copy(place))
                if analysis.first_level_holds_a_pointer(ty) || held_as_pointers(place) =>
            {
                Some(place)
            }
            Some(Operand::Copy(_) | Operand::Constant(_)) | None => None,
        };
        if let Some(place) = handed {
            // **What it names, and as far below as the parameter reads
            // pointers or a block's record says**: `g(&qq)` with `g` taking a
            // `void ***` hands on the block `qq` holds, found by review.
            //
            // **What the argument's value names, and as far below as the
            // parameter reads pointers**, by the walk the exit uses: `g(b)`
            // with `g` taking a `void ***` hands on the block below `b`, and
            // `g(&qq)` the block `qq` holds, found by review. The blocks one
            // level below the value are walked with one level less, since a
            // `void **` parameter reads the pointer in `b`'s block and not
            // what that pointer points at: asking them at full levels doubted
            // `*b = m; g(b);` for a partial copy into `m`. A place of `d`
            // dereferences has the value `levels_below(local, d)` names, as
            // the `return` reads it.
            let levels = analysis.levels_holding_pointers(ty);
            let (value, below): (Vec<Site>, Vec<Site>) =
                match (place.projection.is_empty(), derefs(place)) {
                    (true, _) => (
                        known.sites_of(place.local).collect(),
                        known.levels_below(place.local, 1).0.into_iter().collect(),
                    ),
                    (false, 0) => (Vec::new(), Vec::new()),
                    (false, depth) => (
                        known
                            .levels_below(place.local, depth)
                            .0
                            .into_iter()
                            .collect(),
                        known
                            .levels_below(place.local, depth + 1)
                            .0
                            .into_iter()
                            .collect(),
                    ),
                };
            let mut named = known.held_below(value, levels);
            named.extend(known.held_below(below, levels.saturating_sub(1)));
            ask_partial(
                PartialDoor::Call,
                named.into_iter().map(Loc::Heap),
                findings,
                analysis,
                known,
            );
        }
        let Some(width) = pointee_width(analysis, ty) else {
            continue;
        };
        let answer = match arguments.get(index) {
            Some(argument) => leaving(
                analysis,
                function,
                &Rvalue::Use(argument.clone()),
                width,
                known,
            ),
            // Nothing was passed for a parameter the body believes. The
            // frontend refuses a call with too few arguments as `SC0303`,
            // through `void g();` too, though C17 6.5.2.2 p2 constrains the
            // count only where the callee has a prototype; so nothing
            // reaches this today, and it answers for the day that refusal
            // is lifted.
            None => Some((Fit::Unknown, None)),
        };
        say_leaving(
            findings,
            departed,
            analysis,
            (at, Departure::Argument, width),
            answer,
            false,
        );
    }
}

/// The local a `free` or a `realloc` at this terminator is handed, where it
/// holds a pointer read out of fresh memory that nothing may have replaced,
/// so was reported where it was read: [`Known::holds_fresh_read`]. A doubt
/// about that free is the same defect again. See ADR-0057.
pub(super) fn freed_after_its_read_was_reported(
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
) -> Option<LocalId> {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin: _,
    } = terminator
    else {
        return None;
    };
    match analysis.callee(*callee) {
        Callee::Frees | Callee::Reallocates => {}
        Callee::Allocates
        | Callee::ReturnsFirst
        | Callee::Copies
        | Callee::Ends
        | Callee::Opaque => return None,
    }
    match arguments.first() {
        Some(Operand::Copy(place))
            if place.projection.is_empty() && known.holds_fresh_read(LocalIdx::of(place.local)) =>
        {
            Some(place.local)
        }
        Some(Operand::Copy(_) | Operand::Constant(_)) | None => None,
    }
}

/// Report a `return` of a pointer to an allocation that may have been freed.
///
/// **Asked at the write into the return place, of the local being written**,
/// as [`used`] asks a dereference, so that an escaped local is distrusted here
/// as it is there: asking the return place at [`Terminator::Return`] instead
/// proved a return that a dereference of the same local only doubts, because
/// the return place never escapes. For C the two points are one, since the
/// lowering puts nothing that frees between the write and the return: the
/// sequence point that ends the `return`'s full expression, and the
/// `StorageDead` of every scope the `return` leaves, which ends a local's
/// storage and frees no allocation. A check about a local's own storage is
/// the one for which the two points differ, and has to ask at the return;
/// `docs/c-family.md` says what this asks of another frontend.
///
/// **Only where the function returns a pointer.** Every call's result is a
/// site and exposed, a later call this check cannot read unproves it, and an
/// addition of two integers keeps its operands' sites (ADR-0030), so
/// `return f() + g();` reached `f`'s result after `g` ran and was refused, in
/// six corpus cases.
///
/// **Every site the local may hold, a parameter's included.** Leaving a doubt
/// about a parameter's allocation to the caller was tried and withdrawn: a
/// caller that hands the result on as an argument asks nothing, so a
/// parameter freed on one arm and returned was silent in every function. See
/// ADR-0041.
///
/// **A place of dereferences is asked what it holds**, `return *tab;` as
/// `int *q = *tab; return q;` is, through [`Known::handed_reached`]. Through
/// [`say`], and after [`used`], because a dereference of the same place at the
/// same span can stand at that key, and it is the earlier read: reading
/// `*tab` comes before returning what it held, so a doubt or a proof about it
/// is the report kept, as at a call. See ADR-0045.
pub(super) fn returned(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    element: &Element,
    known: &Known,
) {
    let Element::Assign(operation) = element else {
        return;
    };
    if operation.place != Place::local(function.return_place())
        || !analysis.is_pointer(function, function.return_place())
    {
        return;
    }
    // A constant holds no allocation, and a place with an `Index` is an
    // array's element, which this does not follow.
    let Rvalue::Use(Operand::Copy(source)) = &operation.value else {
        return;
    };
    if !source.projection.is_empty() && derefs(source) == 0 {
        return;
    }

    let Some(verdict) = verdict(
        analysis.sources,
        Kind::ReturnAfterFree,
        known.handed_reached(source),
        known,
    ) else {
        // **And what it points at, at every level**, where it said
        // nothing itself: what leaves holding a freed pointer behind it is
        // the same defect one level down (#651). See `freed_behind`.
        freed_behind(
            findings,
            said,
            analysis,
            source,
            operation.origin.span(),
            Kind::FreedBehindReturn,
            known,
        );
        return;
    };
    say(
        analysis.sources,
        findings,
        said,
        source,
        Finding {
            function: analysis.function,
            kind: Kind::ReturnAfterFree,
            conclusion: verdict.conclusion,
            at: operation.origin.span(),
            local: own_local(source),
            freed: verdict.freed,
            made: verdict.made,
            unproven: verdict.unproven,
        },
    );
}

/// Report a pointer stored where code outside this function may read it,
/// where the allocation it points at may have been freed.
///
/// **A return's other door** (ADR-0041): what a caller, or code this check
/// cannot read, reads out of that memory it believes live, as it believes a
/// call's result, and nothing reads this function's body from there. So the
/// store is the last place anything can be said about it, and it is asked
/// what [`returned`] asks, of the value stored: `free(x); *pp = x;` with
/// `pp` a parameter built in silence (#641). The same event is
/// [`Departure::Store`] to [`departed_in_element`], which asks it whether
/// what leaves is whole rather than whether it is live.
///
/// **A store through a pointer that leaves**, [`Known::leaves`], **or a write of a
/// local whose address such code holds**, `keep(&slot); slot = x;`, which
/// is the same store spelled without the pointer: one
/// [`Known::locals_named_by_unknown_code`] names, which is a local a call
/// was handed the address of and one whose address such a local holds,
/// `keepp(&slot); slot = &t; t = x;`, before any later call (#655).
/// A store into memory only this function reads is not asked here: a load
/// of it carries what it held, and a dereference of that is asked. **If
/// that memory leaves later**, by a return or a store of its address, what
/// it holds is asked there, at every level, by [`freed_behind`] (#651,
/// #654).
///
/// **Not a store of a live pointer that is freed after it**: the store is
/// of what was live as it ran. Where the memory it lands in leaves later,
/// by a return or a store of its address, that door asks what it holds;
/// where it is the caller's own, through a parameter, no door is left,
/// which is #650. A library copy out of the function is asked by
/// [`copied_away`], as a doubt (#652).
///
/// **Like a return, it doubts what it cannot prove live**: a pointer a call
/// this check cannot read returned, stored out after another such call, is
/// `Unknown` with no free in the function. ADR-0041 records that cost.
pub(super) fn stored_away(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    element: &Element,
    known: &Known,
) {
    let Some(store) = analysis.store_in(element, known) else {
        return;
    };
    // An assignment by name or through a dereference; a library copy is
    // `copied_away`'s, and a projection with no dereference lands in this
    // function's own object. See `store::StoreKind`.
    match store.kind {
        StoreKind::Direct => {}
        StoreKind::Through { derefs } if derefs > 0 => {}
        StoreKind::Through { .. } | StoreKind::Copy => return,
    }
    if !store.leaves || !analysis.may_be_pointer(function, &store.into) {
        return;
    }
    // A constant holds no allocation, and a place with an `Index` is an
    // array's element, which this does not follow, as at a return. **No case
    // holds this guard, measured**: the lowering reads an element through
    // its address, so no source of that shape reaches here from C.
    let Written::Value(Rvalue::Use(Operand::Copy(source))) = store.from else {
        return;
    };
    if !source.projection.is_empty() && derefs(source) == 0 {
        return;
    }

    let Some(verdict) = verdict(
        analysis.sources,
        Kind::StoreAfterFree,
        known.handed_reached(source),
        known,
    ) else {
        // **And what it points at, at every level**, where it said
        // nothing itself: what leaves holding a freed pointer behind it is
        // the same defect one level down (#651). See `freed_behind`.
        freed_behind(
            findings,
            said,
            analysis,
            source,
            store.at,
            Kind::FreedBehindStore,
            known,
        );
        return;
    };
    say(
        analysis.sources,
        findings,
        said,
        source,
        Finding {
            function: analysis.function,
            kind: Kind::StoreAfterFree,
            conclusion: verdict.conclusion,
            at: store.at,
            local: own_local(source),
            freed: verdict.freed,
            made: verdict.made,
            unproven: verdict.unproven,
        },
    );
}

/// Report a library copy that puts where code outside this function may
/// read it a pointer that may have been freed, as the doubt about the store
/// it makes: `memcpy(pp, &x, 8)` after `free(x)`, with `pp` a parameter,
/// puts in `*pp` what `*pp = x;` puts there (C17 7.24.2.1, 7.24.2.2), and was
/// only the doubt [`handed`] raises about what the call is handed (#652).
///
/// **Only ever a doubt.** The store reads `x`, whose value C makes
/// indeterminate once what it points at is freed (6.2.4 p2), so the store is
/// undefined in this function and [`stored_away`] proves it. The copy reads
/// `x`'s bytes, which C defines (6.2.6.1 p4): what is undefined is a later
/// read of what the copy put there as a pointer, which may be another
/// function's, so a proof would claim an undefined execution of this one
/// that it does not have (ADR-0027). Measured as a proof, it was also
/// certain of a copy of no bytes.
///
/// **What the copy carries is the local its source names**, where the source
/// is that local's address and nothing else, `&x`, so that `x` is asked by
/// name as the store asks it; otherwise what the source points at. **And
/// only of what was certainly freed**: taking `&x` for the copy makes `x`
/// escaped (ADR-0017), so whatever else it may hold is unproven, and asking
/// that doubted a copy of a live `x`, measured. Anything less is left to
/// [`handed`]'s doubt about what the call is handed, as before.
///
/// Answers whether it said anything, which [`handed`] then asks the copy's
/// source nothing of, so that one defect is one report.
pub(super) fn copied_away(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
) -> Option<Place> {
    let store = analysis.store_at(terminator, known)?;
    // A library copy alone; an assignment is `stored_away`'s. See
    // `store::StoreKind`.
    let from = match (store.kind, &store.from) {
        (StoreKind::Copy, Written::Bytes(from)) => *from,
        (StoreKind::Copy, Written::Value(_))
        | (StoreKind::Direct | StoreKind::Through { .. }, _) => return None,
    };
    if !store.leaves {
        return None;
    }
    let function = analysis.unit.function(analysis.function);
    let held = &known.points_to[from.local];
    let named: Vec<LocalIdx> = held.locals().collect();
    let source = match named.as_slice() {
        [only]
            if from.projection.is_empty()
                && held.sites().next().is_none()
                && matches!(held.offset, At::Bytes(0))
                && !held.beyond().any_unknown() =>
        {
            Place::local(
                function
                    .locals()
                    .nth(only.index())
                    .expect("an edge names a local of this function"),
            )
        }
        _ => {
            let mut source = from.clone();
            source.projection.push(Projection::Deref);
            source
        }
    };
    let reached: Vec<Reached> = known
        .handed_reached(&source)
        .into_iter()
        .filter(|reached| match reached {
            Reached::Site(site) => matches!(known.state[*site], SiteState::Freed { .. }),
            Reached::SetFreed(_) => true,
            Reached::Lost(_) | Reached::Partial => false,
        })
        .collect();
    // **And a freed pointer one level below what it copies**, as the store
    // asks it (`stored_away`): asked by the store alone, `*out = b;` was
    // `SC0414` and `memcpy(out, &b, 8)` of the same `b` was the argument
    // door's `SC0407` (the memory model rebuilt on #924, stage 2). Said, it
    // stands for the copy, so the argument door says nothing more of it.
    // **Or a freed pointer one level below what it copies**, as the store
    // asks it (`stored_away`, by `freed_behind`): asked by the store alone,
    // `*out = b;` was `SC0414` and `memcpy(out, &b, 8)` of the same `b` was
    // the argument door's `SC0407` (the memory model rebuilt on #924, stage
    // 2). **Only what is freed there, certainly or on some path**: the local
    // a copy reads through has had its address taken, so what is below it
    // is always partly unknown, and asked of everything below, a block that
    // holds itself was doubted freed with no free in the program; asked of
    // what is certainly freed alone, a pointer freed on one arm was the
    // argument door's `SC0407` where the store said `SC0414`. Said, it
    // stands for the copy, so the argument door says nothing more of it.
    if reached.is_empty() {
        let below: Vec<Reached> = known
            .handed_below(&source)
            .into_iter()
            .filter(|reached| match reached {
                Reached::Site(site) => matches!(
                    known.state[*site],
                    SiteState::Freed { .. } | SiteState::Unknown
                ),
                Reached::SetFreed(_) => true,
                Reached::Lost(_) | Reached::Partial => false,
            })
            .collect();
        let verdict = verdict(analysis.sources, Kind::FreedBehindStore, below, known)?;
        say(
            analysis.sources,
            findings,
            said,
            &Place {
                local: source.local,
                projection: source
                    .projection
                    .iter()
                    .cloned()
                    .chain([Projection::Deref])
                    .collect(),
            },
            Finding {
                function: analysis.function,
                kind: Kind::FreedBehindStore,
                conclusion: Conclusion::Unknown,
                at: store.at,
                local: None,
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
        return Some(from.clone());
    }
    let verdict = verdict(analysis.sources, Kind::StoreAfterFree, reached, known)?;
    say(
        analysis.sources,
        findings,
        said,
        &source,
        Finding {
            function: analysis.function,
            kind: Kind::StoreAfterFree,
            conclusion: Conclusion::Unknown,
            at: store.at,
            local: own_local(&source),
            freed: verdict.freed,
            made: verdict.made,
            unproven: verdict.unproven,
        },
    );
    Some(from.clone())
}

/// Where every one of `sites` was made, where they all agree, and nothing
/// where they do not or one was not made where this check saw it. One answer
/// for the two stored-out doubts, at a free and at a call, so neither names
/// one `malloc` for an allocation that may be either of two.
pub(super) fn made_where_all_agree(sites: &[Site], known: &Known) -> Option<Span> {
    let mut made = None;
    for (index, &site) in sites.iter().enumerate() {
        let from = match known.state[site] {
            SiteState::Live(made) | SiteState::Freed { made, .. } => made,
            SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
        };
        made = if index == 0 { from } else { same(made, from) };
    }
    made
}

/// What a call this check cannot read may have freed, as its transfer takes
/// it: every allocation `Allocations::handed_to_opaque_call` names and they
/// hold, which the transfer marks `may_be_freed`; what the locals it is handed
/// the address of hold, `Allocations::locals_handed_to_opaque_call`, which it
/// leaves `Reachable`, so `release_pp(&x)` and `release_all(a)` with `a` a
/// local array are asked; and, at a hatch, every allocation, as the hatch's
/// transfer doubts them all (ADR-0038). Not an allocation exposed to an
/// earlier call and not handed to this one, which would doubt every call after
/// a store out. Found by review of #672.
fn freed_by_opaque_call(
    analysis: &Allocations<'_>,
    function: &Function,
    callee: FuncId,
    handed: &[Operand],
    known: &Known,
) -> BTreeSet<Site> {
    if analysis.frees_anything(callee) {
        return known.state.indices().collect();
    }
    let mut sites = analysis.handed_to_opaque_call(function, handed, known);
    for local in Allocations::locals_handed_to_opaque_call(handed, known) {
        sites.extend(known.points_to[local].sites());
    }
    known.closure(sites)
}

/// Report a call this check cannot read handed an allocation whose address
/// was stored where code outside this function may read it (#672): `*pp = x;
/// release(x);` may free what the caller is left holding.
///
/// **What the call may free is what its transfer takes it to have**,
/// [`freed_by_opaque_call`].
/// Only a doubt, since the call may keep what it is handed rather than free
/// it: `*out = n; append(list, n);` is doubted by that decision. The other
/// order, `release(x); *pp = x;`, is the store door's. See
/// `Known::stored_out` and ADR-0041.
pub(super) fn handed_stored_out(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    terminator: &Terminator,
    known: &Known,
) {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return;
    };
    // Written out, so that a kind of callee nobody has added yet answers
    // here whether what it is handed may be freed.
    match analysis.callee(*callee) {
        Callee::Opaque => {}
        Callee::Frees
        | Callee::Reallocates
        | Callee::Allocates
        | Callee::ReturnsFirst
        | Callee::Copies
        | Callee::Ends => return,
    }
    let stored: Vec<Site> = freed_by_opaque_call(analysis, function, *callee, arguments, known)
        .into_iter()
        .filter(|&site| known.stored_out[site].now().is_some())
        .collect();
    if stored.is_empty() {
        return;
    }
    // **Keyed by an argument that holds one by name**, which the words then
    // name, **and otherwise by one that reaches one**, asked one argument at
    // a time as the call is and named by nothing, since what it holds rather
    // than itself was stored out. Not merely the first argument: a report
    // already at this call about another one keeps that key, and `rel2(y,
    // *q)` with `y` doubted freed and `*q` stored out lost the second doubt
    // to the first. Found by review.
    let by_name = arguments.iter().find_map(|argument| match argument {
        Operand::Copy(place)
            if place.projection.is_empty()
                && known.points_to[place.local]
                    .sites()
                    .any(|site| known.stored_out[site].now().is_some()) =>
        {
            Some(place)
        }
        Operand::Copy(_) | Operand::Constant(_) => None,
    });
    let reaching = || {
        arguments.iter().find_map(|argument| match argument {
            Operand::Copy(place)
                if freed_by_opaque_call(
                    analysis,
                    function,
                    *callee,
                    std::slice::from_ref(argument),
                    known,
                )
                .into_iter()
                .any(|site| known.stored_out[site].now().is_some()) =>
            {
                Some(place)
            }
            Operand::Copy(_) | Operand::Constant(_) => None,
        })
    };
    // **A hatch is handed nothing it frees**, and every allocation is what it
    // may free, so no argument need reach what was stored out; its key is the
    // return place, which no other finding at a call takes. Only a hatch
    // reaches it, since every other call's set is its arguments' together.
    let unhanded = Place::local(function.return_place());
    let place = by_name.or_else(reaching).unwrap_or(&unhanded);
    say(
        analysis.sources,
        findings,
        said,
        place,
        Finding {
            function: analysis.function,
            kind: Kind::HandedAfterStore,
            conclusion: Conclusion::Unknown,
            at: origin.span(),
            local: by_name.and_then(own_local),
            freed: None,
            made: made_where_all_agree(&stored, known),
            unproven: None,
        },
    );
}

/// Report a pointer handed to a call where the allocation it points at may
/// have been freed.
///
/// **Asked as [`used`] asks a dereference**, of the local an argument reads, so
/// an escaped local is distrusted here as it is there, and a local that
/// reaches no site says nothing: that is the dereference's answer to an empty
/// may-set and not a free's, because the callee reads what it is handed rather
/// than freeing a pointer this check lost. See ADR-0042.
///
/// **And carried to a later call, as a dereference is.** This asks what was
/// true where the call is reached; `Allocations::call` records the same
/// arguments as a [`PendingRead`](super::parts::PendingRead), and [`used_before`] asks them again at a
/// free the same full expression leaves unordered against this call.
///
/// **One finding per place per call**, so `g(p, p)` is one report, and
/// [`handed_places`] is what says which.
///
/// **Through [`say`], because [`used_before`] reaches the same caret about the
/// same local.** `int **q = &a; (memset(a, 0, 4) != 0) + (free(a), 0)` is
/// doubted here, since `a` escaped, and doubted again from the free: pushed,
/// that was two `SC0407` about `a` at one caret. Pushing fails
/// `an_escaped_pointer_handed_to_a_call_before_a_free_is_one_report`. **And a
/// place of dereferences meets the key a dereference of the same place at
/// the same call holds**, which [`used`] fills first, so the read through
/// `tab` is the report kept over what `*tab` hands on.
pub(super) fn handed(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    terminator: &Terminator,
    known: &Known,
    copied: Option<&Place>,
) {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return;
    };

    // What each pointer is handed as, first, so that a pointer handed on in
    // its own right keeps its own words: in `give(tab, *tab)`, `*tab` is a
    // freed pointer passed, which says more than what `tab` holds behind it.
    let mut silent: Vec<&Place> = Vec::new();
    for place in handed_places(analysis, function, *callee, arguments) {
        // **A copy's source [`copied_away`] has said is still asked itself,
        // and not what it points at**, which is what that said: the one
        // report about it (#652). Skipped whole, `memcpy(*t, b, 8)` with `b`
        // freed lost the proof that `b` was freed, and `--allow-unknown`
        // built it, once #802 made that copy leave. A source that is a
        // local's address, `&x`, reaches the local, which nothing frees, so
        // asking it says nothing twice. **And kept out of `silent`**, the
        // pointers whose targets `freed_behind` asks below, since what this
        // source points at is what `copied_away` already said.
        let said_behind = copied == Some(place);
        let Some(verdict) = verdict(
            analysis.sources,
            Kind::ArgumentAfterFree,
            known.handed_reached(place),
            known,
        ) else {
            if !said_behind {
                silent.push(place);
            }
            continue;
        };
        say(
            analysis.sources,
            findings,
            said,
            place,
            Finding {
                function: analysis.function,
                kind: Kind::ArgumentAfterFree,
                conclusion: verdict.conclusion,
                at: origin.span(),
                local: own_local(place),
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }

    // **And what a pointer that said nothing points at, at every level**, by
    // `freed_behind`. Keyed as `*place`, so that `give(tab, *tab)` is one
    // report rather than two: what `tab` hands on one level in is what
    // `*tab` hands on. Not carried forwards as the pointer is, because only
    // allocations already freed are asked, and those are reported there.
    // See ADR-0042.
    //
    // **Not a library copy's destination**, which `memcpy` and `memmove`
    // write and do not read (C17 7.24.2.1, 7.24.2.2): what it held before is
    // not handed on, and a later read of what the copy left there is asked
    // where it is read. Asked, a destination block that once held a freed
    // pointer was `SC0407` where the same copy into a local was silent
    // (#791).
    // A `match` rather than a `matches!`, so that a kind of callee added
    // later answers here whether it writes a destination it does not read.
    let copies = match analysis.callee(*callee) {
        Callee::Copies => true,
        Callee::Frees
        | Callee::Allocates
        | Callee::Reallocates
        | Callee::ReturnsFirst
        | Callee::Ends
        | Callee::Opaque => false,
    };
    let destination = match arguments.first() {
        Some(Operand::Copy(place)) if copies => Some(place),
        Some(_) | None => None,
    };
    for place in silent {
        if Some(place) == destination {
            continue;
        }
        freed_behind(
            findings,
            said,
            analysis,
            place,
            origin.span(),
            Kind::FreedBehindArgument,
            known,
        );
    }
}

/// Report a pointer that leaves, by a call, a return or a store, where what it
/// points at may hold a freed pointer at any level: [`Known::handed_below`],
/// keyed as `*place`, so a finding about the place itself at the same caret
/// is not repeated. A place of dereferences is asked as its load would be, so
/// `use2(*k)` is not silent where `q = *k; use2(q);` is asked. See ADR-0045.
/// Never a proof, since whoever receives it may only write there; one
/// function for the three doors, so they ask one question one way. See
/// ADR-0042 and ADR-0041.
///
/// **Asked only where the pointer itself drew nothing**, which each caller
/// checks first: a pointer reported in its own right says more than what it
/// holds behind it, as `give(tab, *tab)` shows at a call.
fn freed_behind(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    place: &Place,
    at: Span,
    kind: Kind,
    known: &Known,
) {
    let Some(verdict) = verdict(analysis.sources, kind, known.handed_below(place), known) else {
        return;
    };
    say(
        analysis.sources,
        findings,
        said,
        &Place {
            local: place.local,
            projection: place
                .projection
                .iter()
                .cloned()
                .chain([Projection::Deref])
                .collect(),
        },
        Finding {
            function: analysis.function,
            kind,
            conclusion: verdict.conclusion,
            at,
            // What is behind the pointer that leaves, which no local holds.
            // Nothing reads it for these kinds, whose words name nothing
            // because what leaves is live, so no case moves if this names the
            // place's local instead.
            local: None,
            freed: verdict.freed,
            made: verdict.made,
            unproven: verdict.unproven,
        },
    );
}

/// Put this finding at its caret, or leave the one already standing there.
///
/// **Both halves of the key, and a case for each.** Keying on the span alone
/// collapses `*p = *q;` after two frees into one report, which
/// `two_pointers_used_after_a_free_on_one_line` fails on. Keying on the place
/// alone collapses `*p = 1; *p = 2;` after one free into one, which
/// `one_pointer_used_after_a_free_on_two_lines` fails on. Neither case reaches
/// the other's mutation, which is why there are two.
///
/// **Recorded where the report is made, and not a line earlier.** Marking the
/// place as said when it had only been looked at spent the right to report it:
/// a dereference this check proved live said nothing and registered anyway, so
/// a later one of the same place at the same span was skipped as a repeat of a
/// report that never happened. Two dereferences can share a span: the C
/// lowering writes both arms of a `?:` at the whole conditional's span, and
/// wrote both operands of a `&&` or a `||` at the whole expression's until
/// #147. Through that, `if (*p || (free(p), *p))` was once exit 0 with no
/// output, a proved use of a freed value, silent, which is the worst answer
/// `docs/safety-model.md` allows for. A function rather than the tail of
/// [`used`] because [`used_before`] reaches the same caret from the other
/// direction, and two copies of this rule would be two answers to which report
/// stands.
///
/// **A proof replaces the suspicion standing at this caret**, rather than the
/// first report of a pair winning whatever it concluded.
/// `int **q = &p; if (*p || (free(p), *p))` reports the first read as unproven,
/// because taking a local's address is what makes its sites unknown, and the
/// second read is proved. Keeping the first threw the proof away and exited 0,
/// which is the silence the paragraph above describes arriving through the
/// other door. [`supersedes`] is the rule, and says which half survives a
/// pair that agrees, too.
///
/// The index `said` carries, not the position within `said`: `findings` holds
/// what every caret in this function has said, so anything reported between the
/// pair sits between them. Taking the wrong one overwrites a finding nobody was
/// replacing, and `a_proof_replaces_the_suspicion_at_one_caret` puts a double
/// free in front of the pair so that the two indices differ.
pub(super) fn say(
    sources: &SourceMap,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    place: &Place,
    finding: Finding,
) {
    let standing = said
        .iter()
        .position(|(said_at, said_place, _)| *said_at == finding.at && said_place == place);

    let Some(standing) = standing else {
        said.push((finding.at, place.clone(), findings.len()));
        findings.push(finding);
        return;
    };

    let index = said[standing].2;
    if supersedes(sources, &findings[index], &finding) {
        findings[index] = finding;
    }
}

/// Report every read behind this call that it may be about, where nothing
/// orders the two.
///
/// **The half a forward walk cannot see, arriving from the other side.** A read
/// the walk meets before the call is never asked about it, because a call marks
/// only what follows; carrying the read forwards to the call asks the same
/// question at the only point where both are in hand. `Element::Sequenced` is
/// what says a read is behind rather than beside, and clearing
/// [`Known::pending`] is where that happens. See ADR-0023.
///
/// A read is a dereference or a pointer an earlier call was handed, and each
/// is reported under the code it would have had where it ran. See ADR-0042.
///
/// **Two callees ask it, and they are asking about different things.** A
/// `free` took the site away, so the read may have run after the free. A call
/// this check cannot read may have freed what it was handed, or anything it
/// can reach because it was exposed, which is the same suspicion one step
/// weaker and is what `Allocations::call` writes as `SiteState::Unknown`
/// for everything the call may have freed. Both are the question
/// the forward walk already asks on the other side of the call, so refusing one
/// of them here left `g(*p) + h(p)` silent while `h(p) + g(*p)` reported.
/// What differs is the report: an opaque call has no free to point a second
/// caret at, and saying it had one would be this compiler asserting something
/// it did not establish.
///
/// **Always unproven, and that is the shape rather than a caution.** The read
/// and the call are in one full expression with nothing sequencing them, so one
/// allowed order reads freed storage and another does not, and which an
/// implementation picks is unspecified. There is no program this can be right
/// to call `Unsafe` about, so the worst it can do when it is wrong is a report
/// about a read that was ordered after all.
///
/// **It does not go through [`verdict`].** That answers what a set of sites is
/// worth *now*, and now is before the free, where every one of them is still
/// live: it answers `None` here, correctly, to a different question. One
/// judgement point inheriting a rule written for the other question is the
/// mistake in the other direction.
///
/// **It asks [`Allocations::touching`] over the arguments themselves, where
/// [`reported`] asks it over [`asked`], and the two cannot be made to agree
/// because they can never both have something to say.** ADR-0027's exemption
/// takes an argument out wherever [`NullAtTerminators::established_null`] holds, and
/// three lines put that answer and this walk in different programs:
///
/// 1. `nullability::null_at_terminators` zeroes a block's whole row unless
///    `block.elements.last()` is [`Element::ArgumentsEvaluated`], which is
///    ADR-0027's ordering condition;
/// 2. [`Allocations::element`](super::allocations::Allocations#method.element) answers that same element by clearing
///    [`Known::pending`], because C17 6.5.2.2 p10 orders a call's arguments
///    before the call;
/// 3. this runs after every element of the block and reports only out of
///    `pending`.
///
/// So where a row can exempt anything the marker is last, `pending` was just
/// emptied, and there is nothing here to report; and where this reports, the
/// free is under an unsequenced operator, the marker is absent, and the row is
/// `false` for every local. The `Callee::Opaque` half is the same `pending`
/// reached through the same elements.
///
/// **Written because it was measured and not because it follows**, since a
/// claim that the code cannot be written another way is checked only by
/// writing it that way: the filter was added here, and the reproducer on #219
/// and the whole
/// workspace suite came back byte for byte the same. The `debug_assert!` below
/// is what keeps that true, because a paragraph does not.
///
/// What would make the filter live is an ordering term asked per local across a
/// block edge rather than per block, which is a lattice dimension and is
/// ADR-0027's own Consequences rather than this function's.
pub(super) fn used_before(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
    null: &NullAtTerminators,
    block: BlockId,
) {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return;
    };

    // Which of the two questions above this is, and the one callee that asks
    // neither. Written as a match rather than as a comparison so that a fourth
    // callee is answered for here by `error[E0004]` rather than falling into a
    // row decided before it existed. The second half is whether the call may
    // free what it was not handed, which only code this check cannot read can.
    let (frees, beyond_its_arguments) = match analysis.callee(*callee) {
        Callee::Frees => (true, false),
        // `realloc` may free what it was handed and may not, which is what an
        // opaque call is to a read carried to it. Only what it was handed:
        // C17 7.22.3.5 p2 deallocates the old object and nothing else.
        Callee::Reallocates => (false, false),
        Callee::Opaque => (false, true),
        // `malloc` frees nothing and takes no pointer, so a read carried to it
        // is a read this call has nothing to say about; the library functions
        // that return their first argument free nothing either.
        Callee::Allocates | Callee::ReturnsFirst | Callee::Copies => return,
        // Frees nothing, and nothing after it runs: a read unordered against
        // it either ran before it or never ran. Answered as `Callee::Opaque`
        // is, `(a[0] + (*out = a, 0)) + (exit(1), 0)` doubted `a[0]`, since
        // the store exposed `a`. Mutation: answer `(false, true)`. Then
        // `a_read_unordered_against_exit_after_a_store_exposed_it_is_not_doubted`
        // reports `SC0402`.
        Callee::Ends => return,
    };

    // The machine behind the paragraph above. Placed here because this is
    // where the two rules would have met: `frees` is decided and `touching` is
    // about to be asked over arguments no exemption has been applied to.
    //
    // An assertion rather than the filter, because the filter is a line no
    // mutation can break, and such a line guards nothing, and because it
    // would go quiet on its own the day the ordering term widens: it would
    // exempt a read without anybody asking whether ADR-0027's four conditions
    // still hold where the exemption had newly arrived.
    //
    // **This junction is one the corpus reaches**, which is what makes the
    // assertion worth its line. Deleting the `known.pending.is_empty()`
    // disjunct panics `a_free_of_a_pointer_proved_null`,
    // `a_local_given_nothing_forgets_the_set_it_freed` and
    // `a_pointer_set_to_nothing_after_a_free_holds_nothing`: all three reach
    // here with an argument this check established null, and an empty
    // `pending` is the only reason nothing is reported about it.
    //
    // **What it will not do is catch the widening it is written for, on the
    // corpus.** Forcing `nullability`'s `ordered` true panics nothing; it
    // fails `a_free_in_an_unsequenced_operand_is_not_exempt` and
    // `..._across_a_call_is_not_exempt`, which go from `error[SC0401]` to exit
    // 0 with nothing said. So the widening is already guarded, one row above
    // what this offers, and what this adds is a debug run on a program the
    // corpus does not have: a read carried through a *second* pointer, so that
    // ADR-0025's refinement does not clean the one being freed.
    //
    // Two further measurements, so that the next reader does not repeat them.
    // Weakening this `any` to `all` breaks nothing. Deleting the assertion
    // leaves `null` and `block` unused, which is two warnings, and errors only
    // because the gate runs clippy with `-D warnings`; deleting the two
    // parameters along with it compiles clean. That is a guard against
    // forgetting rather than against deciding.
    debug_assert!(
        !frees
            || known.pending.is_empty()
            || !arguments
                .iter()
                .any(|argument| established_null(argument, null, block)),
        "a read was carried to a free of a pointer established null: `asked` now reaches this walk"
    );

    let touched = Allocations::touching(arguments.iter(), known);
    let taken: Vec<Site> = named(&touched).collect();
    // **A call this check cannot read may free more than it was handed**:
    // whatever it reaches through what it was handed, closed over what those
    // allocations hold, and whatever something else had made reachable to
    // code it cannot read. The second is per read, because the read's own
    // call exposing a site does not make it reachable to this call before the
    // read ran. After a hatch, anything. Asking the arguments alone left
    // `(x = p[0]) + (release_all(), 0)` over a parameter silent while its
    // swapped spelling reported. See ADR-0042, which has the programs each
    // part is held by and the rules it replaced.
    //
    // **Not for a free or `realloc`**, which free only what they are handed.
    let (own_reach, anything) = if beyond_its_arguments {
        let function = analysis.unit.function(analysis.function);
        let reach = analysis.reach(function, arguments, taken.iter().copied(), known);
        (known.closure(reach), analysis.frees_anything(*callee))
    } else {
        (BTreeSet::new(), false)
    };

    for ((.., place, _), read) in &known.pending {
        // **A read this call's own arguments made is behind it**, so it is
        // skipped here and kept for whatever else in the expression may free:
        // `strlen(strcpy(s, t)) + (free(s), 0)` still carries `strcpy`'s
        // argument to the free. See ADR-0043.
        if inside(analysis.sources, read.at, origin.span()) {
            continue;
        }
        let both: Vec<Site> = read
            .sites
            .iter()
            .copied()
            .filter(|site| {
                taken.contains(site)
                    || (beyond_its_arguments
                        && (anything || own_reach.contains(site) || read.reachable.contains(site)))
            })
            .collect();

        if both.is_empty() {
            continue;
        }

        // **Only while every allocation they have in common agrees**, which is
        // `verdict`'s rule about `made` and is here for its reason: naming one
        // of several allocations as *the* one is the may-set mistake about a
        // label, and `allocated here` is the claim.
        let mut made = None;
        for (index, site) in both.iter().enumerate() {
            let from = match known.state[*site] {
                SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
            };
            made = if index == 0 { from } else { same(made, from) };
        }

        say(
            analysis.sources,
            findings,
            said,
            place,
            Finding {
                function: analysis.function,
                // The code the read would have had where it ran, so that
                // `(a[0] = 0) + (free(a), 0)` stays `SC0402` and its memset
                // spelling is `SC0407`, as each is in the other order.
                kind: match read.read {
                    Read::Dereference => Kind::UseAfterFree,
                    Read::Argument => Kind::ArgumentAfterFree,
                },
                conclusion: Conclusion::Unknown,
                at: read.at,
                local: pending_local(read.read, place),
                // Exactly one free, which is this one: the reads are carried
                // to each free separately, so there is nothing folded here and
                // nothing for the caret to be wrong about. An opaque call has
                // freed nothing this check established, so there is no span to
                // put `freed here` on and no order to explain, which is what
                // keeps C17 6.5.2.2 p10's note off a program with no free in
                // it. A label is a claim, and must not say more than the
                // analysis established.
                freed: frees.then(|| origin.span()),
                made,
                unproven: Some(if frees {
                    Unproven::Unsequenced
                } else {
                    // The reason this is, rather than one lent to it: the
                    // variant's own doc says a call this check cannot read was
                    // handed a pointer, or could reach it because it was
                    // exposed, and may have freed it.
                    Unproven::Disagreement
                }),
            },
        );
    }
}

/// Report every pending read that something exposed while a call this check
/// cannot read was pending.
///
/// **The finding `used_before` makes for a read carried to an opaque call**,
/// for the same reason: the call may free what the read read, and which order
/// runs is C's to leave open. It is asked where a marker is about to clear
/// what is pending, and at a return, rather than after every transfer: a free
/// later in the same expression reports through `used_before` first, with
/// `freed here`, and [`say`] keeps it over this one, which names no free.
/// See ADR-0042.
pub(super) fn after_a_call(
    sources: &SourceMap,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    known: &Known,
    function: FuncId,
) {
    for ((.., place, _), read) in &known.pending {
        if !read.after_call {
            continue;
        }
        // Only while every allocation read agrees, for `used_before`'s reason.
        let mut made = None;
        for (index, site) in read.sites.iter().enumerate() {
            let from = match known.state[*site] {
                SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
            };
            made = if index == 0 { from } else { same(made, from) };
        }
        say(
            sources,
            findings,
            said,
            place,
            Finding {
                function,
                kind: match read.read {
                    Read::Dereference => Kind::UseAfterFree,
                    Read::Argument => Kind::ArgumentAfterFree,
                },
                conclusion: Conclusion::Unknown,
                at: read.at,
                local: pending_local(read.read, place),
                freed: None,
                made,
                unproven: Some(Unproven::Disagreement),
            },
        );
    }
}

/// Whether a report at a caret replaces the one already standing there.
///
/// **The one that tells the reader more stands.** [`used`] is where a pair at
/// one caret comes from and why one is collapsed at all; this is which half
/// survives, in five rules, each taking over only where the one before has
/// nothing to say.
///
/// 1. A proof replaces a suspicion, and a suspicion never replaces a proof,
///    because a proof is the one thing a reader must not be denied.
/// 2. Two of different [`Kind`]s leave the standing one. They ask different
///    questions, often about different allocations: in
///    `release(*tab) + (free(q), 0)`, the read of `*tab` is doubted because
///    `tab` may have been freed, and what it hands on is doubted because `q`
///    may have been. The first is nearer the root, which is ADR-0045's order,
///    and what a free names says nothing about which question comes first.
/// 3. Between two suspicions of one kind, one naming a free replaces one
///    naming none. Two proofs always name one. One gives the reader a free to look at and, for
///    [`Unproven::Unsequenced`], the clause that says why it is open:
///    `g(*p) + h(p) + (free(p), 0)` kept the doubt of the call it cannot read
///    and lost both, where `g(*p) + (free(p), 0) + h(p)` kept them, so what the
///    reader got turned on which operand was written first. **What it costs**:
///    the doubt it replaces may have a cause the free's order does not touch,
///    such as an escaped address, and that cause is what the reader meets
///    next, at the same caret, once they have separated the free. One report
///    per caret hides one reason whichever is kept.
/// 4. Where both name one, two proofs name the earlier free, by the same
///    [`earlier`] that [`SiteState::joined`] applies and for its reason: which
///    free is named should not turn on which block the walk reached first.
///    Two suspicions keep the standing one, because a suspicion that names a
///    free comes from a read carried to that free, and the walk carries it to
///    the frees of one expression in the order they run. In
///    `g(*p) + (free((0, free(q), p)), 0)` the inner free runs first and is
///    written later, so the earlier span would name the free the next report
///    on that line calls freed again.
/// 5. Otherwise the standing report stays, a tie included. Nothing here ranks
///    two reasons that name no free against each other: a `Lost` and a
///    `Disagreement` at one caret was looked for and not reached. Replacing
///    there instead moves
///    `a_place_and_what_it_points_at_handed_to_one_call_are_one_report`,
///    `a_table_and_what_it_holds_handed_to_one_call_are_both_asked` and
///    `a_doubted_read_of_a_table_is_kept_over_what_it_returns`.
///
/// **Answered per pair rather than by an ordering.** [`Conclusion`] does not
/// derive `Ord` and should not: its three variants are three answers rather
/// than three degrees, and `Safe` is not a weaker `Unsafe`. The pairs that
/// cannot arise say so rather than falling through, because a fallthrough in
/// this check was once reached by "proved" and by "gave up" at once and
/// reported the second as the first.
fn supersedes(sources: &SourceMap, standing: &Finding, new: &Finding) -> bool {
    match (standing.conclusion, new.conclusion) {
        // First, because a wildcard below would absorb them. [`verdict`]
        // answers `None` where there is nothing to report, so no `Finding`
        // carries `Safe` and no verdict reaching here concludes one; written
        // last, `(Conclusion::Unsafe, _)` answered `(Unsafe, Safe)` in silence
        // while this said every impossible pair is declared.
        (Conclusion::Safe, _) => unreachable!("a finding standing at a caret concluded Safe"),
        (_, Conclusion::Safe) => unreachable!("a verdict about a dereference concluded Safe"),
        (Conclusion::Unknown, Conclusion::Unsafe) => true,
        (Conclusion::Unsafe, Conclusion::Unknown) => false,
        _ if standing.kind != new.kind => false,
        // A proof reaching here is one [`verdict`] made, and it names its
        // free: the interior-free proof, which names none, is pushed rather
        // than said.
        (Conclusion::Unsafe, Conclusion::Unsafe) => match (standing.freed, new.freed) {
            (Some(standing), Some(new)) => earlier(sources, standing, new) != standing,
            (None, _) | (_, None) => unreachable!("a proof standing at a caret named no free"),
        },
        (Conclusion::Unknown, Conclusion::Unknown) => match (standing.freed, new.freed) {
            (None, Some(_)) => true,
            (Some(_), Some(_)) | (Some(_), None) | (None, None) => false,
        },
    }
}
