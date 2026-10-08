//! Whether a program frees one allocation twice, uses one after it was freed,
//! or frees a pointer that is not the start of one.
//!
//! The first checks on [the safety model]'s memory axis, and the first thing
//! this compiler says about what a C program *does* rather than about how it is
//! written.
//!
//! **Five answers out of one walk.** They read one lattice: what a free does to
//! a site is what makes a later use of it a defect, so computing the states
//! twice would be the same computation twice and a second chance for the two
//! copies to disagree. The third asks a free where in its allocation the
//! pointer is, which is ADR-0036, the fourth asks a `return` whether what it
//! hands back was freed, which is ADR-0041, and the fifth asks the same of what
//! a call is handed, which is ADR-0042. [`Kind`] is how the caller tells them
//! apart.
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
//! **One concern per file.** `memory/parts.rs` holds what the lattice value is
//! made of, `known.rs` the value and the walks over it, `built.rs` what a value
//! built out of operands holds, `transfer.rs` the analysis and what each
//! element, terminator and edge does to the value, and `report.rs` what is asked
//! once the walk has settled. This file keeps what a caller names: [`findings`],
//! [`Finding`], [`Kind`], [`Unproven`], and the two readers of a dereference the
//! nullability check shares.
//!
//! [the safety model]: https://github.com/itsakeyfut/safec/blob/main/docs/safety-model.md
//! [`LocalId`]: crate::ir::LocalId

mod built;
mod known;
mod parts;
mod report;
mod transfer;

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve};
use crate::ir::{Element, FuncId, Operand, Place, Rvalue, Terminator, TranslationUnit, Ty};
use crate::nullability::{self};
use crate::source::{SourceMap, Span};

use built::named;
use report::{
    after_a_call, derefs, handed, handed_places, inside, reported, returned, used, used_before,
};
use transfer::Allocations;

/// Which of the five things this check answers about a finding is.
///
/// One walk over one lattice, so this is not five checks and
/// `docs/diagnostics.md` says so where it hands them their codes. What differs
/// is the question: the codes and the words are different, and the thing a
/// caret lands on is a call for a double free, an interior free and an
/// argument after free, a dereference for a use after free, and a `return` for
/// a return after free.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `free(p); free(p);`
    DoubleFree,
    /// `free(p); *p = 42;`
    UseAfterFree,
    /// `free(p + 1);`
    ///
    /// Named for what it catches and not for every invalid free: this is a
    /// pointer into an allocation this check followed, and `free(17)` or a
    /// free of a local's address is not reported under it. See ADR-0036.
    InteriorFree,
    /// `free(p); return p;`
    ///
    /// One of the two reads of a pointer without a dereference this check asks
    /// about, because a caller believes what it is handed is live and nothing
    /// reads this function's body from there. See ADR-0041.
    ReturnAfterFree,
    /// `free(p); g(p);`
    ///
    /// The other, the same belief arriving by the other door: a function's body
    /// believes its pointer parameters live where it starts, and nothing reads
    /// its callers from there. See ADR-0042.
    ArgumentAfterFree,
    /// `free(a); g(&a);`
    ///
    /// Not [`Kind::ArgumentAfterFree`], because what is handed over is live:
    /// it is what it points at that holds a freed pointer, which the callee
    /// may read and use. Never a proof, since it may only write there. See
    /// ADR-0042.
    FreedBehindArgument,
}

/// One thing this check concluded, and where.
///
/// Not a diagnostic: this crate cannot see one. What each conclusion costs a
/// build is `Diagnostic::concluded`'s in `safec`, which is the one place that
/// answers it, and ADR-0001 is why there is only one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The function it was concluded in.
    ///
    /// What lets a driver tell a conclusion about a hatch from one about the
    /// program. The span alone cannot say, because nothing here maps a span
    /// back to the function whose body holds it. See ADR-0038.
    pub function: FuncId,
    /// Which of the two this is.
    pub kind: Kind,
    /// What that check concluded.
    pub conclusion: Conclusion,
    /// Where a caret goes: the call that frees or is handed a freed pointer, or
    /// the element that reads or writes through one, or returns one.
    ///
    /// Not the place's own span, which a [`Place`] does not have: the element's
    /// or the terminator's.
    pub at: Span,
    /// The earliest free reaching here, where the check knows which one it was.
    ///
    /// `None` for most `Unknown`s: what usually makes one unknown is that the
    /// paths or the sites reaching here disagree, so there is no single free to
    /// point at. [`Unproven::Unsequenced`] is the exception, where there is one
    /// and what is open is the order.
    pub freed: Option<Span>,
    /// Where the allocation was made, where this check saw it happen.
    ///
    /// `None` for a parameter, whose allocation is a caller's, and for an
    /// `Unknown` for the reason above. `docs/safety-model.md` asks the
    /// diagnostic for this line and it is honest to leave it off rather than
    /// point at an allocation that may not be the one.
    pub made: Option<Span>,
    /// Why this could not be proven, and `None` where it was.
    ///
    /// One reason rather than a flag per reason. The words a reader is given
    /// differ by reason, so this is what chooses them, and two flags beside
    /// each other would have combinations that mean nothing with only a doc
    /// comment to say so. `Some` exactly where [`Self::conclusion`] is
    /// [`Conclusion::Unknown`].
    ///
    /// **That last sentence is held by nothing.** Every `Verdict` is built
    /// in one function and every [`Finding`] out of a `Verdict`, so the two
    /// fields agree by being written together rather than by a rule anything
    /// checks. A mutation cannot show it either: the arm that would answer for
    /// a reason beside a proof is unreachable.
    pub unproven: Option<Unproven>,
}

/// Why this check stopped following a pointer, as far as the producer of
/// [`Unproven::Lost`] can tell, so that a reader is told what to do about
/// it rather than only that something was lost.
///
/// **As far as the producer can tell, and no further.** The one bit a
/// local carries for having lost what it held, `Held::lost`, is set by
/// several causes the lattice does not keep apart, so a pointer lost that
/// way is [`Self::Other`]. Telling them apart would be a set in the
/// lattice, with its own join and height (#213).
///
/// **No span.** None of these producers has one in the lattice, and a
/// variant with a field it could not fill would be a claim nothing
/// established, so the type has nowhere to put one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LostReason {
    /// Read out of memory: an argument written through a projection,
    /// `free(*pp)`, or a pointer a load gave. This check follows locals,
    /// not what memory holds.
    ReadOutOfMemory,
    /// A pointer this check never followed to any allocation: a local
    /// reaching no site, set where this check cannot see, such as by a
    /// call through its address.
    NeverFollowed,
    /// What may be a local's address on some path, which `free` must never
    /// be handed (C17 7.22.3.3 p2).
    MayBeALocal,
    /// Any other cause, `Held::lost` among them, or more than one reason
    /// at once.
    Other,
}

impl LostReason {
    /// One reason for two: itself where they agree, [`Self::Other`] where
    /// they do not, since naming either would be wrong about the other.
    pub(crate) fn joined(self, other: Self) -> Self {
        if self == other { self } else { Self::Other }
    }
}

/// Why a finding could not be proven.
///
/// **Four reasons and not two, because one of them is about this check and
/// the others are about the program.** A reader told a value may have been
/// freed already is being told something was established somewhere; where this
/// check lost the pointer, nothing was, and saying it anyway is a claim about
/// a program that nobody worked out. Which words each reason gets is
/// `memory_finding`'s in `safec`, for the reason [`Finding`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unproven {
    /// The paths or the sites reaching here disagree about a site that really
    /// was freed, or a call this check cannot read was handed one, or could
    /// reach one because it was exposed, and may have freed it. There is a free
    /// to suspect, and no single one to point at. See ADR-0039.
    Disagreement,
    /// This check stopped following the pointer, so nothing here established a
    /// free at all.
    ///
    /// `Reached::Lost` with nothing else contributing, and it takes the same
    /// word as that variant because it is the same fact reaching the reader.
    /// Every producer sits in one of two functions, which is worth writing out
    /// because the ones a reader meets first are not all of them.
    /// `Known::reached_by` answers it for a local that held a site and lost
    /// the name for it, which is ADR-0018 and which a call this check cannot
    /// read also produces, ADR-0029; and for one whose address escaped, which
    /// is ADR-0017. `Allocations::touching` answers it for an argument
    /// written through a projection, `free(*pp)`, and for one whose local
    /// reaches no site at all. None of them says a free happened; each says
    /// this check can no longer say what the pointer points at.
    ///
    /// Those are private, so they are named here rather than linked: a link
    /// out of a public item to one of them is
    /// `rustdoc::private_intra_doc_links`, which this crate denies.
    Lost(LostReason),
    /// C has not said which order runs. See ADR-0022.
    Unsequenced,
    /// A pointer this check followed was moved by something it cannot
    /// evaluate, so whether it is still at the start of its allocation is not
    /// known. Only [`Kind::InteriorFree`] carries it. See ADR-0036.
    Offset,
}

/// Every double free this unit contains, and every one it cannot rule out.
///
/// One walk per function: the fixpoint answers what holds where each block
/// starts, and this replays each block from there to find the calls to report.
/// The replay rather than a second lattice, because the transfer is what
/// decides which sites a call touches and having two answers to that is having
/// one of them be wrong.
pub fn findings(sources: &SourceMap, unit: &TranslationUnit) -> Vec<Finding> {
    let mut findings = Vec::new();

    for func in unit.functions() {
        let function = unit.function(func);
        // A declaration has no blocks, and `Function::blocks` panics rather
        // than answering for one.
        if !function.is_defined() {
            continue;
        }

        let mut analysis = Allocations {
            sources,
            unit,
            function: func,
            locals: function.locals().len(),
            parameters: function.parameters().collect(),
            exposed_parameters: function
                .parameters()
                .filter(|_| sources.snippet(function.name) != "main")
                // A `match` for `Allocations::is_pointer`'s reason: a kind of
                // type added later is asked whether it is exposed.
                .filter(|&local| match unit.ty(function.local(local)) {
                    Ty::Pointer(_) => true,
                    Ty::Int | Ty::Char | Ty::Void => false,
                })
                .collect(),
            live_in: transfer::live_in(function),
            opaque_results: Vec::new(),
        };
        analysis.opaque_results = analysis.opaque_results_of(function);
        let cfg = Cfg::of(function);
        let solution = solve(&analysis, function, &cfg);
        // The other check's answer, asked at each block's terminator, which is
        // where a free is reached. A free of a pointer that check established
        // is null frees nothing, and `asked` is the one place that is applied.
        // See ADR-0027.
        let null = nullability::null_at_terminators(unit, function, &cfg);

        // Per function, because a span belongs to one of them. The third
        // field is where in `findings` the report standing at that caret is,
        // so that a proof arriving later can replace a suspicion: `findings`
        // is appended to and assigned into, never removed from or reordered,
        // until the sort at the end of this function.
        let mut said: Vec<(Span, Place, usize)> = Vec::new();

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
                    func,
                );
                // After the dereferences in the element, as `handed` is after
                // the ones at a call, so that `return *tab;` keeps a doubt
                // about reading `*tab` over one about what it returns, as
                // `release(*tab)` does. Before the transfer, for the reason
                // above: what is returned is what was held as the write ran.
                returned(
                    &mut findings,
                    &mut said,
                    &analysis,
                    function,
                    element,
                    &known,
                );
                // Just before what is pending is cleared, so that a free in
                // the same expression has reported first, with its label.
                if matches!(
                    element,
                    Element::Sequenced { .. } | Element::ArgumentsEvaluated { .. }
                ) {
                    after_a_call(&mut findings, &mut said, &known, func);
                }
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
                func,
            );
            // After the dereferences at this call, so a caret carrying both
            // reads `SC0402` above `SC0407`, and before the transfer, for the
            // reason above.
            handed(
                &mut findings,
                &mut said,
                &analysis,
                function,
                &block.terminator,
                &known,
            );
            findings.extend(reported(&analysis, &block.terminator, &known, &null, id));
            // After the free's own finding, which is the one whose caret is
            // here: what this adds are reports about carets further back, and a
            // reader meets them in the order the sort at the end puts them in
            // rather than the order they were made. Still before the transfer,
            // because what it asks about is what had been read when the call
            // was reached.
            used_before(
                &mut findings,
                &mut said,
                &analysis,
                &block.terminator,
                &known,
                &null,
                id,
            );
            if matches!(block.terminator, Terminator::Return) {
                after_a_call(&mut findings, &mut said, &known, func);
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

/// Where this element runs, and every place it reads or writes through a
/// pointer there.
///
/// Through a pointer, so a projection: an unprojected place is the local itself
/// and holding a freed pointer is not using it. The span is the element's,
/// because a [`Place`] has none of its own.
pub(crate) fn dereferenced_in_element(element: &Element) -> Option<(Span, Vec<&Place>)> {
    // Every field written out, never `..`, which would let a field added to a
    // variant that already exists walk past an exhaustive match.
    match element {
        Element::Assign(operation) => {
            let mut places = projected(&operation.place);
            places.extend(dereferenced_in_rvalue(&operation.value));
            Some((operation.origin.span(), places))
        }
        // The element that exists so this can see it. `projected` rather than
        // the place itself, even though `Element::Evaluate`'s doc says a
        // producer owes a projection: one rule about what counts as reaching
        // through a pointer, applied everywhere, beats two that agree today.
        Element::Evaluate { place, origin } => Some((origin.span(), projected(place))),
        // Neither marker, nor storage beginning or ending, reads anything
        // through anything.
        Element::Sequenced { origin: _ } | Element::ArgumentsEvaluated { origin: _ } => None,
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
pub(crate) fn dereferenced_in_terminator(terminator: &Terminator) -> Option<(Span, Vec<&Place>)> {
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
        // **A condition is read here and not from an element, because a
        // condition that is exactly a place never becomes one.** `if (*p + 1)`
        // computes into a temporary and the `Operation` that does it carries
        // the dereference; `if (*p)` hands the place straight to the
        // terminator. Both are one defect and the difference is whether the
        // expression needed a temporary, which is not something a reader could
        // predict, so this arm is what makes the answer the same for both.
        Terminator::Branch {
            condition,
            then: _,
            otherwise: _,
            origin,
        } => Some((origin.span(), dereferenced_in(condition))),
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
        // **Taking an address is not a dereference**, whatever the place
        // it is taken of looks like. C17 6.5.3.2 p3 is why `&*p` used to be
        // the example: "neither that operator nor the `&` operator is
        // evaluated and the result is as if both were omitted". The lowering
        // applies that clause now, so no C program reaches here with one, and
        // what does reach here is an address of a place a frontend really
        // meant to take. Reporting it would be a use of a freed value in a
        // program that never touched one.
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
