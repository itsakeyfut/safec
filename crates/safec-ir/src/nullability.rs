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
//! **Nothing here reports.** This builds a [`Conclusion`] and a span; `safec`
//! turns one into a diagnostic, because this crate cannot see one, which is
//! ADR-0011.

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve};
use crate::ir::{
    BinOp, BlockId, Element, Function, LocalId, Operand, Place, Rvalue, Terminator,
    TranslationUnit, Ty, UnOp,
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
enum Nullness {
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
    /// **`None` means proved and nothing else.** RK-034 in the review knowledge
    /// bank is an arm that meant "proved safe" and "gave up" at once, and
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

/// One dereference this check concluded about, and where.
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
    /// What this check concluded about the dereference.
    pub conclusion: Conclusion,
    /// Where a caret goes: the element or terminator that dereferences.
    pub at: Span,
}

/// Which locals are known null, known not null, or neither.
///
/// **Keyed by the local rather than by the place.** [`Place`]'s own doc comment
/// asks a real analysis for a place, and that is right about the memory axis,
/// where `p` and `*p` have separate states. The question here is about the
/// pointer value a local holds, so the local is the key, and what it costs is
/// that `int **pp; *pp` answers [`Nullness::Unknown`]: a warning on correct C
/// rather than silence about it. It also makes [`Analysis::height`] the local
/// count, read straight off the function, which RK-030 is about.
struct Nullability<'a> {
    /// What a local's type is, so that a branch on an `int` is not read as a
    /// branch on a pointer.
    unit: &'a TranslationUnit,
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
    /// **Nothing holds this and it is worth saying so.** Measured: making it
    /// answer `true` for everything leaves the whole workspace green. What it
    /// prevents is a non-pointer local being given a nullness, and a
    /// non-pointer local is never dereferenced in well-formed IR, so no report
    /// moves. A test for it could not fail, which is worse than none. It stays
    /// because a value whose states are about pointers should not be written
    /// about things that are not pointers, and because the day this lattice
    /// keys on something a `Ty` can distinguish, the rule will already be here.
    fn is_pointer(&self, function: &Function, local: LocalId) -> bool {
        matches!(self.unit.ty(function.local(local)), Ty::Pointer(_))
    }

    /// Which local a branch tested against null, and what its `then` arm learns.
    ///
    /// Two shapes reach here and the difference is not something a reader of
    /// the C could predict, so both are answered:
    ///
    /// - `if (p)` hands the pointer's own place to the terminator, and no
    ///   element of the block writes it. Measured on this compiler's lowering.
    /// - `if (p != 0)` writes the comparison to a temporary and hands a copy of
    ///   that, so the comparison is an element of this block, above the
    ///   terminator. Reaching it is what [`Analysis::edge`]'s block is for.
    ///
    /// The walk obeys the rule that method states: it stops at a store through
    /// a projection, because nothing in such an element says which local it
    /// lands in, and stepping over one resolves a comparison the program has
    /// already overwritten.
    fn tested(
        &self,
        function: &Function,
        block: BlockId,
        condition: &Place,
    ) -> Option<(LocalId, Nullness)> {
        // `if (*p)` tests what `p` points at, which says nothing about `p` on
        // either arm. That the dereference happened is recorded by
        // `Analysis::terminator`, which is a different fact.
        if !condition.projection.is_empty() {
            return None;
        }

        for element in function.block(block).elements.iter().rev() {
            let Element::Assign(operation) = element else {
                continue;
            };

            if !operation.place.projection.is_empty() {
                return None;
            }
            if operation.place.local != condition.local {
                continue;
            }

            // The last write to the condition's local, whatever it is. A
            // comparison against zero is the one shape this reads; anything
            // else, `int c = p != 0;` among them, is a value this analysis
            // cannot follow, and refining on a condition it did not read is
            // the one direction a refinement must not be wrong in.
            return match &operation.value {
                Rvalue::Binary { op, lhs, rhs } => self.compared_to_null(function, *op, lhs, rhs),
                // C17 6.5.3.3 p5: "The expression `!E` is equivalent to
                // `(0==E)`." So `if (!p)` is `if (p == 0)` written shorter, and
                // a reader who cannot tell those apart should not be given two
                // answers. The other two operators say nothing about null.
                Rvalue::Unary {
                    op: UnOp::Not,
                    operand,
                } => self.compared_to_null(function, BinOp::Eq, operand, &Operand::Constant(0)),
                Rvalue::Unary {
                    op: UnOp::Neg | UnOp::BitNot,
                    operand: _,
                } => None,
                Rvalue::Use(_) | Rvalue::Address(_) => None,
            };
        }

        // Nothing in this block wrote it, so the branch tests the place itself.
        // A block with no elements at all is ordinary: `if (p)` is one, and so
        // is the arm a short-circuited condition jumps to, which is why the
        // type is asked rather than assumed.
        self.is_pointer(function, condition.local)
            .then_some((condition.local, Nullness::NonNull))
    }

    /// The local an equality against a null pointer constant names, and what
    /// the `then` arm learns about it.
    ///
    /// `p != 0` and `0 != p` are one program, so both orders are read. C17
    /// 6.3.2.3 p3 makes an integer constant expression with the value 0 a null
    /// pointer constant, which is what the lowering leaves here.
    fn compared_to_null(
        &self,
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

        if !self.is_pointer(function, local) {
            return None;
        }

        // Every operator written out rather than `_`, so that one added later
        // has to answer here: RK-018 in the review knowledge bank is a match
        // that walked past a case nobody had thought about.
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

    /// Nothing known about anything.
    ///
    /// A parameter's nullness is a caller's fact and this phase has nothing
    /// that carries one, so `void f(int *p) { *p = 1; }` is a warning. The
    /// annotation that removes it is the roadmap's next line and is #134.
    fn on_entry(&self) -> Self::Value {
        vec![Nullness::Unknown; self.locals]
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

        // Every field written out, never `..`: RK-018 in the review knowledge
        // bank is a field added to a variant that already exists walking past
        // an exhaustive match.
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
                callee: _,
                arguments: _,
                destination,
                then: _,
                origin: _,
            } => {
                // Whatever the callee is. `malloc` is not special here, which
                // is this module's own doc comment and the reason the
                // roadmap's example warns.
                //
                // Two `if`s rather than a let chain, which the workspace's
                // `rust-version` of 1.85 does not have.
                if let Some(place) = destination {
                    if place.projection.is_empty() {
                        value[place.local.index()] = Nullness::Unknown;
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

        let Some((local, on_then)) = self.tested(function, block, condition) else {
            return;
        };

        // **A local this check has already settled is left alone.** `int *p =
        // 0; if (p) { *p = 1; }` reaches the taken arm with `p` known null, and
        // that arm never runs; `Analysis::edge` cannot say so, and refining to
        // non-null there would make this check quiet about a dereference it had
        // proved. What it does instead is report code no execution reaches,
        // which `CLAUDE.md` ranks above going quiet.
        if value[local.index()] != Nullness::Unknown {
            return;
        }

        // `Terminator::successors` pushes `then` and then `otherwise`, so a
        // branch has exactly those two edges and index 1 is the other arm.
        value[local.index()] = if index == 0 {
            on_then
        } else {
            on_then.inverted()
        };
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
fn report(
    analysis: &Nullability<'_>,
    findings: &mut Vec<Finding>,
    dereferenced: Option<(Span, Vec<&Place>)>,
    known: &[Nullness],
) {
    let Some((at, places)) = dereferenced else {
        return;
    };

    let worst = places
        .iter()
        .filter_map(|place| analysis.known(known, place.local).concluded())
        .max_by_key(|conclusion| severity(*conclusion));

    if let Some(conclusion) = worst {
        findings.push(Finding { conclusion, at });
    }
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

    for id in unit.functions() {
        let function = unit.function(id);
        // A declaration has no blocks, and `Function::blocks` panics rather
        // than answering for one.
        if !function.is_defined() {
            continue;
        }

        let analysis = Nullability {
            unit,
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
            for element in &block.elements {
                // Before the transfer, which is what the element does: the
                // question is what was true where it runs.
                report(
                    &analysis,
                    &mut findings,
                    dereferenced_in_element(element),
                    &known,
                );
                analysis.element(function, element, &mut known);
            }

            report(
                &analysis,
                &mut findings,
                dereferenced_in_terminator(&block.terminator),
                &known,
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
    // The first survives, and it cannot be the milder one. `report` has
    // already taken the worst of the places one element dereferences, and the
    // second element a statement lowers to names only the local the first
    // wrote, which by then is either proved non-null and reported by nothing
    // or masked by the escape and reported as unproven. A promotion here was
    // written first and measured unreachable: inverting it, and deleting it,
    // each left the whole suite green while a panic in this body failed nine
    // cases, so the body runs and the promotion never fires.
    findings.dedup_by(|later, earlier| later.at == earlier.at);

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
/// makes a call that does nothing. ADR-0027 is that rule, and two of the three
/// conditions it names for an implementation are held here:
///
/// - the answer is read through [`Nullability::known`], so a local whose
///   address escaped answers nothing at all. A raw read of the value would
///   exempt a free of a pointer a store this check cannot follow has since
///   replaced, which is a double free reported by nobody;
/// - it is recorded where the terminator runs rather than where the block
///   starts. `int *p = 0; free(q); p = q; free(p);` is established null at the
///   entry of the block that frees `p` and is not null where the free runs, and
///   answering with the entry silences a proved double free.
///
/// The third is the caller's and is held by what this does not return: there is
/// no answer here for a point inside a block, so nothing can reach the transfer
/// with it.
///
/// **The replay lives here rather than in the module that asks**, so that there
/// is one walk over this lattice. A second copy of "run the elements, then ask"
/// would agree today and stop agreeing the day [`Analysis::element`] learns
/// something new, with nothing failing when it does.
pub(crate) fn null_at_terminators(
    unit: &TranslationUnit,
    function: &Function,
    cfg: &Cfg,
) -> Vec<Vec<bool>> {
    let analysis = Nullability {
        unit,
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

        // Before the terminator's own transfer, which is the position
        // [`findings`] reports from and the position the free is reached at.
        for local in function.locals() {
            null[id.index()][local.index()] = analysis.known(&known, local) == Nullness::Null;
        }
    }

    null
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
