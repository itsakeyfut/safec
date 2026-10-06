//! The report: what [`findings`](super::findings) asks at each free,
//! dereference, `return` and call once the walk has settled, and how what it is
//! asked becomes a [`Finding`] with a conclusion, a caret and the spans a reader
//! is shown.

use std::collections::BTreeSet;

use crate::analysis::Conclusion;
use crate::ir::{
    BlockId, Element, FuncId, Function, Operand, Place, Projection, Rvalue, Terminator,
};
use crate::nullability::NullAtTerminators;
use crate::source::Span;

use super::built::named;
use super::known::Known;
use super::parts::{Callee, Freeing, Offset, Reached, Read, SiteState, same};
use super::transfer::Allocations;
use super::{Finding, Kind, Unproven};

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

/// What these sites amount to, or nothing where they amount to no report.
///
/// The caller decides what reaching nothing means, by what it puts in
/// `reached`: a free hands a [`Reached::Lost`] for an argument it stopped
/// following, and a dereference hands an empty iterator. That asymmetry is the
/// design rather than an accident, and [`used`] says why.
fn verdict(
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
    // `may free it again here` on a program with one free in it.
    let mut lost = false;
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
                    Some(already) => already.joined(freed),
                    None => freed,
                });
                continue;
            }
            // **Not the same as proving it live.** This is the check having
            // lost the pointer, and answering nothing about it is the failure
            // `docs/safety-model.md` is written to prevent rather than the one
            // it tolerates.
            Reached::Lost => {
                lost = true;
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
                    Some(already) => already.joined(before),
                    None => before,
                });
            }
            SiteState::Unknown | SiteState::Reachable => unknown = true,
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
    let settled = !live && !unknown && !lost && !partial;

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
        None if lost && !unknown => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Lost),
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
        Callee::Allocates | Callee::ReturnsFirst | Callee::Copies | Callee::Opaque => {
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

    let finding = |kind, verdict: Verdict| Finding {
        function: analysis.function,
        kind,
        conclusion: verdict.conclusion,
        at: origin.span(),
        freed: verdict.freed,
        made: verdict.made,
        unproven: verdict.unproven,
    };

    // Asked first because `verdict` takes what was reached by value; pushed
    // second, for the reason above.
    let inside = interior(&reached, offset, known);

    let mut found = Vec::new();
    if let Some(verdict) = verdict(Kind::DoubleFree, reached, known) {
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
            Reached::SetFreed(_) | Reached::Lost | Reached::Partial => return None,
        }
    }

    let (&first, rest) = sites.split_first()?;

    // The allocation, where every site agrees about it, because naming
    // one of several as the one freed is a caret on an allocation the value
    // may not hold.
    let made_of = |site: usize| match known.state[site] {
        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
        SiteState::Unknown | SiteState::Reachable => None,
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
        let Some(verdict) = verdict(Kind::UseAfterFree, known.reached_by(place.local), known)
            .or_else(|| {
                (1..derefs(place)).find_map(|depth| {
                    verdict(
                        Kind::UseAfterFree,
                        known.reached_below(place.local, depth),
                        known,
                    )
                })
            })
        else {
            continue;
        };

        say(
            findings,
            said,
            place,
            Finding {
                function,
                kind: Kind::UseAfterFree,
                conclusion: verdict.conclusion,
                at,
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }
}

/// Report a `return` of a pointer to an allocation that may have been freed.
///
/// **Asked at the write into the return place, of the local being written**,
/// as [`used`] asks a dereference, so that an escaped local is distrusted here
/// as it is there: asking the return place at [`Terminator::Return`] instead
/// proved a return that a dereference of the same local only doubts, because
/// the return place never escapes. For C the two points are one, since the
/// lowering puts nothing that frees between the write and the return, only the
/// sequence point that ends the `return`'s full expression;
/// `docs/c-family.md` says what that asks of another frontend.
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

    let Some(verdict) = verdict(Kind::ReturnAfterFree, known.handed_reached(source), known) else {
        return;
    };
    say(
        findings,
        said,
        source,
        Finding {
            function: analysis.function,
            kind: Kind::ReturnAfterFree,
            conclusion: verdict.conclusion,
            at: operation.origin.span(),
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
        let Some(verdict) = verdict(Kind::ArgumentAfterFree, known.handed_reached(place), known)
        else {
            silent.push(place);
            continue;
        };
        say(
            findings,
            said,
            place,
            Finding {
                function: analysis.function,
                kind: Kind::ArgumentAfterFree,
                conclusion: verdict.conclusion,
                at: origin.span(),
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
        let Some(verdict) = verdict(Kind::FreedBehindArgument, known.handed_below(place), known)
        else {
            continue;
        };
        say(
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
        Callee::Frees | Callee::Allocates => return Vec::new(),
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
/// report that never happened. Two dereferences do share a span, because both
/// operands of a `&&` or a `||` are written into one temporary at the whole
/// expression's span, and `if (*p || (free(p), *p))` was exit 0 with no output:
/// a proved use of a freed value, silent, which is the worst answer
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
/// other door. [`supersedes`] is the rule, and says why only this direction
/// replaces anything.
///
/// The index `said` carries, not the position within `said`: `findings` holds
/// what every caret in this function has said, so anything reported between the
/// pair sits between them. Taking the wrong one overwrites a finding nobody was
/// replacing, and `a_proof_replaces_the_suspicion_at_one_caret` puts a double
/// free in front of the pair so that the two indices differ.
pub(super) fn say(
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
    if supersedes(findings[index].conclusion, finding.conclusion) {
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
        if inside(read.at, origin.span()) {
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
                SiteState::Unknown | SiteState::Reachable => None,
            };
            made = if index == 0 { from } else { same(made, from) };
        }

        say(
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
/// `freed here`, and [`say`] keeps the first report at a caret. See ADR-0042.
pub(super) fn after_a_call(
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
                SiteState::Unknown | SiteState::Reachable => None,
            };
            made = if index == 0 { from } else { same(made, from) };
        }
        say(
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
pub(super) fn inside(inner: Span, outer: Span) -> bool {
    inner != outer
        && inner.file() == outer.file()
        && inner.start() >= outer.start()
        && inner.end() <= outer.end()
}

/// Whether a report at a caret replaces the one already standing there.
///
/// **A proof replaces a suspicion, and nothing else replaces anything.** Where
/// two proofs meet at one caret the first stands: both are true of the same
/// place and there is nothing to choose between them. [`used`] is where a pair
/// at one caret comes from and why one is collapsed at all.
///
/// **Answered per pair rather than by an ordering.** [`Conclusion`] does not
/// derive `Ord` and should not: its three variants are three answers rather
/// than three degrees, and `Safe` is not a weaker `Unsafe`. The pairs that
/// cannot arise say so rather than falling through, because a fallthrough in
/// this check was once reached by "proved" and by "gave up" at once and
/// reported the second as the first.
fn supersedes(standing: Conclusion, new: Conclusion) -> bool {
    match (standing, new) {
        // First, because a wildcard below would absorb them. [`verdict`]
        // answers `None` where there is nothing to report, so no `Finding`
        // carries `Safe` and no verdict reaching here concludes one; written
        // last, `(Conclusion::Unsafe, _)` answered `(Unsafe, Safe)` in silence
        // while this said every impossible pair is declared.
        (Conclusion::Safe, _) => unreachable!("a finding standing at a caret concluded Safe"),
        (_, Conclusion::Safe) => unreachable!("a verdict about a dereference concluded Safe"),
        (Conclusion::Unknown, Conclusion::Unsafe) => true,
        (Conclusion::Unknown, Conclusion::Unknown) | (Conclusion::Unsafe, _) => false,
    }
}
