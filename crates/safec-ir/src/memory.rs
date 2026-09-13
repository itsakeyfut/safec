//! Whether a program frees one allocation twice, or uses one after it was
//! freed.
//!
//! The first checks on [the safety model]'s memory axis, and the first thing
//! this compiler says about what a C program *does* rather than about how it is
//! written.
//!
//! **Two answers out of one walk.** They read one lattice: what a free does to
//! a site is what makes a later use of it a defect, so computing the states
//! twice would be the same computation twice and a second chance for the two
//! copies to disagree. [`Kind`] is how the caller tells them apart.
//!
//! **This answers a [`Finding`] rather than a diagnostic.** ADR-0011 keeps this
//! crate from seeing one, and what that buys is a check testable against IR
//! built by hand instead of only through the frontend. `safec_llvm`'s `Refusal`
//! is the same shape, and `safec` turns one of those into a diagnostic too.
//!
//! **An allocation is named by the local it first landed in.** A call's
//! destination and a parameter are the two places one can arrive from, and both
//! are locals, so a site is a [`LocalId`] and there is no second numbering to
//! keep in step. Two calls are two locals because the lowering gives each call
//! its own temporary. A parameter has to be a site as well: without one,
//! `void f(int *p) { free(p); free(p); }` has nothing to mark and the second
//! free is missed in silence, which is the worst thing this compiler can do.
//!
//! **Everything here is about one function.** Nothing reads a callee's body and
//! nothing reads a caller, so `void g(int *a, int *b) { free(a); free(b); }` is
//! silent: `a` and `b` are two sites as far as this can see, and a caller that
//! hands it one pointer twice is a double free this does not find. That is the
//! boundary rather than a defect in it, and what moves the boundary is a
//! summary per function, which nothing here has.
//!
//! [the safety model]: https://github.com/itsakeyfut/safec/blob/main/docs/safety-model.md

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve};
use crate::ir::{
    Element, FuncId, Function, LocalId, Operand, Place, Rvalue, Terminator, TranslationUnit,
};
use crate::source::{SourceMap, Span};

/// What this check can read in a callee's name.
///
/// By name because nothing else is available: an annotation saying what a
/// function does to what it is passed is the phase's last issue and does not
/// exist. C17 7.1.3 reserves the identifiers the library declares, so a program
/// that defines its own `free` has no behaviour C defines. `clang -std=c17
/// -pedantic-errors` does not diagnose one, measured, so a program that does it
/// anyway is read wrongly here and there is no way to tell from inside.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Callee {
    /// C17 7.22.3.3's `free`.
    Frees,
    /// C17 7.22.3.4's `malloc`. Read only so that its arguments are left
    /// alone: it is otherwise an ordinary call, and an allocation is named by
    /// where it landed rather than by which function made it.
    Allocates,
    /// Anything else. It may free what it was passed and this cannot tell.
    Opaque,
}

/// What is known about the allocation one site stands for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SiteState {
    /// Not freed on any path that reaches here, and where it came from.
    ///
    /// `None` where this check did not see the allocation happen: a parameter,
    /// whose allocation is a caller's, and every local nothing has allocated
    /// into. The diagnostic leaves its `allocated here` label off rather than
    /// pointing somewhere it guessed.
    Live(Option<Span>),
    /// Freed, and where the earliest free reaching here is.
    ///
    /// Earliest by position rather than by which path arrived first.
    ///
    /// ADR-0016 records that a value carrying a span can fail to reach a
    /// fixpoint, because a join keeping whichever one arrived oscillates where
    /// two of them meet below a branch inside a loop, and that it was measured
    /// as a hang. **That is not what this rule is doing here**, and borrowing
    /// the record's reason would be a claim nothing holds: `Unknown` is a top
    /// per site, so a span that would have alternated is absorbed before it
    /// can. Measured, on this lattice: keeping whichever arrived still ends
    /// every walk, and the loop-shaped test built to look for the hang passes
    /// under it.
    ///
    /// What the rule is for is **which free the diagnostic names**. Without it
    /// the answer is whichever path the worklist reached last, which is stable
    /// for one program and arbitrary between two that differ only in the order
    /// their blocks were built. `a_join_names_the_earlier_free` is the guard.
    Freed {
        /// Where the allocation came from, on the same terms as [`Self::Live`].
        made: Option<Span>,
        /// The earliest free reaching here.
        freed: Span,
    },
    /// Freed on one path and not on another, or handed to a call this check
    /// cannot read.
    Unknown,
}

impl SiteState {
    /// The two together, which is `Unknown` unless they agree.
    fn joined(self, other: Self) -> Self {
        match (self, other) {
            (Self::Live(here), Self::Live(there)) => Self::Live(same(here, there)),
            (
                Self::Freed {
                    made: here,
                    freed: from_here,
                },
                Self::Freed {
                    made: there,
                    freed: from_there,
                },
            ) => Self::Freed {
                made: same(here, there),
                freed: earlier(from_here, from_there),
            },
            _ => Self::Unknown,
        }
    }
}

/// Where an allocation came from, where two paths agree about it.
///
/// `None` where they do not, which only ever loses what was known and so cannot
/// cycle. Two paths reaching one site with two different allocations is not a
/// shape the frontend produces, because a site is the local a call writes into
/// and each call has its own; the type allows it and so this answers for it
/// rather than picking one and being wrong on the day something else does.
fn same(here: Option<Span>, there: Option<Span>) -> Option<Span> {
    match (here, there) {
        (Some(here), Some(there)) if here == there => Some(here),
        _ => None,
    }
}

/// The earlier of two spans, by where they are rather than by which arrived.
///
/// A total order over the pair, so a value carrying one can only move one way
/// and the walk ends. RK-007 in the review knowledge bank is why the file is
/// the first half: a span names its own file and two of them need not share
/// one.
fn earlier(here: Span, there: Span) -> Span {
    if (here.file().index(), here.start()) <= (there.file().index(), there.start()) {
        here
    } else {
        there
    }
}

/// What one argument of a call reaches.
///
/// The two are not the same answer and were once the same silence. A `free`
/// whose argument reaches no site used to be indistinguishable from one whose
/// sites were all proved live, and the check reported nothing for both. The
/// second is a proof; the first is this check having lost the pointer, and
/// saying nothing about it is [the safety model]'s worst failure rather than
/// its best one.
///
/// [the safety model]: https://github.com/itsakeyfut/safec/blob/main/docs/safety-model.md
enum Reached {
    /// A site the argument may hold.
    Site(usize),
    /// A pointer this check was not following: one written through a
    /// projection, or a local whose sites it had and lost.
    Lost,
}

/// Which allocations each local may hold, and what is known about each.
#[derive(Clone, PartialEq, Eq)]
struct Known {
    /// Per local, a bit per site it may point at. A site is a local, so this is
    /// square in the locals.
    ///
    /// `Vec<bool>` rather than a packed bitset: this crate takes no
    /// dependencies, and a byte per local per local is nothing at the sizes
    /// one function reaches.
    points_to: Vec<Vec<bool>>,
    /// Per site, what is known about it. Meaningless for a local nothing points
    /// at, which is most of them.
    state: Vec<SiteState>,
}

impl Known {
    /// Every site this local may point at.
    fn sites_of(&self, local: LocalId) -> impl Iterator<Item = usize> + '_ {
        self.points_to[local.index()]
            .iter()
            .enumerate()
            .filter_map(|(site, points)| points.then_some(site))
    }

    /// Stop following whatever this local held.
    fn clear(&mut self, local: LocalId) {
        self.points_to[local.index()].fill(false);
    }
}

/// The analysis: where an allocation is, and whether it has been freed.
struct Allocations<'a> {
    sources: &'a SourceMap,
    unit: &'a TranslationUnit,
    /// How many locals the function has, which is how many sites there can be.
    locals: usize,
    /// The locals a caller filled, which are sites because an allocation can
    /// arrive through one.
    parameters: Vec<LocalId>,
}

impl Allocations<'_> {
    /// What this check can read in the name of the function being called.
    fn callee(&self, id: FuncId) -> Callee {
        match self.sources.snippet(self.unit.function(id).name) {
            "free" => Callee::Frees,
            "malloc" => Callee::Allocates,
            _ => Callee::Opaque,
        }
    }

    /// What the arguments of a call reach, in the order they were written.
    ///
    /// Shared with [`check`], so that the walk which reports and the walk which
    /// computes cannot disagree about what a call touches.
    fn touching(arguments: &[Operand], known: &Known) -> Vec<Reached> {
        let mut reached = Vec::new();

        for argument in arguments {
            let place = match argument {
                // Not a pointer that went missing. `free(0)` is the case, and
                // C17 7.22.3.3 p2 makes it do nothing, so it is written on
                // purpose and is not something this check lost track of.
                Operand::Constant(_) => continue,
                Operand::Copy(place) => place,
            };

            if !place.projection.is_empty() {
                // `free(*pp)` frees whatever `pp` points at, and this check
                // follows locals rather than what they point at.
                reached.push(Reached::Lost);
                continue;
            }

            let before = reached.len();
            reached.extend(known.sites_of(place.local).map(Reached::Site));
            if reached.len() == before {
                reached.push(Reached::Lost);
            }
        }

        reached
    }
}

impl Analysis for Allocations<'_> {
    type Value = Known;

    fn height(&self, function: &Function) -> usize {
        let locals = function.locals().len();
        // Each local's set of sites only grows, so it takes at most one step
        // per site, and a site is a local. Each site's state walks `Live` to
        // `Freed` to `Unknown`; its `freed` span can only move to an earlier
        // one, which it can do at most once per site that frees; and its `made`
        // span can only fall from `Some` to `None`, once. Generous rather
        // than tight, which is the direction `Analysis::height` says to err in:
        // answering too low stops a correct analysis.
        //
        // **Nothing holds this number.** Measured: answering `locals` instead
        // leaves the whole suite passing, because no function here takes more
        // visits to a block than it has locals, and building one that did
        // would be building a program for the bound rather than for the check.
        // What a wrong answer costs is what that method promises: too low is a
        // panic naming `Analysis::height`, which is a build that stops with
        // something to read rather than a wrong answer about a program.
        locals * locals + locals * (locals + 3)
    }

    fn on_entry(&self) -> Self::Value {
        let mut known = Known {
            points_to: vec![vec![false; self.locals]; self.locals],
            // Nothing has allocated into any of these yet, so none of them can
            // say where it came from. A parameter stays this way: its
            // allocation happened somewhere this check cannot see.
            state: vec![SiteState::Live(None); self.locals],
        };

        // A parameter holds whatever the caller passed, which is a thing this
        // function can free and did not make. The module comment says why one
        // has to be a site of its own.
        for &parameter in &self.parameters {
            known.points_to[parameter.index()][parameter.index()] = true;
        }

        known
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        for (here, there) in into.points_to.iter_mut().zip(&from.points_to) {
            for (here, there) in here.iter_mut().zip(there) {
                *here = *here || *there;
            }
        }

        for (here, there) in into.state.iter_mut().zip(&from.state) {
            *here = here.joined(*there);
        }
    }

    fn element(&self, _function: &Function, element: &Element, value: &mut Self::Value) {
        // Every field written out, never `..`: RK-018 in the review knowledge
        // bank is a field added to a variant that already exists walking past
        // an exhaustive match.
        match element {
            Element::Assign(operation) => {
                // A write through a projection changes what a pointer points
                // at rather than which allocation a local holds, and this check
                // follows locals.
                if !operation.place.projection.is_empty() {
                    return;
                }

                let destination = operation.place.local;
                match &operation.value {
                    // **Not an optimisation.** `int *p = malloc(4);` lowers to
                    // a call into a temporary and a copy out of it, so without
                    // this the allocation never leaves the temporary and the
                    // headline program is not an error at all.
                    Rvalue::Use(Operand::Copy(source)) if source.projection.is_empty() => {
                        value.points_to[destination.index()] =
                            value.points_to[source.local.index()].clone();
                    }
                    // **Arithmetic on a pointer is followed.** C17 6.5.6 p8
                    // keeps the result inside the object the operand points
                    // into, so `p + 1` may hold whatever `p` holds, and `p[i]`
                    // is that addition: 6.5.2.1 p2 defines `E1[E2]` as
                    // `(*((E1)+(E2)))`. This arm used to clear the destination
                    // instead, on the stated ground that nothing read the
                    // precision yet. Something does now, and until it did the
                    // cost was invisible: `free(p); p[i] = 42;` was silence
                    // rather than a diagnostic, which is the worst answer this
                    // compiler has.
                    //
                    // Both operands, and a union rather than a choice. Which
                    // one is the pointer is a question about types and this
                    // does not read them; taking both is a may-set growing,
                    // which is the direction that cannot make a proof out of
                    // nothing. `q - p` is an integer and picks up both, and
                    // nothing dereferences an integer.
                    //
                    // Read before the write, so `p = p + 1` keeps what `p`
                    // held rather than clearing it and unioning the result.
                    //
                    // **It costs a proof where the index is a local.** A
                    // parameter is a site, so `p[i]` unions the allocation `p`
                    // holds with the site `i` is, and a site that is live stops
                    // the result being proved: `free(p); p[i] = 42;` is a
                    // warning where `free(p); p[0] = 42;` is an error. The
                    // types are in the IR and reading them would separate the
                    // two, which is #143 rather than a line here.
                    Rvalue::Binary { op: _, lhs, rhs } => {
                        let mut reached = vec![false; value.points_to.len()];
                        for operand in [lhs, rhs] {
                            let Operand::Copy(source) = operand else {
                                continue;
                            };
                            if !source.projection.is_empty() {
                                continue;
                            }
                            for (here, there) in reached
                                .iter_mut()
                                .zip(&value.points_to[source.local.index()])
                            {
                                *here = *here || *there;
                            }
                        }
                        value.points_to[destination.index()] = reached;
                    }
                    // A constant, a read through a projection, or a unary
                    // operator. None of the three is a pointer this check can
                    // follow: C17 6.5.3.3 gives unary `+`, `-` and `~`
                    // arithmetic operands only, and `!` yields an `int`.
                    Rvalue::Use(_) | Rvalue::Unary { .. } => {
                        value.clear(destination);
                    }
                    // The address of a place is not an allocation this check
                    // follows: nothing here frees a local's own storage, which
                    // is `StorageDead` and is the lifetime phase's.
                    //
                    // **But the place whose address is taken stops being
                    // something anything here proved.** A write through the
                    // pointer that just escaped can put a different allocation
                    // in it, and this check does not follow what a pointer
                    // points at, so it would not see the write. `Rvalue::Address`
                    // is the only way a local's address is taken in this IR, so
                    // this is the one door, and leaving it open is what made
                    // `int **pp = &p; *pp = q; free(q); free(p);` silent about
                    // a double free.
                    Rvalue::Address(taken) => {
                        value.clear(destination);
                        for site in value.sites_of(taken.local).collect::<Vec<_>>() {
                            value.state[site] = SiteState::Unknown;
                        }
                    }
                }
            }
            // Storage beginning or ending says nothing about what the local
            // held before, and what it holds now is nothing.
            Element::StorageLive { local, origin: _ } => value.clear(*local),
            Element::StorageDead { origin: _, local } => value.clear(*local),
        }
    }

    fn terminator(&self, _function: &Function, terminator: &Terminator, value: &mut Self::Value) {
        let Terminator::Call {
            callee,
            arguments,
            destination,
            then: _,
            origin,
        } = terminator
        else {
            // Nothing else moves an allocation. Written out rather than `_`
            // for RK-018's reason: a terminator added later has to be answered
            // for here.
            match terminator {
                Terminator::Goto(_)
                | Terminator::Branch { .. }
                | Terminator::Return
                | Terminator::Abnormal { .. } => return,
                Terminator::Call { .. } => unreachable!("the let above took it"),
            }
        };

        // What the call does to what it was handed, before what it leaves
        // behind, which is the order the two happen in.
        // A `Reached::Lost` moves nothing, because there is nothing to move:
        // what it says is that this call touched something the check was not
        // following, which is a fact about the report rather than about the
        // lattice.
        let touched = Self::touching(arguments, value);
        let sites = || {
            touched.iter().filter_map(|reached| match reached {
                Reached::Site(site) => Some(*site),
                Reached::Lost => None,
            })
        };

        match self.callee(*callee) {
            Callee::Frees => {
                for site in sites().collect::<Vec<_>>() {
                    // Whatever the site was known to have come from survives
                    // the free: the diagnostic wants to name it.
                    let made = match value.state[site] {
                        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                        SiteState::Unknown => None,
                    };
                    value.state[site] = SiteState::Freed {
                        made,
                        freed: origin.span(),
                    };
                }
            }
            // It does not free what it is passed, which is the whole of why the
            // name is read.
            Callee::Allocates => {}
            Callee::Opaque => {
                for site in sites().collect::<Vec<_>>() {
                    value.state[site] = SiteState::Unknown;
                }
            }
        }

        let Some(place) = destination else {
            return;
        };
        if !place.projection.is_empty() {
            return;
        }

        if self.callee(*callee) == Callee::Frees {
            // `free` returns nothing. The local the lowering writes it into is
            // `void` and holds none of this, which is #135.
            value.clear(place.local);
            return;
        }

        // A call leaves behind something this function did not have before, and
        // the local it landed in is what names it. **Live rather than joined
        // with what was there**: a second turn of a loop through the same call
        // is a second allocation, and carrying the first one's `Freed` across
        // would report a double free for code that allocates each time round.
        value.clear(place.local);
        value.points_to[place.local.index()][place.local.index()] = true;
        // **The span only where this check saw an allocation.** Every call's
        // destination is a site, because a call this cannot read may hand back
        // anything and a site is how that is tracked. But `allocated here` is a
        // claim, and `void *p = bar();` gives no evidence that `bar` allocated
        // anything. Naming that line was a caret asserting something nothing
        // had established, so a site whose call is not `malloc` is `Live(None)`
        // and the diagnostic leaves the label off.
        let made = (self.callee(*callee) == Callee::Allocates).then(|| origin.span());
        value.state[place.local.index()] = SiteState::Live(made);
    }
}

/// Which of the two things this check answers about a finding is.
///
/// One walk over one lattice, so this is not two checks and `docs/diagnostics.md`
/// says so where it hands the two their codes. What differs is the question:
/// the codes and the words are different, and the thing a caret lands on is a
/// call in one and a dereference in the other.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `free(p); free(p);`
    DoubleFree,
    /// `free(p); *p = 42;`
    UseAfterFree,
}

/// One thing this check concluded, and where.
///
/// Not a diagnostic: this crate cannot see one. What each conclusion costs a
/// build is `Diagnostic::concluded`'s in `safec`, which is the one place that
/// answers it, and ADR-0001 is why there is only one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Which of the two this is.
    pub kind: Kind,
    /// What that check concluded.
    pub conclusion: Conclusion,
    /// Where a caret goes: the call that frees, or the element that reads or
    /// writes through a freed pointer.
    ///
    /// Not the place's own span, which a [`Place`] does not have: the element's
    /// or the terminator's.
    pub at: Span,
    /// The earliest free reaching here, where the check knows which one it was.
    ///
    /// `None` for an `Unknown`: what makes it unknown is that the paths or the
    /// sites reaching here disagree, so there is no single free to point at.
    pub freed: Option<Span>,
    /// Where the allocation was made, where this check saw it happen.
    ///
    /// `None` for a parameter, whose allocation is a caller's, and for an
    /// `Unknown` for the reason above. `docs/safety-model.md` asks the
    /// diagnostic for this line and it is honest to leave it off rather than
    /// point at an allocation that may not be the one.
    pub made: Option<Span>,
}

/// Every double free this unit contains, and every one it cannot rule out.
///
/// One walk per function: the fixpoint answers what holds where each block
/// starts, and this replays each block from there to find the calls to report.
/// The replay rather than a second lattice, because the transfer is what
/// decides which sites a call touches and having two answers to that is having
/// one of them be wrong.
pub fn check(sources: &SourceMap, unit: &TranslationUnit) -> Vec<Finding> {
    let mut findings = Vec::new();

    for id in unit.functions() {
        let function = unit.function(id);
        // A declaration has no blocks, and `Function::blocks` panics rather
        // than answering for one.
        if !function.is_defined() {
            continue;
        }

        let analysis = Allocations {
            sources,
            unit,
            locals: function.locals().len(),
            parameters: function.parameters().collect(),
        };
        let cfg = Cfg::of(function);
        let solution = solve(&analysis, function, &cfg);

        // Per function, because a span belongs to one of them.
        let mut said: Vec<(Span, Place)> = Vec::new();

        for &id in cfg.order() {
            // `Cfg::order` holds exactly the reachable blocks and `solve` gives
            // every reachable block a value, so nothing skips here today. It is
            // a `continue` rather than a panic because what the `None` says is
            // that no execution reaches this block, and saying nothing about
            // code nothing runs is the right answer to that however it arose.
            let Some(mut known) = solution.value(id).cloned() else {
                continue;
            };

            let block = function.block(id);
            for element in &block.elements {
                // Before the transfer, which is what the element does: the
                // question is what was true where it runs.
                used(
                    &mut findings,
                    &mut said,
                    dereferenced_in_element(element),
                    &known,
                );
                analysis.element(function, element, &mut known);
            }

            // Before the terminator's own transfer, which is what turns a live
            // allocation into a freed one: the question is what was true when
            // the call was reached.
            used(
                &mut findings,
                &mut said,
                dereferenced_in_terminator(&block.terminator),
                &known,
            );
            if let Some(finding) = reported(&analysis, &block.terminator, &known) {
                findings.push(finding);
            }
        }
    }

    // In the order a reader's eye goes rather than the order the walk reached
    // them. `Cfg::of` hands blocks back in reverse postorder, so two findings
    // in two arms of one branch come out with the later line first, and
    // `DiagnosticSink` does not sort. Doing it here rather than there because
    // the sink holds diagnostics from every stage and their order is the order
    // the stages ran, which is right; this is one stage disagreeing with
    // itself. `Terminator::successors` already says its order will change when
    // an unwinding call gains an edge, and this is what keeps that from moving
    // every expectation that holds two findings.
    findings.sort_by_key(|finding| (finding.at.file().index(), finding.at.start()));

    findings
}

/// What a set of sites says about whatever touched them.
///
/// The fold from many sites to one conclusion, and the one place either kind
/// of finding makes it. RK-035 in the review knowledge bank is why there is
/// one: a
/// may-analysis's join is forced to be right by the lattice, and the code that
/// reads the answer is where the same rule gets lost.
struct Verdict {
    conclusion: Conclusion,
    /// The earliest free reaching here, where the conclusion is a proof.
    freed: Option<Span>,
    /// Where that allocation came from, where this check saw it happen.
    made: Option<Span>,
}

/// What these sites amount to, or nothing where they amount to no report.
///
/// The caller decides what reaching nothing means, by what it puts in
/// `reached`: a free hands a [`Reached::Lost`] for an argument it stopped
/// following, and a dereference hands an empty iterator. That asymmetry is the
/// design rather than an accident, and [`used`] says why.
fn verdict(reached: impl IntoIterator<Item = Reached>, known: &Known) -> Option<Verdict> {
    let mut earliest: Option<Span> = None;
    // Where the allocation came from, kept only while every freed site agrees.
    // **RK-035 one level down**: proving a double free from one of several
    // sites is the may-set mistake the fold below is written to avoid, and
    // naming one of several allocations as *the* one is the same mistake about
    // a label. `if (c) p = malloc(); else p = malloc();` reaches both, and
    // pointing at either would be a caret on an allocation the value may not
    // hold.
    let mut made: Option<Span> = None;
    let mut any_freed = false;
    let mut live = false;
    let mut unknown = false;

    for entry in reached {
        let site = match entry {
            Reached::Site(site) => site,
            // **Not the same as proving it live.** This is the check having
            // lost the pointer, and answering nothing about it is the failure
            // `docs/safety-model.md` is written to prevent rather than the one
            // it tolerates.
            Reached::Lost => {
                unknown = true;
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
                any_freed = true;
                earliest = Some(match earliest {
                    Some(already) if earlier(already, before) == already => already,
                    _ => before,
                });
            }
            SiteState::Unknown => unknown = true,
        }
    }

    // **`points_to` is a may-set, so one freed site among several is not a
    // proof.** `SiteState::joined` already answers `Unknown` where one path
    // freed a site and another did not; this is the same question across two
    // sites rather than across two paths, and answering it differently let the
    // spelling of a program decide whether it was a warning or an error. A
    // proof needs every site reached to have been freed, and nothing about it
    // to have been lost.
    let proved = !live && !unknown;

    match earliest {
        Some(freed) if proved => Some(Verdict {
            conclusion: Conclusion::Unsafe,
            freed: Some(freed),
            made,
        }),
        // Neither span is carried. What makes this unproven is that the sites
        // or the paths disagree, so there is no one free that every execution
        // reaching here went through, and no one allocation to name beside it.
        Some(_) => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
        }),
        None if unknown => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
        }),
        None => None,
    }
}

/// What this terminator is worth reporting as a free, if anything.
///
/// One finding per call rather than one per site: a local may point at several
/// allocations where a branch put them there, and two carets on one `free` say
/// one thing twice.
fn reported(analysis: &Allocations<'_>, terminator: &Terminator, known: &Known) -> Option<Finding> {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return None;
    };

    if analysis.callee(*callee) != Callee::Frees {
        return None;
    }

    let verdict = verdict(Allocations::touching(arguments, known), known)?;

    Some(Finding {
        kind: Kind::DoubleFree,
        conclusion: verdict.conclusion,
        at: origin.span(),
        freed: verdict.freed,
        made: verdict.made,
    })
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
/// written behind this check's back does arrive here as `SiteState::Unknown`
/// rather than as no site at all, because taking a local's address is what
/// makes its sites unknown. Pointer arithmetic is followed for the same reason.
/// But a pointer read out of another pointer, `int *p = *pp;`, reaches no site
/// and is silence, and so is a dereference inside a controlling expression,
/// which is #141. `docs/diagnostics.md` says what exit 0 does not mean here,
/// because a boundary that lives only in a comment is one no user can find.
fn used(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place)>,
    at: Option<(Span, Vec<&Place>)>,
    known: &Known,
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
        if said.contains(&(at, place.clone())) {
            continue;
        }
        said.push((at, place.clone()));

        let sites = known.sites_of(place.local).map(Reached::Site);
        let Some(verdict) = verdict(sites, known) else {
            continue;
        };

        findings.push(Finding {
            kind: Kind::UseAfterFree,
            conclusion: verdict.conclusion,
            at,
            freed: verdict.freed,
            made: verdict.made,
        });
    }
}

/// Where this element runs, and every place it reads or writes through a
/// pointer there.
///
/// Through a pointer, so a projection: an unprojected place is the local itself
/// and holding a freed pointer is not using it. The span is the element's,
/// because a [`Place`] has none of its own.
fn dereferenced_in_element(element: &Element) -> Option<(Span, Vec<&Place>)> {
    // Every field written out, never `..`: RK-018 in the review knowledge bank
    // is a field added to a variant that already exists walking past an
    // exhaustive match.
    match element {
        Element::Assign(operation) => {
            let mut places = projected(&operation.place);
            places.extend(dereferenced_in_rvalue(&operation.value));
            Some((operation.origin.span(), places))
        }
        // Storage beginning or ending reads nothing through anything.
        Element::StorageLive {
            local: _,
            origin: _,
        } => None,
        Element::StorageDead {
            origin: _,
            local: _,
        } => None,
    }
}

/// The same, for what a terminator reads.
fn dereferenced_in_terminator(terminator: &Terminator) -> Option<(Span, Vec<&Place>)> {
    match terminator {
        Terminator::Call {
            callee: _,
            arguments,
            destination,
            then: _,
            origin,
        } => {
            let mut places: Vec<&Place> = arguments.iter().flat_map(dereferenced_in).collect();
            if let Some(destination) = destination {
                places.extend(projected(destination));
            }
            Some((origin.span(), places))
        }
        // The span is here now and nothing reads it yet: that is the next
        // commit, and this one is about the IR carrying it.
        Terminator::Branch {
            condition: _,
            then: _,
            otherwise: _,
            origin: _,
        } => None,
        Terminator::Goto(_) | Terminator::Return | Terminator::Abnormal { to: _ } => None,
    }
}

/// The same, for what an rvalue reads.
fn dereferenced_in_rvalue(value: &Rvalue) -> Vec<&Place> {
    match value {
        Rvalue::Use(operand) => dereferenced_in(operand),
        Rvalue::Unary { op: _, operand } => dereferenced_in(operand),
        Rvalue::Binary { op: _, lhs, rhs } => {
            let mut places = dereferenced_in(lhs);
            places.extend(dereferenced_in(rhs));
            places
        }
        // **Taking an address is not a dereference**, even where what is
        // written looks like one. C17 6.5.3.2 p3: if the operand of `&` is the
        // result of a unary `*`, "neither that operator nor the `&` operator is
        // evaluated and the result is as if both were omitted". So `&*p` reads
        // nothing through `p`, and reporting it would be a use of a freed value
        // in a program that never touched one.
        Rvalue::Address(_) => Vec::new(),
    }
}

/// The same, for one operand.
fn dereferenced_in(operand: &Operand) -> Vec<&Place> {
    match operand {
        Operand::Copy(place) => projected(place),
        Operand::Constant(_) => Vec::new(),
    }
}

/// The place, where it goes through a projection, and nothing where it does not.
fn projected(place: &Place) -> Vec<&Place> {
    if place.projection.is_empty() {
        Vec::new()
    } else {
        vec![place]
    }
}
