//! The report: what [`findings`](super::findings) asks at each free,
//! dereference, `return` and call once the walk has settled, and how what it is
//! asked becomes a [`Finding`] with a conclusion, a caret and the spans a reader
//! is shown.

use std::collections::BTreeSet;

use crate::analysis::Conclusion;
use crate::ir::{
    BlockId, Element, FuncId, Function, LocalId, Operand, Place, Projection, Rvalue, Terminator,
    Ty, TyId,
};
use crate::nullability::NullAtTerminators;
use crate::source::{SourceMap, Span};

use super::built::named;
use super::known::Known;
use super::parts::{
    Callee, Extent, Freeing, Levels, Offset, Reach, Reached, Read, SiteState, earlier, same,
};
use super::transfer::Allocations;
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

/// How many dereferences `place` is, when it is nothing but dereferences, and
/// zero otherwise.
///
/// One answer for the load and for the report, so that the two cannot disagree
/// about which places are followed below their first level. A place with an
/// `Index` in it is an array's element and is not one of them. No program
/// this compiler accepts tells that apart from following it too, measured: an
/// array of pointers is refused as `SC0304`. See ADR-0045.
pub(super) fn derefs(place: &Place) -> usize {
    if place
        .projection
        .iter()
        .all(|step| matches!(step, Projection::Deref))
    {
        place.projection.len()
    } else {
        0
    }
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
        // **Not a `return`'s answer.** A call is not the end of its full
        // expression, so `(free(a), 0) + use(a)` may call `use` before the
        // free, and answering `true` proved a program C defines on that order.
        // See ADR-0042.
        Kind::ArgumentAfterFree => earliest.is_some_and(|freed| freed.sequenced),
        // The same question, one level in. No proof reaches it, because
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
        // Never asked: it is said by `loaded_indeterminate`, about no free.
        Kind::IndeterminateLoad => true,
        // Never asked: it is said by `may_not_fit`, about no free.
        Kind::OutsideItsObject { .. } => true,
        // Never asked: it is said where a pointer leaves, about no free.
        Kind::HandedOnNarrow { .. } => true,
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
/// on one `free` say one thing twice. **Two questions, though, and one caret
/// can carry both**: in `free(p); free(p + 1);` the second call frees an
/// allocation already freed, and through a pointer that is not its start.
///
/// The double free first, so that a caret carrying both reads `SC0401` above
/// `SC0404`: the sort at the end of [`findings`](super::findings) is stable and the two share a
/// span. See ADR-0036.
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
                Some(known.points_to[place.local.index()].offset)
            }
            // A constant frees nothing and a projection is already a
            // `Reached::Lost` above, which `interior` answers nothing for.
            Operand::Copy(_) | Operand::Constant(_) => None,
        })
        .reduce(Offset::joined)
        .unwrap_or(Offset::Zero);

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
    // second, for the reason above.
    let inside = interior(&reached, offset, known);

    let mut found = Vec::new();
    if let Some(verdict) = verdict(analysis.sources, Kind::DoubleFree, reached, known) {
        found.push(finding(Kind::DoubleFree, verdict));
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
///   nothing at all; [`reported`] then hands this `Offset::Zero`, which says
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
fn interior(reached: &[Reached], offset: Offset, known: &Known) -> Option<Verdict> {
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
    let made_of = |site: usize| match known.state[site] {
        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
        SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
    };
    let made = rest
        .iter()
        .fold(made_of(first), |made, &site| same(made, made_of(site)));

    let (conclusion, unproven) = match offset {
        Offset::Zero => return None,
        Offset::NonZero => (Conclusion::Unsafe, None),
        Offset::Unknown => (Conclusion::Unknown, Some(Unproven::Offset)),
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
        Operand::Copy(place) if place.projection.is_empty() => null.established(block, place.local),
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
/// `free(p); p;` is quiet about reading an indeterminate pointer, which 6.2.4
/// p2 makes undefined and which belongs to an axis with no check. A `return`
/// of one and an argument of a call are the exceptions, and [`returned`] and
/// [`handed`] ask them rather than this.
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

/// What this check can say about whether an access fits the object its
/// pointer reaches.
///
/// Three answers and no fourth: proved, which is silence; outside, which is a
/// proof; and unknown. A pointer that holds nothing is not one of them and
/// is `None` from [`fits`], because a null or absent pointer is the
/// nullability check's question and not a size. See ADR-0060.
pub(super) enum Fit {
    Inside,
    Outside {
        extent: Option<u64>,
        made: Option<Span>,
    },
    Unknown,
}

/// Whether an access of `width` bytes through `local`'s own value fits the
/// object it reaches, or `None` where `local` holds nothing to reach.
///
/// **What it is believed or was read to take, and nothing else proves it.**
/// The order of the tests is the order in which each stops this check
/// knowing anything:
///
/// 1. Holding no site, no local's address, nothing read out of memory or
///    written by a call, nothing lost, and at its start, is a null or absent
///    pointer, which is `None` here. **At its start**, because arithmetic on
///    a null or on a local's address leaves a value holding none of those
///    that is not null either, and a null test on it takes the nullability
///    check's doubt away: `int *q = p + 1; if (q) *q = 1;` with `p` null or
///    `&x` was answered by neither check. Found by review.
/// 2. Where `doubt_unseen_writes`, for an access through the local itself:
///    lost, written by a call this check cannot read, or an escaped local
///    holding anything: whatever is known about it may be stale (ADR-0017,
///    ADR-0018), so it is unknown. The escape is asked because a write
///    through the escaped address can put any pointer in the local where
///    this check does not see it, so what it holds proves nothing.
/// 3. A value read out of memory on every path is believed by the type it was
///    read as, and nothing more ([`Held::believed`](super::parts::Held::believed)),
///    nor more than what that memory is recorded to hold. One read on some
///    paths only is unknown.
/// 4. Off its start, or perhaps off it: unknown, since no distance is
///    carried (ADR-0036).
/// 5. An edge that may be the address of something inside a local rather
///    than the local: unknown, since the local's size is not that object's
///    ([`Held::inner_address`](super::parts::Held::inner_address)). Not
///    [`Held::writes_elsewhere`](super::parts::Held::writes_elsewhere), which
///    a call's result and a join with a null path both set while every edge
///    names a whole local: asking it made `int *q = f(&x); return *q;`
///    unknown.
/// 6. Otherwise every object it may reach, each site's [`Extent`] and each
///    local whose address it holds at that local's size, is folded. Every
///    lower bound at least `width` is proved; every one exact and below
///    `width` is outside; anything between is unknown, since a may-set
///    proves nothing from one member: an access that fits one of two
///    allocations a local may hold may still overflow the other.
pub(super) fn fits(
    analysis: &Allocations<'_>,
    function: &Function,
    local: LocalId,
    width: u64,
    known: &Known,
    doubt_unseen_writes: bool,
) -> Option<Fit> {
    let held = &known.points_to[local.index()];
    let holds_something = held.sites().next().is_some()
        || held.offset != Offset::Zero
        || held.may_be_a_locals_address()
        || held.loaded
        || held.foreign
        || held.lost;
    if !holds_something {
        return None;
    }
    // **Where a value leaves, or a local is reached through its own address,
    // none of these is doubted**, because each
    // stands for a write that was asked where it happened. The escape stands
    // for writes this check did not see, and each marks the local
    // (`Known::replaced`): one through a pointer it cannot pin down is a
    // store in this function, asked as one; one by a call it cannot read is
    // a store in the callee's body, asked there. `lost` is one of those, or a
    // site made again by the same call, which makes it with the same size
    // every time, so the sites folded below answer for it. Doubting them
    // where a value leaves refused `grow(&a); grow(&a);` with `a` a whole
    // allocation, the output-parameter idiom, since the first call is what
    // marks `a`; doubting the escape of a local reached through its address
    // doubted every `**pi`, since that address is what escapes it. An access
    // through the local itself still doubts all three, which costs a false
    // report and not a silence. See ADR-0060.
    if doubt_unseen_writes && (held.lost || held.foreign || known.escaped[local.index()]) {
        return Some(Fit::Unknown);
    }
    // **What a call may have written is no wider than the local was handed
    // as**, on any path ([`Known::handed_at_most`]). The belief the call left
    // is a proof and goes at a join with a path that made no call, while the
    // bound is kept: `if (c) put(pv, &c);` with `put` taking `void **` left
    // `v` holding nothing believed and nothing named, and a read through its
    // address believed a whole `int` (#632).
    if held.foreign && known.handed_at_most[local.index()].is_some_and(|most| most < width) {
        return Some(Fit::Unknown);
    }
    let sites = held
        .sites()
        .map(|site| known.extent[site].unwrap_or(Extent::AtLeast(0)));
    // `LocalId` cannot be built from an index outside the IR, so the locals
    // are walked and kept where the edge names them.
    let locals = function
        .locals()
        .filter(|target| held.writes_to.get(target.index()))
        .map(|target| {
            analysis
                .unit
                .size_of(function.local(target))
                .map_or(Extent::AtLeast(0), Extent::Exactly)
        });
    let extents: Vec<Extent> = sites.chain(locals).collect();

    // **No wider than what the memory it was read from is known to hold**,
    // where that is recorded: a `void *` slot written with a `char`'s address
    // and read back through an `int **` alias, or an `int *` copied in by
    // `memcpy`, arrives believed four bytes wide and is known to name one.
    // The belief is the floor where nothing is recorded, and the record is
    // the floor where it says less. Found by review. Where the slot is a
    // local, the belief is kept only where the local's value fits, by
    // `Allocations::read_through`, and a pointer handed on is asked at the
    // width its receiver believes, by [`contents_leaving`] (#627); a pointer
    // off its start stored as a `void *` is asked at the store.
    if let Some(believed) = held.believed {
        let recorded = extents.iter().all(|extent| extent.lower() >= width);
        return Some(if believed >= width && recorded {
            Fit::Inside
        } else {
            Fit::Unknown
        });
    }
    if held.loaded || held.offset != Offset::Zero {
        return Some(Fit::Unknown);
    }
    if held.may_be_a_locals_address() && held.inner_address {
        return Some(Fit::Unknown);
    }

    match folded(&extents, width) {
        Fit::Outside { extent, .. } => {
            let mut made: Option<Option<Span>> = None;
            for site in held.sites() {
                let here = match known.state[site] {
                    SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                    SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
                };
                made = Some(match made {
                    Some(already) => same(already, here),
                    None => here,
                });
            }
            Some(Fit::Outside {
                extent,
                made: made.flatten(),
            })
        }
        fit => Some(fit),
    }
}

/// What every object a pointer may reach, by its extent, answers for an
/// access of `width` bytes: proved where every lower bound is at least
/// `width`, outside where every one is exact and below it, and unknown
/// otherwise, since a may-set proves nothing from one member. No allocation
/// is named; the caller knows which sites these were.
fn folded(extents: &[Extent], width: u64) -> Fit {
    if extents.iter().all(|extent| extent.lower() >= width) {
        return Fit::Inside;
    }
    let exact: Option<Vec<u64>> = extents
        .iter()
        .map(|extent| match extent {
            Extent::Exactly(bytes) if *bytes < width => Some(*bytes),
            Extent::Exactly(_) | Extent::AtLeast(_) => None,
        })
        .collect();
    match exact {
        Some(bytes) => {
            // **One size to name only where every object agrees**, as one
            // allocation is named only where every site agrees: naming the
            // largest of two said `an object of 2` about a pointer that may
            // reach one of 1. Found by review.
            let extent = bytes
                .first()
                .copied()
                .filter(|first| bytes.iter().all(|other| other == first));
            Fit::Outside { extent, made: None }
        }
        None => Fit::Unknown,
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

/// The locals whose value a pointer read `depth` dereferences below `local`
/// may be, and whether it is certainly the one local named, with no memory
/// beside it.
///
/// **The walk every reader of a load takes**, [`Known::levels_below`], one
/// level short of the pointer, so that what it was read from is what a load
/// would read: a local's address stored in an allocation, `*m = &v;`, is
/// found through `m` as it is through `pi = &v;`. Walking the address edges
/// alone left `***m = 1` believing `v` after `v = p + 4;`. Found by review.
fn read_from_locals(
    function: &Function,
    known: &Known,
    local: LocalId,
    depth: usize,
) -> (Vec<LocalId>, bool) {
    let Some(above) = depth.checked_sub(1) else {
        return (Vec::new(), false);
    };
    let (sites, locals) = known.levels_below(local, above);
    let certain = sites.is_empty() && locals.len() == 1;
    // `LocalId` cannot be built from an index outside the IR, so the locals
    // are walked and kept where the walk names them.
    let locals = function
        .locals()
        .filter(|target| locals.contains(&target.index()))
        .collect();
    (locals, certain)
}

/// What one local a pointer may have been read from answers for the access,
/// where it may also have been read from somewhere else.
///
/// **A proof only where the pointer is certainly that local's value.** A
/// may-set proves nothing from one member, as [`fits`] folds one: with `pi`
/// either `&v` or `&u`, `**pi` fits wherever `pi` is `&v`, so `u` being too
/// narrow is a doubt and not a proof. Found by review.
fn only_if_certain(fit: Fit, certain: bool) -> Fit {
    match fit {
        Fit::Outside { .. } if !certain => Fit::Unknown,
        fit => fit,
    }
}

/// What the locals a pointer read through `place` may have been read from
/// answer for an access of `width` bytes through it, and the one local where
/// it is certainly that local's value, or `None` where it cannot be a local's.
///
/// **Asked where a pointer read out of memory is handed on**, as
/// [`may_not_fit`] asks it where one is gone through: `put(*pi)` and
/// `memset(*pi, 0, 4)` hand on `v`'s value, which no store asked, as `**pi`
/// goes through it. Believing the type `*pi` is read as let both through
/// after `v` was given one `char`. Found by review.
fn through_locals(
    analysis: &Allocations<'_>,
    function: &Function,
    place: &Place,
    width: u64,
    known: &Known,
) -> Option<(Fit, Option<LocalId>)> {
    let (locals, certain) = read_from_locals(function, known, place.local, derefs(place));
    let mut answer: Option<Fit> = None;
    for &local in &locals {
        // A local holding nothing is a null, which is the nullability
        // check's, and adds nothing here.
        let fit = match fits(analysis, function, local, width, known, false) {
            Some(fit) => only_if_certain(fit, certain),
            None => Fit::Inside,
        };
        answer = Some(match (answer, fit) {
            (_, Fit::Outside { extent, made }) => Fit::Outside { extent, made },
            (Some(Fit::Outside { extent, made }), _) => Fit::Outside { extent, made },
            (Some(Fit::Unknown), _) | (_, Fit::Unknown) => Fit::Unknown,
            (None | Some(Fit::Inside), Fit::Inside) => Fit::Inside,
        });
    }
    let named = match locals.as_slice() {
        [local] if certain => Some(*local),
        _ => None,
    };
    answer.map(|fit| (fit, named))
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
    let (conclusion, extent, made, unproven) = match fit {
        None | Some(Fit::Inside) => return,
        Some(Fit::Outside { extent, made }) => (Conclusion::Unsafe, extent, made, None),
        Some(Fit::Unknown) => (Conclusion::Unknown, None, None, Some(Unproven::Extent)),
    };
    let finding = Finding {
        function: analysis.function,
        kind: Kind::OutsideItsObject { width, extent },
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
/// **What the call is asked to touch is the access**, as [`LIBRARY`](super::parts::LIBRARY) says per
/// argument: a [`Reach::Length`] is an access of as many bytes as that
/// argument holds, read as the parameter receives it, and any length this
/// check cannot read is unknown; a [`Reach::String`] runs as far as a string
/// does, which nothing here reads, so it is unknown wherever the pointer
/// holds anything. A null holds nothing and is the nullability check's.
/// See ADR-0060.
pub(super) fn reached_by_library(
    findings: &mut Vec<Finding>,
    touched: &mut Vec<(Span, LocalId)>,
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
        if touched.contains(&(at, local)) {
            continue;
        }
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
            // believed by the type it was read as, as one assigned to a local
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
        let (conclusion, extent, made, unproven) = match fit {
            None | Some(Fit::Inside) => continue,
            Some(Fit::Outside { extent, made }) => (Conclusion::Unsafe, extent, made, None),
            Some(Fit::Unknown) => (Conclusion::Unknown, None, None, Some(Unproven::Extent)),
        };
        touched.push((at, local));
        findings.push(Finding {
            function: analysis.function,
            // No width where none was read: it is printed only beside a
            // proof, and a proof needs one.
            kind: Kind::OutsideItsObject {
                width: width.unwrap_or(0),
                extent,
            },
            conclusion,
            at,
            local: place.projection.is_empty().then_some(local),
            freed: None,
            made,
            unproven,
        });
    }
}

/// How many bytes one object of the type `ty` points to takes, where `ty`
/// is a pointer, and `None` for anything else.
///
/// **Zero for a `void *`, and not `None`.** A `void *` leaves believing no
/// width, but whoever converts it back believes it starts an object, so it is
/// asked that much: a width of zero makes [`fits`] answer from the start
/// alone. Answering `None` asked a `void *` nothing, and a pointer off its
/// start stored through a `void **` alias and read back through an `int **`
/// one was believed whole (#624).
///
/// **The same answer `Allocations::pointee_size` gives a load**, and on
/// purpose: what a pointer is asked where it leaves is what it is believed
/// where it arrives, so the two have to agree. See ADR-0060.
fn pointee_width(analysis: &Allocations<'_>, ty: TyId) -> Option<u64> {
    match analysis.unit.ty(ty) {
        Ty::Pointer(pointee) => Some(analysis.unit.size_of(pointee).unwrap_or(0)),
        Ty::Int | Ty::Char | Ty::Void => None,
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
        // A value leaving doubts nothing unseen, as [`fits`] says.
        Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => {
            fits(analysis, function, place.local, width, known, false)
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

/// Push what [`leaving`] answered, once per value per caret and door.
fn say_leaving(
    findings: &mut Vec<Finding>,
    departed: &mut Vec<(Span, Departure, Option<LocalId>, usize)>,
    analysis: &Allocations<'_>,
    left: (Span, Departure, u64),
    answer: Option<(Fit, Option<LocalId>)>,
) {
    let (at, how, width) = left;
    let Some((fit, local)) = answer else {
        return;
    };
    // **Unknown either way**, for the reason `Kind::HandedOnNarrow` gives;
    // what was shown is carried into the words.
    let (narrower, extent, made) = match fit {
        Fit::Inside => return,
        Fit::Outside { extent, made } => (true, extent, made),
        Fit::Unknown => (false, None, None),
    };
    let finding = Finding {
        function: analysis.function,
        kind: Kind::HandedOnNarrow {
            how,
            width,
            narrower,
            extent,
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
            if narrower && !shown {
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
    departed.push((at, how, local, findings.len()));
    findings.push(finding);
}

/// The locals a value leaving this function may point at, where it is a
/// pointer to pointers: a local's address taken here, or one a local's value
/// may hold. Empty for a constant and for anything this check does not
/// follow to a local.
fn locals_reached(function: &Function, value: &Rvalue, known: &Known) -> Vec<LocalId> {
    match value {
        Rvalue::Address(place) if place.projection.is_empty() => vec![place.local],
        Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => {
            let held = &known.points_to[place.local.index()];
            function
                .locals()
                .filter(|target| held.writes_to.get(target.index()))
                .collect()
        }
        Rvalue::Use(_) | Rvalue::Address(_) | Rvalue::Unary { .. } | Rvalue::Binary { .. } => {
            Vec::new()
        }
    }
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
/// ([`Known::inside`], [`Known::inside_locals`]) by their size. What the
/// record does not hold is not asked by its record, since asking it would
/// doubt every pointer to a parameter's memory handed on (#627); it is
/// asked by what it is believed to hold instead, from the type the
/// departing value arrived as and from what was stored there
/// ([`Held::contents_at_most`](super::parts::Held::contents_at_most),
/// [`Known::stored_at_most`], #632). See ADR-0060.
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
    if let (Some(receiver), Some(width)) =
        (analysis.type_levels(ty), pointee_width(analysis, read_as))
    {
        if arrived.narrower_than(&receiver) {
            say_leaving(
                findings,
                departed,
                analysis,
                (at, how, width),
                Some((Fit::Unknown, None)),
            );
        }
    }
    let (locals, sites) = slots_leaving(analysis, function, read_as, value, known);
    for (local, read_as) in locals {
        // A local that holds no pointer, read as none, has nothing a reader
        // believes of it; a `void *` is asked whether it starts an object.
        let own = pointee_width(analysis, function.local(local));
        let receiver = read_as.and_then(|ty| pointee_width(analysis, ty));
        let Some(width) = own.max(receiver) else {
            continue;
        };
        // Reached through its address, so its escape is not doubted, as
        // [`fits`] says.
        let fit = fits(analysis, function, local, width, known, false);
        // **And below its own level, by what its value believes of its
        // memory**, against what the receiver reads there: `use(&x)` with
        // `x` a `void **` converted to `int **` and `use` taking `int ***`
        // hands on memory whose pointers nobody was asked the width of,
        // which the local's own fit cannot see (#634). Narrower is unknown,
        // and a fit shown outside stays the answer, since it says more.
        let narrower = read_as
            .and_then(|ty| analysis.type_levels(ty))
            .is_some_and(|receiver| {
                known.points_to[local.index()]
                    .contents_at_most
                    .narrower_than(&receiver)
            });
        let fit = match fit {
            Some(Fit::Outside { .. }) => fit,
            _ if narrower => Some(Fit::Unknown),
            _ => fit,
        };
        let answer = fit.map(|fit| (fit, Some(local)));
        say_leaving(findings, departed, analysis, (at, how, width), answer);
    }
    for (site, read_as) in sites {
        // A `void *` receiver believes only a start, which the store that put
        // each pointer here asked; and memory recorded to hold nothing says
        // nothing.
        let Some(width) = pointee_width(analysis, read_as).filter(|&width| width > 0) else {
            continue;
        };
        let extents: Vec<Extent> = known.inside[site]
            .ones()
            .map(|held| known.extent[held].unwrap_or(Extent::AtLeast(0)))
            .chain(
                function
                    .locals()
                    .filter(|local| known.inside_locals[site].get(local.index()))
                    .map(|held| {
                        analysis
                            .unit
                            .size_of(function.local(held))
                            .map_or(Extent::AtLeast(0), Extent::Exactly)
                    }),
            )
            .collect();
        // And what was stored there that the record does not name, by what it
        // was believed ([`Known::stored_at_most`], #632).
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
            .stored_under(Some(width), analysis.deepest_pointer);
        let stored_narrower = known.stored_at_most[site].narrower_than(&receiver);
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
        );
    }
}

/// What [`slots_leaving`] finds: each local with the type the receiver reads
/// it as, or `None` below a type that stops being a pointer, and each site
/// with the type the receiver reads what it holds as.
pub(super) type Slots = (Vec<(LocalId, Option<TyId>)>, Vec<(usize, TyId)>);

/// Every slot a pointer leaving as a pointer to `read_as` lets leave with it,
/// with the type the receiver reads each as: the locals it may point at, at
/// `read_as`, and so on down, with `None` below a type that stops being a
/// pointer; and the memory it may point at, at `read_as`, and so on down
/// while the receiver's type is a pointer.
///
/// **One walk for every question about what a call or a store lets leave**:
/// [`contents_leaving`] asks each slot whether it fits, and
/// `Allocations::terminator` takes from it the narrowest type a call was
/// handed each local as. Two walks would answer the two differently.
pub(super) fn slots_leaving(
    analysis: &Allocations<'_>,
    function: &Function,
    read_as: TyId,
    value: &Rvalue,
    known: &Known,
) -> Slots {
    // What the departing value points at, seeded as a load assigned first
    // would hold it: a local's own value names its sites and its address
    // edges; a pointer read out of memory and handed straight on, `use(*b)`,
    // names what the walk every load takes finds there, which handed
    // nothing on before (found by review); a local's address is
    // [`locals_reached`]'s; and what arithmetic builds is asked by
    // [`leaving`] and names nothing to walk.
    let (seed_sites, seed_locals): (Vec<usize>, Vec<LocalId>) = match value {
        Rvalue::Use(Operand::Copy(place)) if derefs(place) > 0 => {
            let (sites, locals) = known.levels_below(place.local, derefs(place));
            let locals = function
                .locals()
                .filter(|local| locals.contains(&local.index()))
                .collect();
            (sites.into_iter().collect(), locals)
        }
        Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => (
            known.points_to[place.local.index()].sites().collect(),
            locals_reached(function, value, known),
        ),
        Rvalue::Use(_) | Rvalue::Address(_) | Rvalue::Unary { .. } | Rvalue::Binary { .. } => {
            (Vec::new(), locals_reached(function, value, known))
        }
    };
    let mut locals: Vec<(LocalId, Option<TyId>)> = seed_locals
        .into_iter()
        .map(|local| (local, Some(read_as)))
        .collect();
    let mut sites: Vec<(usize, Option<TyId>)> = seed_sites
        .into_iter()
        .map(|site| (site, Some(read_as)))
        .collect();
    let mut asked_locals: Vec<(LocalId, Option<TyId>)> = Vec::new();
    let mut asked_sites: Vec<(usize, TyId)> = Vec::new();
    // Each pair is expanded once, behind `asked_locals` and `asked_sites`,
    // and there are finitely many: the locals, the sites, and the types the
    // departing one points through. So the walk ends.
    loop {
        if let Some((local, read_as)) = locals.pop() {
            if asked_locals.contains(&(local, read_as)) {
                continue;
            }
            asked_locals.push((local, read_as));
            let below = read_as.and_then(|ty| pointee(analysis, ty));
            // What it points at is walked too, since a `void *` that
            // believes nothing may still point at a local that holds
            // something believed.
            let read = Rvalue::Use(Operand::Copy(Place::local(local)));
            locals.extend(
                locals_reached(function, &read, known)
                    .into_iter()
                    .map(|next| (next, below)),
            );
            sites.extend(
                known.points_to[local.index()]
                    .sites()
                    .map(|site| (site, below)),
            );
        } else if let Some((site, read_as)) = sites.pop() {
            // Below a type that is not a pointer the receiver believes
            // nothing of this memory, and its stores were asked at theirs.
            let Some(read_as) = read_as else {
                continue;
            };
            if asked_sites.contains(&(site, read_as)) {
                continue;
            }
            asked_sites.push((site, read_as));
            let below = pointee(analysis, read_as);
            sites.extend(known.inside[site].ones().map(|next| (next, below)));
            locals.extend(
                function
                    .locals()
                    .filter(|local| known.inside_locals[site].get(local.index()))
                    .map(|next| (next, below)),
            );
        } else {
            break;
        }
    }
    (asked_locals, asked_sites)
}

/// The type `ty` points to, where it is a pointer, and `None` otherwise.
fn pointee(analysis: &Allocations<'_>, ty: TyId) -> Option<TyId> {
    match analysis.unit.ty(ty) {
        Ty::Pointer(pointee) => Some(pointee),
        Ty::Int | Ty::Char | Ty::Void => None,
    }
}

/// Report a pointer this element hands back by a `return` or stores through
/// a pointer, where this check cannot show it points at one whole object of
/// the type it leaves as.
///
/// **The departure half of ADR-0060.** A caller believes what it is handed
/// back, and a later load believes what it reads, so this is where either
/// belief is asked. A `void *` is asked only whether it starts an object, and
/// a constant is a null, which the nullability check answers.
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
    let how = if operation.place == Place::local(function.return_place()) {
        Departure::Return
    } else if !operation.place.projection.is_empty() {
        Departure::Store
    } else {
        // A copy into a local leaves nothing: it is asked where the local's
        // value or its address leaves.
        return;
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
    say_leaving(findings, departed, analysis, (at, how, width), answer);
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
    // **A body here is what runs, whatever it is called.** Defining a
    // library name is undefined (C17 7.1.3 p2), and the check reads such a
    // call by name everywhere else; but the departure is asked of the body
    // that believes its parameters, so a definition of `memcmp` taking
    // `int *` is asked as any function is. Found by review.
    let defined = declared.is_defined();
    match (defined, analysis.callee(*callee)) {
        (true, _) | (false, Callee::Opaque) => {}
        // **`memcpy` and `memmove` store what they copy**, so the pointers
        // their source reaches leave into the destination as a store would
        // put them there. Found by review.
        (false, Callee::Copies) => {
            let source_ty = match arguments.get(1) {
                Some(Operand::Copy(place)) => analysis.unit.place_ty(function, place),
                Some(Operand::Constant(_)) | None => None,
            };
            if let (Some(source), Some(source_ty)) = (arguments.get(1), source_ty) {
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
            return;
        }
        // `free`, an allocator, `realloc` and a call that does not return
        // read nothing they are handed as an object, and a library function
        // that returns its first argument is asked how far it reaches by
        // `reached_by_library` instead.
        (
            false,
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Ends,
        ) => return,
    }
    let reached: Vec<usize> = analysis
        .library(*callee)
        .filter(|_| !defined)
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
        );
    }
}

/// Report every read at `at` of a pointer out of memory nothing wrote, which
/// is a use of an indeterminate value (C17 7.22.3.1 p2, 7.22.3.4 p2, Annex
/// J.2), once per place per caret. The later uses of what it read are not
/// reported again. See ADR-0057.
pub(super) fn loaded_indeterminate(
    sources: &SourceMap,
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    at: Option<(Span, Vec<Place>)>,
    known: &Known,
) {
    let Some((at, read)) = at else {
        return;
    };
    for place in read {
        if !analysis.reads_fresh(function, &place, known) {
            continue;
        }
        // **Keyed by the pointer read through, not by `*l`.** A dereference
        // of `l` at the same caret is said at `*l` by `used`, and one key
        // keeps one finding: a read out of a block nothing wrote before it
        // was freed lost its use after free to this report, though both are
        // true of it. Mutation: key by `place`, and
        // `a_pointer_read_out_of_a_freed_table_and_returned` loses `SC0402`.
        let local = place.local;
        let root = Place::local(local);
        if said
            .iter()
            .any(|(said_at, said_place, _)| *said_at == at && *said_place == root)
        {
            continue;
        }
        say(
            sources,
            findings,
            said,
            &root,
            Finding {
                function: analysis.function,
                kind: Kind::IndeterminateLoad,
                conclusion: Conclusion::Unsafe,
                at,
                local: Some(local),
                freed: None,
                made: None,
                unproven: None,
            },
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
    if !matches!(
        analysis.callee(*callee),
        Callee::Frees | Callee::Reallocates
    ) {
        return None;
    }
    match arguments.first() {
        Some(Operand::Copy(place))
            if place.projection.is_empty() && known.holds_fresh_read(place.local.index()) =>
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
/// true where the call is reached; `Allocations::terminator` records the same
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
        let Some(verdict) = verdict(
            analysis.sources,
            Kind::ArgumentAfterFree,
            known.handed_reached(place),
            known,
        ) else {
            silent.push(place);
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

    // **And what a pointer that said nothing points at, one level in**, keyed
    // as `*place`, so that `give(tab, *tab)` is one report rather than two:
    // what `tab` hands on one level in is what `*tab` hands on. A place of
    // dereferences is asked as its load would be, so `use2(*k)` is not
    // silent where `q = *k; use2(q);` is asked. See ADR-0045. Not carried
    // forwards as the pointer is, because only allocations already freed are
    // asked, and those are reported here. See ADR-0042.
    for place in silent {
        let Some(verdict) = verdict(
            analysis.sources,
            Kind::FreedBehindArgument,
            known.handed_below(place),
            known,
        ) else {
            continue;
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
                kind: Kind::FreedBehindArgument,
                conclusion: verdict.conclusion,
                at: origin.span(),
                // What is behind the pointer handed over, which no local holds.
                // Nothing reads it for this kind, whose words name nothing
                // because what is passed is live, so no case moves if this
                // names the place's local instead.
                local: None,
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }
}

/// The arguments a call is asked about as a pointer it was handed, one place
/// per local.
///
/// **One function for its two readers**, [`handed`] at the call and
/// `Allocations::terminator` carrying them forwards, because one rule written
/// in two places drifts apart inside the change that touches one of them.
///
/// **Which arguments turn on the callee**, and the `match` is written out so
/// that a new kind of callee answers here. What `free` and `realloc`'s first
/// argument are handed is asked already, as a double free, by [`reported`];
/// an allocator and `realloc`'s size are handed integers.
///
/// **A local of pointer type, or a place of dereferences that may be a
/// pointer.** The second is asked what its deepest level holds, through
/// [`Known::handed_reached`], so `release(*tab)` is asked what `q = *tab;
/// release(q);` is (ADR-0045). What the call reaches through it is
/// `read_out`'s (ADR-0040). One place is asked once, compared as a place, so
/// `g(p, *p)` asks both. The pointer test on a place of dereferences changes
/// no case, measured: without a cast, an integer is read out of an allocation
/// that holds integers, and nothing was stored there for it to hold. An integer can hold sites, since
/// an addition keeps its operands' (ADR-0030), and asking one refused
/// `h(f() + g())`, a program with no pointer in it.
///
/// **The repeat test is answered twice, and a mutation sees only the other
/// answer.** Since [`handed`] reports through [`say`], a place handed twice
/// lands on one key, so dropping the test leaves the whole suite green. It is
/// kept, because it says what is asked rather than what happens to collapse
/// afterwards, and measured without `say` it fails
/// `a_freed_pointer_handed_twice_to_one_call_is_one_report`.
///
/// **A place of dereferences lands on the key a dereference of the same
/// place at the same call holds**, which [`used`] fills first. So
/// `free(tab); release(*tab);` keeps the dereference's proof about reading
/// `*tab`, and what `*tab` holds is asked only when that says nothing.
pub(super) fn handed_places<'a>(
    analysis: &Allocations<'_>,
    function: &Function,
    callee: FuncId,
    arguments: &'a [Operand],
) -> Vec<&'a Place> {
    let arguments = match analysis.callee(callee) {
        Callee::Opaque | Callee::ReturnsFirst | Callee::Copies => arguments,
        Callee::Reallocates => &arguments[arguments.len().min(1)..],
        // A call that does not return reads nothing it is handed after its
        // arguments are evaluated, which `dereferenced_in_terminator` asks.
        // No C program reaches the difference: none of the four takes a
        // pointer as the library declares it, and one redeclared to take one
        // is undefined and believed all the same, as ADR-0051 believes it.
        Callee::Frees | Callee::Allocates | Callee::Ends => return Vec::new(),
    };

    let mut asked: Vec<&Place> = Vec::new();
    for argument in arguments {
        let Operand::Copy(place) = argument else {
            continue;
        };
        let pointer = if place.projection.is_empty() {
            analysis.is_pointer(function, place.local)
        } else {
            derefs(place) > 0 && analysis.may_be_pointer(function, place)
        };
        if !pointer || asked.contains(&place) {
            continue;
        }
        asked.push(place);
    }
    asked
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
/// weaker and is what `Allocations::terminator` writes as `SiteState::Unknown`
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
/// takes an argument out wherever [`NullAtTerminators::established`] holds, and
/// three lines put that answer and this walk in different programs:
///
/// 1. `nullability::null_at_terminators` zeroes a block's whole row unless
///    `block.elements.last()` is [`Element::ArgumentsEvaluated`], which is
///    ADR-0027's ordering condition;
/// 2. [`Allocations::element`](super::transfer::Allocations#method.element) answers that same element by clearing
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
    let taken: Vec<usize> = named(&touched).collect();
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
        let both: Vec<usize> = read
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

/// Whether a read at `inner` is part of the arguments of the call at `outer`.
///
/// **Strictly inside**, because in C a call's designator and arguments are
/// written within the call and nothing else is, so every read an argument
/// makes, a call nested there included, has a span inside the call's and no
/// read in another operand does. C17 6.5.2.2 p10's first sentence orders the
/// first kind before the call.
///
/// **An equal span is not inside.** The reads that carry exactly a call's span
/// are its own operand reads and what it is handed, which are recorded after
/// the call has been asked, so nothing is lost here by refusing them. What it
/// buys is the direction of a mistake: were a call and a sibling operand ever
/// given one span, the sibling is reported rather than skipped. See ADR-0043,
/// and `docs/c-family.md` for what this asks of another frontend.
///
/// **Located, across an instance.** A read in an included file, or later in a
/// macro's expansion, is inside the call that holds its use, as it would be
/// written out: both spans are compared in the nearest file they share, and
/// two with none in common are not inside each other. Equality is asked there
/// too, after locating, so a span that locates to exactly the outer one is not
/// inside it. That is the rule above in the direction it chose: were a call
/// ever exactly an instance's use, a read in the instance is reported rather
/// than taken for the call's own. Found by review. See ADR-0054.
pub(super) fn inside(sources: &SourceMap, inner: Span, outer: Span) -> bool {
    let Some(file) = sources.shared(inner, outer) else {
        return false;
    };
    match (sources.locate(inner, file), sources.locate(outer, file)) {
        (Some(inner), Some(outer)) => {
            inner != outer && outer.start() <= inner.start() && inner.end() <= outer.end()
        }
        _ => false,
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
