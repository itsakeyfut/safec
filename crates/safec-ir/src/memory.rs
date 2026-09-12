//! Whether a program frees one allocation twice.
//!
//! The first check on [the safety model]'s memory axis, and the first thing
//! this compiler says about what a C program *does* rather than about how it is
//! written.
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
use crate::ir::{Element, FuncId, Function, LocalId, Operand, Rvalue, Terminator, TranslationUnit};
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
    /// Not freed on any path that reaches here.
    Live,
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
    /// can. Measured, on this lattice: keeping whichever arrived leaves the
    /// whole suite green and every walk still ends.
    ///
    /// What the rule is for is **which free the diagnostic names**. Without it
    /// the answer is whichever path the worklist reached last, which is stable
    /// for one program and arbitrary between two that differ only in the order
    /// their blocks were built. `a_join_names_the_earlier_free` is the guard.
    Freed(Span),
    /// Freed on one path and not on another, or handed to a call this check
    /// cannot read.
    Unknown,
}

impl SiteState {
    /// The two together, which is `Unknown` unless they agree.
    fn joined(self, other: Self) -> Self {
        match (self, other) {
            (Self::Live, Self::Live) => Self::Live,
            (Self::Freed(here), Self::Freed(there)) => Self::Freed(earlier(here, there)),
            _ => Self::Unknown,
        }
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

    /// Move every site the arguments reach, which is what a call does to what
    /// it was handed.
    ///
    /// Shared with [`check`], so that the walk which reports and the walk which
    /// computes cannot disagree about which sites a call touches.
    fn touching<'a>(
        arguments: &'a [Operand],
        known: &'a Known,
    ) -> impl Iterator<Item = usize> + 'a {
        arguments
            .iter()
            .filter_map(|argument| match argument {
                // A projected argument is a value read through a pointer rather
                // than the pointer itself, and this check follows locals.
                Operand::Copy(place) if place.projection.is_empty() => Some(place.local),
                Operand::Copy(_) | Operand::Constant(_) => None,
            })
            .flat_map(|local| known.sites_of(local).collect::<Vec<_>>())
    }
}

impl Analysis for Allocations<'_> {
    type Value = Known;

    fn height(&self, function: &Function) -> usize {
        let locals = function.locals().len();
        // Each local's set of sites only grows, so it takes at most one step
        // per site, and a site is a local. Each site's state walks `Live` to
        // `Freed` to `Unknown`, and its span can only move to an earlier one,
        // which it can do at most once per site that frees. Generous rather
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
        locals * locals + locals * (locals + 2)
    }

    fn on_entry(&self) -> Self::Value {
        let mut known = Known {
            points_to: vec![vec![false; self.locals]; self.locals],
            state: vec![SiteState::Live; self.locals],
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
                    // Arithmetic on a pointer is not followed. C17 6.5.6 keeps
                    // the result inside the same object, so `p + 1` points at
                    // what `p` points at, and following it would be right;
                    // this does not, because the sibling that reports a use of
                    // a freed value is what would read it and there is no
                    // caller for the precision yet.
                    Rvalue::Use(_) | Rvalue::Unary { .. } | Rvalue::Binary { .. } => {
                        value.clear(destination);
                    }
                    // The address of a place is not an allocation this check
                    // follows: nothing here frees a local's own storage, which
                    // is `StorageDead` and is the lifetime phase's.
                    Rvalue::Address(_) => value.clear(destination),
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
        let touched: Vec<usize> = Self::touching(arguments, value).collect();
        match self.callee(*callee) {
            Callee::Frees => {
                for site in touched {
                    value.state[site] = SiteState::Freed(origin.span());
                }
            }
            // It does not free what it is passed, which is the whole of why the
            // name is read.
            Callee::Allocates => {}
            Callee::Opaque => {
                for site in touched {
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
        value.state[place.local.index()] = SiteState::Live;
    }
}

/// One thing this check concluded, and where.
///
/// Not a diagnostic: this crate cannot see one. What each conclusion costs a
/// build is `Diagnostic::concluded`'s in `safec`, which is the one place that
/// answers it, and ADR-0001 is why there is only one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// What the check concluded about this call.
    pub conclusion: Conclusion,
    /// The call that frees, which is where a caret goes.
    pub at: Span,
    /// The earlier free, where the check knows which one it was.
    ///
    /// `None` for an `Unknown`: what makes it unknown is that the paths
    /// reaching here disagree, so there is no single earlier free to point at.
    pub freed: Option<Span>,
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
                analysis.element(function, element, &mut known);
            }

            // Before the terminator's own transfer, which is what turns a live
            // allocation into a freed one: the question is what was true when
            // the call was reached.
            if let Some(finding) = reported(&analysis, &block.terminator, &known) {
                findings.push(finding);
            }
        }
    }

    findings
}

/// What this terminator is worth reporting, if anything.
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

    // The worst of what the arguments reach. A proved double free outranks one
    // that could not be ruled out, because the two are a different claim rather
    // than a different volume: `docs/safety-model.md` gives the first to an
    // error at every level and leaves the second to `--deny-unknown`.
    let mut freed: Option<Span> = None;
    let mut unknown = false;

    for site in Allocations::touching(arguments, known) {
        match known.state[site] {
            SiteState::Live => {}
            SiteState::Freed(before) => {
                freed = Some(match freed {
                    Some(already) => earlier(already, before),
                    None => before,
                });
            }
            SiteState::Unknown => unknown = true,
        }
    }

    match (freed, unknown) {
        (Some(before), _) => Some(Finding {
            conclusion: Conclusion::Unsafe,
            at: origin.span(),
            freed: Some(before),
        }),
        (None, true) => Some(Finding {
            conclusion: Conclusion::Unknown,
            at: origin.span(),
            freed: None,
        }),
        (None, false) => None,
    }
}
