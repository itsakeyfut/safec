//! Whether a pointer can be null, and what a dereference of one is worth.
//!
//! `docs/safety-model.md` lists "Can `p` be NULL?" among the questions
//! ordinary C does not answer, and this is the analysis that asks it. A
//! dereference of a place whose local is known null is [`Conclusion::Unsafe`];
//! of one that is neither known null nor known not null, it is
//! [`Conclusion::Unknown`]; of one known not null, it is nothing at all.
//!
//! **The second analysis written against [`crate::dataflow::Analysis`]**, and
//! it follows [`crate::memory`] rather than inventing a shape: a fixpoint over
//! each function, then a replay of each block from its entry value to find the
//! elements to report. [`crate::analysis`] says what an analysis module's entry
//! point is called and why each owns its own [`Finding`].
//!
//! **A `malloc` result is not known to be anything.** C17 7.22.3.4 p3 says the
//! function returns either a null pointer or a pointer to the allocated space,
//! so the roadmap's own headline example, `int *p = malloc(sizeof(int)); *p =
//! 42;`, is a warning here until something tests `p`. Answering that it is not
//! null would be this compiler naming a safety it has not established about the
//! program where the allocation failed, which `docs/safety-model.md` calls the
//! worst answer available. The state that says so is private, so it is named
//! here rather than linked, which is what this crate's other check does with
//! its own.
//!
//! **A `_Nonnull` parameter is believed by its body and checked at every
//! call.** It enters the lattice not null, and every argument passed to one is
//! asked what a dereference is asked. What is believed is only what no call in
//! the translation unit reaches, which is ADR-0037.
//!
//! **A function that promises the pointer it returns is not null is asked at
//! every `return`, and a call believes it.** The promise is
//! [`crate::ir::Function::promised`], written `_Nonnull` or made by level 5's
//! default, and this check reads only that it was made (ADR-0050).
//!
//! **Nothing here reports.** This builds a [`Conclusion`] and a span; `safec`
//! turns one into a diagnostic, because this crate cannot see one, which is
//! ADR-0011.

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve};
use crate::ir::{
    BinOp, BlockId, Element, FuncId, Function, LocalId, Operand, Place, Projection, Promise,
    Rvalue, Terminator, TranslationUnit, Ty, UnOp,
};
use crate::memory::{dereferenced_in_element, dereferenced_in_terminator};
use crate::source::Span;

/// What is known about whether a pointer is null.
///
/// **Three rather than two.** A pointer nothing has said anything about is not
/// known null and is not known not null, and `docs/safety-model.md`'s whole
/// subject is that the third answer is expressible rather than rounded to one
/// of the others.
///
/// [`Self::Unknown`] is the top: two arms that disagree meet there, and
/// nothing rises above it, which is what bounds [`Nullability::height`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nullness {
    /// Known null. A dereference is a fact about the program.
    Null,
    /// Known not null. A dereference is nothing to say.
    NonNull,
    /// Neither established.
    Unknown,
}

impl Nullness {
    /// Where two paths meet.
    ///
    /// Equal stays and different rises, which is the only shape that has a top:
    /// `Null` and `NonNull` are incomparable, so their meet is the thing above
    /// both.
    fn joined(self, other: Self) -> Self {
        if self == other { self } else { Self::Unknown }
    }

    /// The same fact about the other arm of the branch that established it.
    fn inverted(self) -> Self {
        match self {
            Self::Null => Self::NonNull,
            Self::NonNull => Self::Null,
            // A test that told one arm nothing tells the other nothing.
            Self::Unknown => Self::Unknown,
        }
    }

    /// What a dereference of a place whose local holds this is worth.
    ///
    /// **`None` means proved and nothing else.** The memory check once had an
    /// arm that meant "proved safe" and "gave up" at once, and
    /// reported the second as the first; here the giving up has a variant of
    /// its own that is reported, so the only thing that reaches `None` is a
    /// pointer this analysis established is not null.
    fn concluded(self) -> Option<Conclusion> {
        match self {
            Self::Null => Some(Conclusion::Unsafe),
            Self::Unknown => Some(Conclusion::Unknown),
            Self::NonNull => None,
        }
    }
}

/// One dereference, one argument, or one `return`, this check concluded about,
/// and where.
///
/// Not a diagnostic: this crate cannot see one. What each conclusion costs a
/// build is `Diagnostic::concluded`'s in `safec`, which is the one place that
/// answers it, and ADR-0001 is why there is only one.
///
/// **One span and not two.** `docs/safety-model.md` asks the memory axis's
/// diagnostic for three positions and asks this one for none, and a second span
/// is not free: a value carrying one does not reach a fixpoint unless its
/// payload is chosen by a rule rather than by which side arrived, which is
/// ADR-0016 and was measured here as a hang. Saying where a pointer became null
/// waits for something that needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The function it was concluded in, for the reason
    /// [`crate::memory::Finding::function`] gives.
    pub function: FuncId,
    /// What this check concluded about the pointer.
    pub conclusion: Conclusion,
    /// Where a caret goes: the element or terminator that dereferences, the
    /// call that passes the argument, or the `return` that hands the pointer
    /// back.
    pub at: Span,
    /// Which question the conclusion answers.
    pub asked: Asked,
}

/// Which question a [`Finding`] answers.
///
/// Three, because they are three different programs to fix and three different
/// codes for a reader to search for: a read through a pointer that may be
/// null, a pointer that may be null handed to a parameter that promised it is
/// not, and one handed back by a function that promised it is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// Whether a pointer read or written through is null.
    Dereference {
        /// Whether a pointer this goes through was read out of memory, as the
        /// one `**pp` reads through second is.
        ///
        /// Not a different question, and not a different code: what differs is
        /// the remedy. A test of `*pp` is kept nowhere, because this lattice
        /// has a row per local and none for `*pp`, so telling the reader to
        /// test the pointer is advice to do what may already have been done.
        /// What settles it is reading the pointer into a local and testing
        /// that (#333).
        through_memory: bool,
    },
    /// Whether an argument passed to a `_Nonnull` parameter is null.
    Argument {
        /// What the parameter promised, which is what this argument is
        /// checked against and where a report points.
        promise: Promise,
    },
    /// Whether the pointer a function returns is null, where it promised not.
    Return {
        /// What the function promised, which is where a report points the
        /// reader who decides the promise was wrong.
        promise: Promise,
        /// Whether the `return` is the end of the body, reached on a path
        /// that wrote nothing to return.
        ///
        /// Its own answer, because the caller would believe whatever the
        /// return place holds and there is no `return` to point at: the caret
        /// goes on the function's name and the reader is told the body can
        /// end without one. C17 6.9.1 p12 makes using that value undefined.
        reached_end: bool,
    },
}

impl Asked {
    /// Whether two findings at one caret answer one question, so that one of
    /// them is enough to say.
    ///
    /// A dereference is one question whether or not it went through memory:
    /// `*p && **q` writes both operands at one span, and the reader is told
    /// one thing about it. An argument is a question per promise, and a return
    /// is one per promise and per whether it is the end of the body.
    ///
    /// The first element of every pair written out, so that a fourth question
    /// is answered for here by `error[E0004]`.
    fn same_question(self, other: Self) -> bool {
        match (self, other) {
            (Self::Dereference { through_memory: _ }, Self::Dereference { through_memory: _ }) => {
                true
            }
            (Self::Argument { promise }, Self::Argument { promise: other }) => promise == other,
            (
                Self::Return {
                    promise: _,
                    reached_end: _,
                },
                Self::Return {
                    promise: _,
                    reached_end: _,
                },
            ) => self == other,
            (
                Self::Dereference { through_memory: _ }
                | Self::Argument { promise: _ }
                | Self::Return {
                    promise: _,
                    reached_end: _,
                },
                _,
            ) => false,
        }
    }

    /// One question out of two that [`Asked::same_question`] calls the same.
    ///
    /// A dereference that went through memory in either keeps saying so,
    /// because the remedy the reader is given has to settle both: the one
    /// written for a pointer read out of memory covers the other pointer too,
    /// and the plain one does not cover it.
    fn joined(self, other: Self) -> Self {
        match (self, other) {
            (
                Self::Dereference { through_memory },
                Self::Dereference {
                    through_memory: other,
                },
            ) => Self::Dereference {
                through_memory: through_memory || other,
            },
            (asked, _) => asked,
        }
    }
}

/// Which locals are known null, known not null, or neither.
///
/// **Keyed by the local rather than by the place.** [`Place`]'s own doc comment
/// asks a real analysis for a place, and that is right about the memory axis,
/// where `p` and `*p` have separate states. The question here is about the
/// pointer value a local holds, so the local is the key, and what it costs is
/// that the pointer `*pp` holds answers [`Nullness::Unknown`], however it is
/// reached: copied into a local, or dereferenced in place as `**pp`, which
/// [`report`] answers because no row here can. A warning on correct C rather
/// than silence about it. It also makes [`Analysis::height`] the local
/// count, read straight off the function, which is an answer the trait asks
/// each analysis for rather than guessing one on its behalf.
struct Nullability<'a> {
    /// What a local's type is, so that a branch on an `int` is not read as a
    /// branch on a pointer.
    unit: &'a TranslationUnit,
    /// The function this is about, for which of its parameters were declared
    /// `_Nonnull`.
    function: &'a Function,
    /// How many locals the function has, for the value's length.
    locals: usize,
    /// Which locals have their address taken anywhere in the function.
    ///
    /// **Nothing this lattice establishes about such a local is believed**, and
    /// that is what stops this check being quiet about a dereference it did not
    /// prove. A store through a pointer writes an object the pointer names, and
    /// **in this IR** the only way a pointer can come to name a local is for
    /// the local's address to have been taken, so a local whose address is
    /// never taken cannot be written except where this check can see it. One
    /// whose address *is* taken
    /// can be written by a store this check cannot follow, by a callee it
    /// cannot read, or by either on a path it is not on, and a positive claim
    /// that survives one of those is [`docs/safety-model.md`]'s worst answer:
    /// measured, `int *p = &x; int **pp = &p; *pp = 0; *p = 1;` said nothing at
    /// all before this field existed.
    ///
    /// **Read where it is used rather than written into the value.** The escape
    /// is a property of the whole function rather than of a point in it, so it
    /// needs no lattice dimension, no join and no extra height, and there is no
    /// list of places a write can happen for it to be missing one of. That is
    /// the shape ADR-0017 arrived at for the memory axis after the other one
    /// cost it a silent double free.
    ///
    /// **"In this IR" is load-bearing and is not a claim about C.** C17 6.3.2.1
    /// p3 converts an array to a pointer to its first element with no `&`
    /// anywhere, so `int a[2]; int *q = a; *q = 0;` names and writes a local
    /// this scan would call unescaped. `Ty` has no array and the frontend
    /// refuses one, measured, so there is no such door today; the day there is,
    /// this is what has to answer for it. A function-scope `static` and a
    /// `volatile` local are the same shape and are refused for the same
    /// reason.
    ///
    /// [`docs/safety-model.md`]: https://github.com/itsakeyfut/safec/blob/main/docs/safety-model.md
    escaped: Vec<bool>,
}

/// Which locals have their address taken anywhere in this function.
///
/// The whole function rather than the path, deliberately: a walk that marked a
/// local escaped only from the `Rvalue::Address` onwards would believe a claim
/// made above one, and a loop puts the write before the escape in the order
/// this walks even when the program runs them the other way round.
fn escaped_in(function: &Function) -> Vec<bool> {
    let mut escaped = vec![false; function.locals().len()];

    let mut taken = |value: &Rvalue| {
        if let Rvalue::Address(place) = value {
            escaped[place.local.index()] = true;
        }
    };

    for block in function.blocks() {
        for element in &block.elements {
            if let Element::Assign(operation) = element {
                taken(&operation.value);
            }
        }
    }

    escaped
}

impl Nullability<'_> {
    /// What is known about this local, which is nothing at all if its address
    /// has escaped.
    ///
    /// Every read of the value goes through here. [`Self::escaped`] says why,
    /// and why it is applied at the read rather than propagated.
    ///
    /// **This is the mask that keeps this check out of the bottom row**, so it
    /// is worth saying what holds it: having it read the value rather than the
    /// escape fails `a_pointer_written_through_its_own_address_is_not_proved`
    /// and nothing else.
    fn known(&self, value: &[Nullness], local: LocalId) -> Nullness {
        if self.escaped[local.index()] {
            Nullness::Unknown
        } else {
            value[local.index()]
        }
    }

    /// Whether this local holds a pointer.
    ///
    /// `if (p)` and `if (x)` lower to the same terminator, and only the first
    /// says anything about a pointer. Without this, the temporary a `&&`
    /// computes into would be refined as though it were the pointer under it.
    ///
    /// **Something holds this now, and it did not used to.** For the branch this
    /// is written beside, nothing does: making it answer `true` for everything
    /// left the whole workspace green, because what it prevents there is a
    /// non-pointer local being given a nullness and a non-pointer local is
    /// never dereferenced in well-formed IR, so no report moved. It stayed on
    /// the ground that a value whose states are about pointers should not be
    /// written about things that are not pointers.
    ///
    /// [`null_at_terminators`] is the caller that made that ground
    /// load-bearing: a nullness the memory check reads decides whether a `free`
    /// is reported at all, so an `int` holding zero being called null is a
    /// diagnostic that disappears. Mutation: drop the call there.
    /// `a_free_of_an_int_that_holds_zero_is_not_exempt` fails, and nothing else
    /// in the suite moves.
    fn is_pointer(&self, function: &Function, local: LocalId) -> bool {
        matches!(self.unit.ty(function.local(local)), Ty::Pointer(_))
    }
}

/// Which local a branch tested against null, and what its `then` arm learns.
///
/// **A function, not a method, because two checks read it**: this one, and the
/// memory check, which learns from the same branch what a `realloc` did to the
/// allocation it was handed. One reader, so the two cannot disagree about which
/// local a branch tested. See ADR-0039.
///
/// Two shapes reach here and the difference is not something a reader of
/// the C could predict, so both are answered:
///
/// - `if (p)` hands the pointer's own place to the terminator. The branch
///   reads that pointer where it runs, so whatever wrote it before, `int *q
///   = r; if (q)` included, the refinement is about the value tested, and it
///   is answered before the walk. Walking to the last write first answered a
///   copy with nothing, which doubted `if (q)` where `q != 0` was proved
///   (#334).
/// - `if (p != 0)` writes the comparison to a temporary and hands a copy of
///   that, so the comparison is an element of this block, above the
///   terminator. Reaching it is what [`Analysis::edge`]'s block is for, and
///   the walk below is for this shape alone. A comparison's result is an
///   `int`, which is how the two are told apart.
///
/// Either way the answer is one local: the one the branch read in the first
/// shape and the one the comparison read in the second. For `if ((q = p))`
/// and `if ((q = p) != 0)` that is a temporary copied from `q`, and carrying
/// the refinement back to `q` and `p` is [`copied_from`]'s, called from
/// [`Analysis::edge`] alone, so what the memory check reads here is the same
/// with it or without it.
///
/// **A local something below the comparison may have changed is not
/// refined.** The walk records every local it steps over a write to, and
/// refuses the refinement if the comparison turns out to be about one of
/// them, because refining on a value the program has since overwritten is a
/// fact about a pointer that is no longer there. A store through a
/// projection is the one element that names no local, so it says every
/// local may have changed and the walk gives up where it stands.
///
/// **The refusal is per local rather than positional**, and that is not a
/// refinement of taste. Stopping the walk at the first write to anything
/// refuses a comparison whose branch is reached past a write to some other
/// local, which changed nothing the comparison read. This frontend never
/// lowers a write between a comparison and its branch, so the shape is
/// another frontend's, and
/// `a_write_to_another_local_between_a_comparison_and_its_branch_keeps_the_refinement`
/// in `crates/safec-ir/tests/nulls.rs` holds it. It used to be argued from
/// `int x = 5; if (p) { *p = x; }`, which the positional rule did refuse,
/// and nothing failed when it was applied: no case held that program, and
/// it no longer reaches the walk at all (#334).
pub(crate) fn tested_against_null(
    unit: &TranslationUnit,
    function: &Function,
    block: BlockId,
    condition: &Place,
) -> Option<(LocalId, Nullness)> {
    // `if (*p)` tests what `p` points at, which says nothing about `p` on
    // either arm. That the dereference happened is recorded by
    // `Analysis::terminator`, which is a different fact.
    //
    // **This check never sees the difference**: that record has settled `p`
    // not null before `Analysis::edge` asks, and a settled local is left
    // alone. The reader that does is the memory check, which reads a branch
    // on what a `realloc` returned with nothing settled in front of it, and
    // would take the arm where `*q` is zero for the call failing.
    // `a_branch_on_what_reallocs_result_points_at_says_nothing_about_the_call`
    // loses its double free without this line.
    if !condition.projection.is_empty() {
        return None;
    }
    // The first shape: the branch tests the pointer itself, so it is refined
    // whatever wrote it. The type is asked rather than assumed, because a
    // short-circuited `&&` or `||` branches on an `int` temporary holding a
    // comparison, which is the second shape and the walk's to read.
    if pointer_typed(unit, function, condition.local) {
        return Some((condition.local, Nullness::NonNull));
    }

    // Which locals something below the comparison may have changed. A
    // refinement about one of them is about a value the branch no longer
    // reads, and refusing it is the whole of this walk's give-up rule.
    let mut changed = vec![false; function.locals().len()];

    for element in function.block(block).elements.iter().rev() {
        // Every variant written out, and every field with it, for the
        // reason `Analysis::element` gives: an exhaustive match that
        // writes `..` lets a field added to a variant that already exists
        // walk past it. The question here is whether anything
        // below the comparison replaced what it read, so an element kind
        // added later is exactly the thing that would have to answer.
        let operation = match element {
            Element::Assign(operation) => operation,
            // None of them computes into a local, so none can have changed
            // one, and there is nothing to record. **One arm rather than
            // three**, so that this is one behaviour with one guard,
            // `a_marker_between_a_comparison_and_its_branch_is_stepped_over`;
            // split into three, two of them would be held by nothing.
            // Giving up here instead would cost a refinement wherever a
            // marker separates a comparison from its branch, which is a
            // false positive on each of them.
            Element::Evaluate {
                place: _,
                origin: _,
            }
            | Element::Sequenced { origin: _ }
            | Element::ArgumentsEvaluated { origin: _ } => continue,
            // Storage ending takes the object the comparison read away and
            // storage beginning brings a different one, so either leaves
            // the local holding something the comparison never saw. That
            // is a change like any other and is recorded like one.
            // `Analysis::element` answers `Nullness::Unknown` for both and
            // `Analysis::edge` refines only a local that is `Unknown`, so
            // the two do not cancel out: without this they would combine
            // into a refinement about an object that is gone. One arm
            // again, for the reason above.
            Element::StorageLive { local, origin: _ }
            | Element::StorageDead { origin: _, local } => {
                changed[local.index()] = true;
                continue;
            }
        };

        // A store through a projection may have landed in any local,
        // because nothing in such an element says which one it lands in, so
        // there is no single local to record and the walk cannot carry on.
        if !operation.place.projection.is_empty() {
            return None;
        }
        // A direct store says exactly which local it changed, so the walk
        // carries on and the answer below is refused only if it turns out
        // to be about this one.
        if operation.place.local != condition.local {
            changed[operation.place.local.index()] = true;
            continue;
        }

        // The last write to the condition's local, whatever it is. A
        // comparison against zero is the one shape this reads; anything
        // else, `int c = p != 0;` among them, is a value this analysis
        // cannot follow, and refining on a condition it did not read is
        // the one direction a refinement must not be wrong in.
        return (match &operation.value {
            Rvalue::Binary { op, lhs, rhs } => compared_to_null(unit, function, *op, lhs, rhs),
            // C17 6.5.3.3 p5: "The expression `!E` is equivalent to
            // `(0==E)`." So `if (!p)` is `if (p == 0)` written shorter, and
            // a reader who cannot tell those apart should not be given two
            // answers. The other two operators say nothing about null.
            Rvalue::Unary {
                op: UnOp::Not,
                operand,
            } => compared_to_null(unit, function, BinOp::Eq, operand, &Operand::Constant(0)),
            Rvalue::Unary {
                op: UnOp::Neg | UnOp::BitNot,
                operand: _,
            } => None,
            Rvalue::Use(_) | Rvalue::Address(_) => None,
        })
        // **The refusal, and the only one this walk makes about a local it
        // can name.** The comparison read this local above; everything
        // between here and the branch has been recorded, so a local in
        // that set is one the branch no longer reads the compared value
        // from.
        .filter(|(local, _)| !changed[local.index()]);
    }

    // Nothing in this block wrote a condition that is not a pointer, so there
    // is no comparison here for this walk to read.
    //
    // **Nothing holds this `None`, and nothing can.** Answering such a
    // condition non-null instead leaves the suite green, because an `int` is
    // never read through, and the memory check acts on a branch only for a
    // local holding exactly what a `realloc` returned, which an `int` does
    // not, so no report moves. It is the answer that is right about the
    // branch rather than one a test can tell apart.
    None
}

/// The locals a branch's tested local was copied from, nearest first, where
/// each still holds what was copied out of it when the branch reads.
///
/// `if ((q = p))` is the shape. C17 6.5.16 p3 gives an assignment
/// expression the value of its left operand after the assignment, so the
/// lowering writes `q`, copies it into a temporary, and branches on the
/// temporary, which is the local [`tested_against_null`] answers. Nothing
/// reads that temporary again, so a refinement kept on it alone is lost,
/// while `q` and `p` held the same value when the branch ran and are what
/// the arm reads. A chain, `if ((r = q = p))`, is followed copy by copy.
///
/// **Only a copy whose source nothing below it may have changed is
/// followed**, by the rule `tested_against_null` keeps and with the same
/// record: a direct store names the local it changed, and a store through
/// a projection names none, so the walk stops there with what it has. A
/// copy below such a store is still followed, because nothing the store
/// can reach changes which value the copy took. The walk stays in the
/// branch's block, and reaching the block's start loses a refinement
/// rather than making a wrong one. A call or a `?:` in the condition ends
/// a block, and the assignment and the copy of it still land after it,
/// beside the branch: `while ((q = next(q)))` and `if ((q = c ? p : r))`
/// both refine `q`. An `&&` or `||` branches on the `int` it computed, which
/// is not a copy of anything, so `if (x && (q = p))` refines nothing here.
fn copied_from(function: &Function, block: BlockId, local: LocalId) -> Vec<LocalId> {
    let mut changed = vec![false; function.locals().len()];
    let mut following = local;
    let mut sources = Vec::new();

    for element in function.block(block).elements.iter().rev() {
        // Every variant and field written out, for the reason
        // `tested_against_null`'s walk gives, and each answered as it is
        // there.
        let operation = match element {
            Element::Assign(operation) => operation,
            Element::Evaluate {
                place: _,
                origin: _,
            }
            | Element::Sequenced { origin: _ }
            | Element::ArgumentsEvaluated { origin: _ } => continue,
            // Storage beginning or ending is a write like any other, and for
            // the local being followed it is the last one: what that local
            // held before is a different object's, so nothing above it is
            // followed.
            Element::StorageLive { local, origin: _ }
            | Element::StorageDead { origin: _, local } => {
                if *local == following {
                    return sources;
                }
                changed[local.index()] = true;
                continue;
            }
        };

        if !operation.place.projection.is_empty() {
            return sources;
        }
        if operation.place.local != following {
            changed[operation.place.local.index()] = true;
            continue;
        }

        // The last write to the local being followed. A copy is the one
        // shape that leaves two locals holding one value; anything else
        // computed it, and there is nothing further to follow.
        match &operation.value {
            Rvalue::Use(Operand::Copy(source))
                if source.projection.is_empty() && !changed[source.local.index()] =>
            {
                sources.push(source.local);
                following = source.local;
            }
            Rvalue::Use(_)
            | Rvalue::Address(_)
            | Rvalue::Unary { op: _, operand: _ }
            | Rvalue::Binary {
                op: _,
                lhs: _,
                rhs: _,
            } => return sources,
        }
    }

    sources
}

/// The local an equality against a null pointer constant names, and what
/// the `then` arm learns about it.
///
/// `p != 0` and `0 != p` are one program, so both orders are read. C17
/// 6.3.2.3 p3 makes an integer constant expression with the value 0 a null
/// pointer constant, which is what the lowering leaves here.
fn compared_to_null(
    unit: &TranslationUnit,
    function: &Function,
    op: BinOp,
    lhs: &Operand,
    rhs: &Operand,
) -> Option<(LocalId, Nullness)> {
    let local = match (lhs, rhs) {
        (Operand::Copy(place), Operand::Constant(0))
        | (Operand::Constant(0), Operand::Copy(place))
            if place.projection.is_empty() =>
        {
            place.local
        }
        _ => return None,
    };

    if !pointer_typed(unit, function, local) {
        return None;
    }

    // Every operator written out rather than `_`, so that one added later
    // has to answer here rather than pass as a case nobody had thought
    // about.
    match op {
        BinOp::Ne => Some((local, Nullness::NonNull)),
        BinOp::Eq => Some((local, Nullness::Null)),
        BinOp::Mul
        | BinOp::Div
        | BinOp::Rem
        | BinOp::Add
        | BinOp::Sub
        | BinOp::Shl
        | BinOp::Shr
        | BinOp::Lt
        | BinOp::Gt
        | BinOp::Le
        | BinOp::Ge
        | BinOp::BitAnd
        | BinOp::BitXor
        | BinOp::BitOr => None,
    }
}

/// Whether a local's type is a pointer, so that a branch on an `int` is not
/// read as a branch on a pointer.
fn pointer_typed(unit: &TranslationUnit, function: &Function, local: LocalId) -> bool {
    matches!(unit.ty(function.local(local)), Ty::Pointer(_))
}

/// What an rvalue is worth as a pointer.
///
/// Read against the value that holds where the operation runs, so that a copy
/// of a local carries what that local was established to be.
impl Nullability<'_> {
    fn nullness_of(&self, value: &Rvalue, known: &[Nullness]) -> Nullness {
        match value {
            // `int *p = 0;`. C17 6.3.2.3 p3: an integer constant expression with
            // the value 0 is a null pointer constant.
            Rvalue::Use(Operand::Constant(0)) => Nullness::Null,
            // Any other constant reaching a pointer needs a cast this subset does
            // not have, so nothing is claimed about it.
            Rvalue::Use(Operand::Constant(_)) => Nullness::Unknown,
            Rvalue::Use(Operand::Copy(place)) => {
                if place.projection.is_empty() {
                    self.known(known, place.local)
                } else {
                    // What `*pp` holds is a question about a place rather than a
                    // local, which this lattice's key cannot ask.
                    Nullness::Unknown
                }
            }
            // The address of an object is never null: C17 6.3.2.3 p3 says a
            // null pointer compares unequal to a pointer to any object or
            // function, and this is a pointer to one. It is the one rvalue this
            // analysis can prove.
            //
            // **Not 6.5.3.2 p3**, which is what this cited first and which says
            // only what `&` yields. Its footnote runs the other way: `&*E` is
            // `E` even where `E` is null. That shape never arrives here because
            // the lowering folds it away, which is ADR-0021, so the rule is
            // safe for a reason that has nothing to do with the clause it used
            // to name.
            Rvalue::Address(_) => Nullness::NonNull,
            // Pointer arithmetic and anything computed. `p + 1` off a non-null `p`
            // is non-null in practice and this does not say so: the operand is an
            // integer's worth of work away from the rule above, and a warning on
            // correct C is the cheaper of the two ways to be wrong.
            Rvalue::Unary { op: _, operand: _ }
            | Rvalue::Binary {
                op: _,
                lhs: _,
                rhs: _,
            } => Nullness::Unknown,
        }
    }
}

impl Analysis for Nullability<'_> {
    type Value = Vec<Nullness>;

    /// One step per local.
    ///
    /// A local's value at a block's entry only ever rises, and it can rise
    /// once: [`Nullness::Unknown`] is the top and the other two are
    /// incomparable, so a local that moves at all moves to the top and stops.
    /// The transfers rebuild from the entry value on each visit rather than
    /// editing it, so nothing here lowers one.
    fn height(&self, function: &Function) -> usize {
        function.locals().len()
    }

    /// Nothing known about anything, except a parameter declared `_Nonnull`.
    ///
    /// A parameter's nullness is a caller's fact, so `void f(int *p) { *p =
    /// 1; }` is not established here. `_Nonnull` is how a caller's fact is
    /// carried: the body believes it, and every call in the translation unit is
    /// asked whether it keeps it, which is ADR-0037.
    ///
    /// It is a fact at the entry and nowhere else. The body can replace it like
    /// any other value, and a parameter whose address escapes answers
    /// `Unknown` whatever it was declared, because every read goes through
    /// [`Nullability::known`].
    fn on_entry(&self) -> Self::Value {
        let mut value = vec![Nullness::Unknown; self.locals];
        for parameter in self.function.parameters() {
            if self.function.nonnull(parameter).is_some() {
                value[parameter.index()] = Nullness::NonNull;
            }
        }
        value
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        for (here, there) in into.iter_mut().zip(from) {
            *here = here.joined(*there);
        }
    }

    fn element(&self, _function: &Function, element: &Element, value: &mut Self::Value) {
        // Before the assignment below, for the reason `Self::terminator` gives
        // about a call's destination: a dereference is a fact about the value
        // the local held when it ran, and the assignment is what replaces that
        // value. `pp = *pp;` is the shape that tells them apart, and with this
        // the other way round the fact about the old pointer was written over
        // the answer about the new one, leaving `pp` proved non-null when
        // nothing at all was known about what it now holds.
        met(dereferenced_in_element(element), value);

        // Every field written out, never `..`, which would let a field added
        // to a variant that already exists walk past an exhaustive match.
        match element {
            Element::Assign(operation) => {
                if operation.place.projection.is_empty() {
                    value[operation.place.local.index()] =
                        self.nullness_of(&operation.value, value);
                }
                // A write through a projection lands somewhere this lattice
                // cannot name, so it says nothing about what was written. What
                // it does say about the pointer it went through is below,
                // where every dereference says the same thing.
            }
            // Evaluating a place computes nothing into a local.
            Element::Evaluate {
                place: _,
                origin: _,
            } => {}
            // Neither marker says what a pointer holds. This check keeps no
            // fact that waits for an order, so it has nothing for either of
            // them to bound.
            Element::Sequenced { origin: _ } | Element::ArgumentsEvaluated { origin: _ } => {}
            // Storage beginning or ending leaves a local holding nothing this
            // check can name, and the same on both, because what is lost is
            // the same either way.
            Element::StorageLive { local, origin: _ } => value[local.index()] = Nullness::Unknown,
            Element::StorageDead { origin: _, local } => value[local.index()] = Nullness::Unknown,
        }
    }

    fn terminator(&self, _function: &Function, terminator: &Terminator, value: &mut Self::Value) {
        // Before the destination is written, because the arguments are read
        // where the call is reached and the destination is written when it
        // returns.
        met(dereferenced_in_terminator(terminator), value);

        match terminator {
            Terminator::Call {
                callee,
                arguments: _,
                destination,
                then: _,
                origin: _,
            } => {
                // Nothing, unless the callee promised otherwise. `malloc` is
                // not special here, which is this module's own doc comment and
                // the reason the roadmap's example warns.
                //
                // **A promise is believed of what the call returns**, and this
                // is a new place that mints `NonNull`, so it is worth saying
                // what makes it sound: every `return` of a function that
                // promises is asked by `report_return`. What is believed
                // unasked is a hatch's unproven return, which is listed rather
                // than reported and is the hatch's own boundary (ADR-0038),
                // and a `_Nonnull` on a function this unit does not define,
                // which nobody here can ask (ADR-0050); both are the boundary
                // a written promise draws, as ADR-0037 does of a
                // parameter.
                //
                // Two `if`s rather than a let chain, which the workspace's
                // `rust-version` of 1.85 does not have.
                if let Some(place) = destination {
                    if place.projection.is_empty() {
                        value[place.local.index()] = match self.unit.function(*callee).promised() {
                            Some(_) => Nullness::NonNull,
                            None => Nullness::Unknown,
                        };
                    }
                }
            }
            Terminator::Goto(_)
            | Terminator::Branch {
                condition: _,
                then: _,
                otherwise: _,
                origin: _,
            }
            | Terminator::Return
            | Terminator::Abnormal { to: _ } => {}
        }
    }

    fn edge(
        &self,
        function: &Function,
        block: BlockId,
        terminator: &Terminator,
        index: usize,
        value: &mut Self::Value,
    ) {
        // Matched before the index is read, which is the rule the trait states:
        // a `Goto`'s one edge is index 0 as well.
        let Terminator::Branch {
            condition: Operand::Copy(condition),
            ..
        } = terminator
        else {
            return;
        };

        let Some((local, on_then)) = tested_against_null(self.unit, function, block, condition)
        else {
            return;
        };

        // **A local this check has already settled is left alone.** `int *p =
        // 0; if (p) { *p = 1; }` reaches the taken arm with `p` known null, and
        // that arm never runs; `Analysis::edge` cannot say so, and refining to
        // non-null there would make this check quiet about a dereference it had
        // proved. What it does instead is report code no execution reaches:
        // a false report the reader can see, where going quiet would be saying
        // safe wrongly.
        if value[local.index()] != Nullness::Unknown {
            return;
        }

        // `Terminator::successors` pushes `then` and then `otherwise`, so a
        // branch has exactly those two edges and index 1 is the other arm.
        let learned = if index == 0 {
            on_then
        } else {
            on_then.inverted()
        };

        value[local.index()] = learned;

        // Every local the tested one was copied from and still equals, each
        // under the rule above: one this check has settled is left alone.
        for source in copied_from(function, block, local) {
            if value[source.index()] == Nullness::Unknown {
                value[source.index()] = learned;
            }
        }
    }
}

/// What every dereference in one element or terminator says about the pointers
/// it went through.
///
/// A dereference that this check did not report as null did not trap on the
/// path that reached it, so the pointer was not null there and the next
/// dereference on that path is nothing to say about. It is per path and not per
/// function: an arm that skips the dereference joins `Unknown` back in, which
/// is what makes this a fact rather than a suppression. See ADR-0025.
fn met(dereferenced: Option<(Span, Vec<&Place>)>, value: &mut [Nullness]) {
    let Some((_, places)) = dereferenced else {
        return;
    };
    for place in places {
        value[place.local.index()] = Nullness::NonNull;
    }
}

/// One finding for one element, naming the worst of what it dereferences.
///
/// Not one per place: the words a reader is given do not name the value, which
/// is #136, so two findings at one caret are two diagnostics nobody can tell
/// apart. The worst rather than the first, because a proved null dereference
/// beside an unproven one is still a proved null dereference.
///
/// **A place asks two questions where it goes through memory.** Its first
/// dereference reads the local's own value, which is what this lattice knows.
/// Every one after that reads a pointer the dereference above it loaded, and
/// that pointer has no row here, so it is asked as [`Nullness::Unknown`]: a
/// pointer read out of memory proves nothing, which is ADR-0045 on the memory
/// axis. Asking the local alone was how `int *p = 0; int **pp = &p; return
/// **pp;` built in silence while `int *q = *pp; return *q;` was refused, and the
/// two are one C program (#333).
fn report(
    analysis: &Nullability<'_>,
    findings: &mut Vec<Finding>,
    dereferenced: Option<(Span, Vec<&Place>)>,
    known: &[Nullness],
    function: FuncId,
) {
    let Some((at, places)) = dereferenced else {
        return;
    };

    let worst = places
        .iter()
        .flat_map(|place| {
            let below = read_out_of_memory(place).then_some(Nullness::Unknown);
            [Some(analysis.known(known, place.local)), below]
        })
        .flatten()
        .filter_map(Nullness::concluded)
        .max_by_key(|conclusion| severity(*conclusion));

    if let Some(conclusion) = worst {
        findings.push(Finding {
            function,
            conclusion,
            at,
            asked: Asked::Dereference {
                through_memory: places.iter().any(|place| read_out_of_memory(place)),
            },
        });
    }
}

/// Whether a place dereferences a pointer that was read out of memory.
///
/// A `Deref` anywhere after the first element, rather than two or more of
/// them, so that an element selected by an index and then dereferenced counts
/// too: the pointer an index selects is just as much a load. Nothing builds a
/// [`Projection::Index`] from C today, so a unit test with IR built by hand is
/// what holds that half.
fn read_out_of_memory(place: &Place) -> bool {
    place
        .projection
        .iter()
        .skip(1)
        .any(|projection| matches!(projection, Projection::Deref))
}

/// One finding for each argument a call passes to a `_Nonnull` parameter,
/// unless it is established not null.
///
/// The dereference's question, asked of an argument, and asked at the same
/// point: before the terminator's transfer, so a call's own destination is
/// written after, and `q = g(q)` asks about the `q` that was passed.
///
/// **This is the second reader of `Nullness::NonNull`, and for it being wrong
/// is a silence.** A dereference reads it too, and each place that mints it
/// was made sound for that reader: an address, a dereference that did not trap
/// (ADR-0025), a branch that tested the pointer, and a `_Nonnull` parameter.
/// The argument is read where the dereference is, so it inherits that reader's
/// answers **and its gaps**, and this sentence used to claim only the first.
/// The one review found: every argument is lowered before the call, so
/// `g(q, q = &x)` asks about `q` after the second argument wrote it, and a null
/// `q` is silent. C17 6.5 p2 makes that call undefined, and `h(*q, q = &x)` is
/// silent on the dereference side for the same reason. ADR-0037 records it as
/// a consequence, and #247 is the fix. An approximation is safe in one
/// direction only, and its reader decides which, so a second reader has to
/// check the first one's soundness rather than assume it.
///
/// One per argument rather than the worst per call, because each argument has
/// its own promise to point at.
///
/// **Walked over the parameters, not over the arguments.** A call written
/// through `void g();` passes however many arguments it likes, and C17 6.5.2.2
/// p6 makes a call with fewer than the definition takes undefined, so the
/// parameter it left out holds nothing anybody chose. The body believes it all
/// the same. So a `_Nonnull` parameter with no argument is one this check did
/// not establish, and is reported as that. Zipping the two lists instead was
/// how this shipped to review, and it left `void g(); void h(void) { g(); }`
/// silent against a body that dereferences its parameter.
fn report_arguments(
    analysis: &Nullability<'_>,
    findings: &mut Vec<Finding>,
    terminator: &Terminator,
    known: &[Nullness],
    function: FuncId,
) {
    // Every terminator written out rather than `let ... else`, so that a
    // second way to call a function has to be answered for here. A call
    // through a pointer is the one the roadmap brings, and passed by in
    // silence it would be every argument to a `_Nonnull` parameter unasked.
    // A field walking past the `..` of an exhaustive match is the same shape
    // one level down.
    let (callee, arguments, origin) = match terminator {
        Terminator::Call {
            callee,
            arguments,
            destination: _,
            then: _,
            origin,
        } => (callee, arguments, origin),
        Terminator::Goto(_)
        | Terminator::Branch {
            condition: _,
            then: _,
            otherwise: _,
            origin: _,
        }
        | Terminator::Return
        | Terminator::Abnormal { to: _ } => return,
    };

    let callee = analysis.unit.function(*callee);
    for (index, parameter) in callee.parameters().enumerate() {
        let Some(promise) = callee.nonnull(parameter) else {
            continue;
        };
        let passed = match arguments.get(index) {
            Some(argument) => analysis.nullness_of(&Rvalue::Use(argument.clone()), known),
            None => Nullness::Unknown,
        };
        if let Some(conclusion) = passed.concluded() {
            findings.push(Finding {
                function,
                conclusion,
                at: origin.span(),
                asked: Asked::Argument { promise },
            });
        }
    }
}

/// One finding for a `return` of a function that promised the pointer it
/// returns is not null, unless the pointer is established not null there.
///
/// **Asked at the `Return` terminator rather than at the write into the
/// return place**, because a path that reaches the end of the body writes
/// nothing, and a caller would believe whatever the return place holds:
/// `int *f(int c) { int x; if (c) return &x; }` lowers to a `Return` in a block
/// with no write. The caret is the span of the last write into the return
/// place in this block, which is the `return` statement, and the function's
/// name where there is none. The lowering puts a `return`'s write in the block
/// its `Return` ends, measured with `return g();` among others, and
/// `docs/c-family.md` says what that asks of another frontend.
///
/// Read through [`Nullability::known`] like every other answer, though the
/// return place cannot have its address taken from C.
fn report_return(
    analysis: &Nullability<'_>,
    findings: &mut Vec<Finding>,
    terminator: &Terminator,
    known: &[Nullness],
    returned_at: Option<Span>,
    function: FuncId,
) {
    // Every terminator written out rather than `let ... else`, for the reason
    // `report_arguments` gives.
    match terminator {
        Terminator::Return => {}
        Terminator::Goto(_)
        | Terminator::Branch {
            condition: _,
            then: _,
            otherwise: _,
            origin: _,
        }
        | Terminator::Call {
            callee: _,
            arguments: _,
            destination: _,
            then: _,
            origin: _,
        }
        | Terminator::Abnormal { to: _ } => return,
    }
    let Some(promise) = analysis.function.promised() else {
        return;
    };
    let returned = analysis.known(known, analysis.function.return_place());
    let Some(conclusion) = returned.concluded() else {
        return;
    };
    let (at, reached_end) = match returned_at {
        Some(at) => (at, false),
        None => (analysis.function.name, true),
    };
    findings.push(Finding {
        function,
        conclusion,
        at,
        asked: Asked::Return {
            promise,
            reached_end,
        },
    });
}

/// Every dereference of a null pointer this unit contains, and every one it
/// cannot rule out.
///
/// One walk per function: the fixpoint answers what holds where each block
/// starts, and this replays each block from there to find the elements to
/// report. The replay rather than a second lattice, for the reason
/// [`crate::memory::findings`] gives.
///
/// **No `SourceMap`.** The memory check takes one because it folds two spans
/// into one finding; nothing here compares spans, and an argument no
/// implementation reads is one a caller can get wrong without anything saying
/// so.
pub fn findings(unit: &TranslationUnit) -> Vec<Finding> {
    let mut findings = Vec::new();

    for func in unit.functions() {
        let function = unit.function(func);
        // A declaration has no blocks, and `Function::blocks` panics rather
        // than answering for one.
        if !function.is_defined() {
            continue;
        }

        let analysis = Nullability {
            unit,
            function,
            locals: function.locals().len(),
            escaped: escaped_in(function),
        };
        let cfg = Cfg::of(function);
        let solution = solve(&analysis, function, &cfg);

        for &id in cfg.order() {
            // What a `None` says is that no execution reaches this block, and
            // saying nothing about code nothing runs is the right answer to
            // that however it arose.
            let Some(mut known) = solution.value(id).cloned() else {
                continue;
            };

            let block = function.block(id);
            // The last `return` statement's write into the return place in
            // this block, which is where a report about what it returned
            // points.
            let mut returned_at = None;
            for element in &block.elements {
                // Before the transfer, which is what the element does: the
                // question is what was true where it runs.
                report(
                    &analysis,
                    &mut findings,
                    dereferenced_in_element(element),
                    &known,
                    func,
                );
                if let Element::Assign(operation) = element {
                    if operation.place == Place::local(function.return_place()) {
                        returned_at = Some(operation.origin.span());
                    }
                }
                analysis.element(function, element, &mut known);
            }

            report(
                &analysis,
                &mut findings,
                dereferenced_in_terminator(&block.terminator),
                &known,
                func,
            );
            report_arguments(&analysis, &mut findings, &block.terminator, &known, func);
            report_return(
                &analysis,
                &mut findings,
                &block.terminator,
                &known,
                returned_at,
                func,
            );
        }
    }

    // In the order a reader's eye goes rather than the order the walk reached
    // them, for the reason `crate::memory::findings` gives at its own sort.
    findings.sort_by_key(|finding| (finding.at.file().index(), finding.at.start()));

    // **One statement is one thing to say, however many elements it became.**
    // `*p = 1;` lowers to the write and a read of what it wrote, both carrying
    // that statement's span, so a pointer this check cannot settle is asked
    // about twice at one caret and the reader is told the same thing twice.
    // Where a dereference refines the pointer the second element already
    // answered nothing, so this only reaches the locals that refinement cannot
    // help, which is the escaped ones.
    //
    // **The worst survives, not the first.** `report` takes the worst of the
    // places one element dereferences, but two elements can share a caret and
    // disagree: `*p && *q` writes both operands at the whole expression's span,
    // so an unproven `*p` and a proved `*q` arrive as two findings at one
    // caret, the unproven one first. Keeping the first reported a proof as a
    // suspicion, and inside a hatch, where a suspicion is listed rather than
    // reported, a proved null dereference built and ran. `memory::report::say`
    // answers the same shape with `supersedes`; this is the same rule.
    //
    // **The question is part of the key.** A call that passes two arguments
    // to two `_Nonnull` parameters has one caret and two promises, and a
    // dereference inside a call's arguments shares its caret with the call.
    // Each of those is a different thing to say. Whether a dereference went
    // through memory is not part of it, and the survivor keeps it if either
    // did, which `Asked::joined` says why.
    //
    // **So is the function.** The sort above is over the whole unit, and a
    // finding is routed by the function it names: two functions sharing a
    // caret, which one `#include`d fragment would do, must not fold one into
    // the other, or a hatch's could absorb an ordinary function's.
    findings.dedup_by(|later, earlier| {
        let same = later.function == earlier.function
            && later.at == earlier.at
            && later.asked.same_question(earlier.asked);
        if same {
            if severity(later.conclusion) > severity(earlier.conclusion) {
                std::mem::swap(later, earlier);
            }
            earlier.asked = earlier.asked.joined(later.asked);
        }
        same
    });

    findings
}

/// Which locals this check established are null where each block's terminator
/// runs.
///
/// Indexed by [`BlockId::index`], a row of one `bool` per local. A block no
/// execution reaches holds a row of `false`, which says nothing was established
/// about those locals rather than that something was.
///
/// **Nothing in this module reads it.** [`crate::memory`] does, to exempt a
/// free of a pointer this check established is null, which C17 7.22.3.3 p2
/// makes a call that does nothing. ADR-0027 is that rule, and three of the four
/// conditions it names for an implementation are held here:
///
/// - the answer is read through [`Nullability::known`], so a local whose
///   address escaped answers nothing at all. A raw read of the value would
///   exempt a free of a pointer a store this check cannot follow has since
///   replaced, which is a double free reported by nobody;
/// - it is recorded where the terminator runs rather than where the block
///   starts. `int *p = 0; free(q); p = q; free(p);` is established null at the
///   entry of the block that frees `p` and is not null where the free runs, and
///   answering with the entry silences a proved double free;
/// - it is recorded only where C has ordered the block's writes before the
///   terminator that reads them, which the body says more about. This lattice
///   has no notion of order and says so, and the check that reads this answer
///   is built on one.
///
/// The fourth is the caller's and is held by what this does not return: there
/// is no answer here for a point inside a block, so nothing can reach the
/// transfer with it.
///
/// **The replay lives here rather than in the module that asks**, so that there
/// is one walk over this lattice. A second copy of "run the elements, then ask"
/// would agree today and stop agreeing the day [`Analysis::element`] learns
/// something new, with nothing failing when it does.
pub(crate) fn null_at_terminators(
    unit: &TranslationUnit,
    function: &Function,
    cfg: &Cfg,
) -> NullAtTerminators {
    let analysis = Nullability {
        unit,
        function,
        locals: function.locals().len(),
        escaped: escaped_in(function),
    };
    let solution = solve(&analysis, function, cfg);

    let mut null = vec![vec![false; function.locals().len()]; function.blocks().len()];

    for &id in cfg.order() {
        // What a `None` says is that no execution reaches this block, and the
        // row of `false` it keeps says nothing was established there, which is
        // the answer that exempts nothing. Nothing holds that and nothing can:
        // the caller walks the reachable blocks, so a row filled with `true`
        // here changes no program. It is the answer that is right about a
        // block rather than the answer that is convenient.
        let Some(mut known) = solution.value(id).cloned() else {
            continue;
        };

        let block = function.block(id);
        for element in &block.elements {
            analysis.element(function, element, &mut known);
        }

        // **C has to have ordered what this row rests on before the terminator
        // that reads it**, which is ADR-0027's fourth condition and is where
        // the program that needs it is written out. The replay above is what
        // makes it necessary: it walks every element of the block without
        // asking whether C put any of them before the call at the end.
        //
        // [`Element::ArgumentsEvaluated`] is the answer already in the IR.
        // ADR-0026 emits it only where no unsequenced operator encloses the
        // call, so this is the same test as the lowering's `at_root`, and that
        // is why it is **sufficient** rather than merely suggestive: `at_root`
        // holds only when every ancestor of the call sequences its operands, so
        // everything before the call in this block is ordered before it, and a
        // later sibling cannot be in this block because ADR-0010 makes the call
        // end it.
        //
        // **The block's last element, because the rule is about this
        // terminator.** A call ends its block, so a block holds at most one
        // call and the marker for it is always last; measured over the corpus,
        // every occurrence is immediately followed by its `Call`. Searching the
        // whole block would answer the same on every program this compiler can
        // build, so nothing holds the difference and nothing can. How many
        // occurrences there are is not written here, because a count is a
        // fact about a corpus that grows.
        //
        // **A block with no elements is not ordered either**, which is the
        // `None` this answers `false` for and is reached by a program rather
        // than by tidiness: a call ends a block, so an ordinary call between
        // the two unsequenced operands leaves the free at the terminator of an
        // empty one.
        // `a_free_in_an_unsequenced_operand_across_a_call_is_not_exempt` is
        // that program, and adding `| None` here leaves it silent about a
        // double free and fails nothing else.
        //
        // **Accepting [`Element::Sequenced`] here as well is held by nothing.**
        // Measured: it leaves the whole workspace green, because a block whose
        // last element is that marker and whose terminator is a call is a
        // shape no program here builds. It is refused anyway, because the
        // markers conclude different things and ADR-0026 is where the
        // difference is written.
        //
        // **This zeroes the whole row rather than one local**, so a null
        // established in an earlier, properly ordered statement is refused the
        // exemption along with everything else in a block whose call is not at
        // the root. That is a false positive on well-defined C and ADR-0027's
        // Consequences carry the programs.
        let ordered = matches!(
            block.elements.last(),
            Some(Element::ArgumentsEvaluated { .. })
        );

        // Before the terminator's own transfer, which is the position
        // [`findings`] reports from and the position the free is reached at.
        //
        // **A local that does not hold a pointer is never established null
        // here, whatever the lattice says about it.** [`Nullability::nullness_of`]
        // answers [`Nullness::Null`] for a constant zero without asking what it
        // is being assigned to, which costs nothing where the answer is only
        // read about a dereference and costs a diagnostic here: `int x = 0;
        // free(x);` was reported as a pointer this check stopped following and
        // went silent when the exemption started reading these rows. C17
        // 6.3.2.3 p3 makes a null pointer constant an *integer constant
        // expression* converted to a pointer type, and an `int` lvalue holding
        // zero is neither, so nothing about that program is the clause the
        // exemption rests on.
        for local in function.locals() {
            null[id.index()][local.index()] = ordered
                && analysis.is_pointer(function, local)
                && analysis.known(&known, local) == Nullness::Null;
        }
    }

    NullAtTerminators { rows: null }
}

/// What [`null_at_terminators`] answered, keyed by the two things it is about.
///
/// **A named type rather than the `Vec<Vec<bool>>` it holds**, because the
/// consumer is a rule that can go quiet. `docs/roadmap.md` queues three more
/// analyses against this framework and each will want to hand a settled fact to
/// a sibling the same way, so a reader of `memory::report::reported` would soon
/// be given several `&[bool]` that no type tells apart: measured, adding a
/// second one and passing the two in the wrong order builds with no warning at
/// all, and what fails is a named test rather than the compiler. A mistake the
/// compiler refuses is better than one a test has to catch, and a named type is
/// what moves this one there.
///
/// Both axes are named for the same reason. A bare row is indexed by a local,
/// a bare table by a block, and `null[block.index()]` and `null[local.index()]`
/// are both `usize`: the wrong one is a panic where the lengths differ and a
/// silent wrong exemption where they do not.
pub(crate) struct NullAtTerminators {
    /// One row per block, one `bool` per local, both in the order the function
    /// hands its ids out.
    rows: Vec<Vec<bool>>,
}

impl NullAtTerminators {
    /// Whether this check established that `local` is null where `block`'s
    /// terminator runs.
    ///
    /// # Panics
    ///
    /// If either id belongs to a different function than the one this was built
    /// for, which is the only way to be out of range and is a caller's mistake
    /// rather than an input's.
    pub(crate) fn established(&self, block: BlockId, local: LocalId) -> bool {
        self.rows[block.index()][local.index()]
    }
}

/// How much a conclusion outranks another where both stand at one caret.
///
/// Not an `Ord` on [`Conclusion`]: that type is `safec_ir`'s answer about one
/// thing rather than a scale, and ADR-0002 puts the ordering a reader sees on
/// `Severity` in `safec`, which is the crate that owns what a conclusion costs.
fn severity(conclusion: Conclusion) -> u8 {
    match conclusion {
        Conclusion::Unsafe => 2,
        Conclusion::Unknown => 1,
        Conclusion::Safe => 0,
    }
}
