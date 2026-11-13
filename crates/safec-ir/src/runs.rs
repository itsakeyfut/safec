//! Whether a loop is shown to run its body at least once.
//!
//! A loop may run zero times, so the path that skips its body leaves what
//! the body writes unwritten, and the memory check doubts a read after it
//! (#783). Where the loop's own test is true on the way in, that path is not
//! one any execution takes, and the doubt would refuse `for (int i = 0; i <
//! 2; i++) m[0] = &x;` for a turn that never happens. This answers that one
//! shape and nothing wider: a test of constants, and of locals given a
//! constant on the one way into the loop. See ADR-0064.
//!
//! **Wrong in both directions, and the two are not alike.** A loop this
//! fails to show is doubted, which the reader sees. A loop this shows that
//! can run zero times is a read nothing wrote believed written, so every
//! rule below refuses rather than guesses.

use crate::cfg::{Cfg, Loop};
use crate::ir::{
    BinOp, BlockId, Element, Function, LocalId, Operand, Rvalue, Terminator, TranslationUnit, Ty,
};

use super::bits::Bits;
use super::index::LocalIdx;

/// That the loop headed by [`Self::head`] runs at least once, whose
/// zero-turn path no execution takes.
///
/// **Only [`shown_to_run`] builds one**: the field is private, so a hook
/// that answered "this loop runs" for a loop nobody showed does not compile
/// (`error[E0451]`). That answer is a silence about the loop's zero-turn
/// path, so it was the one mutation of the solver's hook that built and
/// misreported. See #895 and ADR-0083.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShownToRun {
    head: BlockId,
}

impl ShownToRun {
    /// The head of the loop shown to run.
    pub fn head(self) -> BlockId {
        self.head
    }
}

/// Whether `each` is shown to run its body at least once, with its head's
/// test the only way past it and taken only after a whole turn.
///
/// `taken` is every local whose address is taken, which `taken::address_taken`
/// answers: a write through an address is not followed here, so such a local
/// is never taken to hold a constant.
///
/// **All of these, each refused rather than guessed:**
///
/// - one edge into the head from outside the loop, so "on the way in" is one
///   path's answer and not a join this would have to take;
/// - one latch, so every turn that comes round ends at the same block: a
///   second back edge leaving the body early, a `continue`, carries a turn
///   that did not finish;
/// - no edge leaving the loop from inside its body reaches where the head's
///   test leaves it, so the only way past the loop is the head's own test on a
///   later turn: a `break` reaching that block on the first turn would carry
///   the skipped write past it. The only way out of a body today is a
///   `return`, the frontend reading neither `break` nor `goto` yet;
/// - the head's test is true on the way in, by [`head_test`].
pub(crate) fn shown_to_run(
    unit: &TranslationUnit,
    function: &Function,
    cfg: &Cfg,
    each: &Loop,
    taken: &Bits<LocalIdx>,
) -> Option<ShownToRun> {
    let head = each.head();
    let mut outside = cfg
        .predecessors(head)
        .iter()
        .filter(|&&from| !each.contains(from));
    let (Some(&entry), None) = (outside.next(), outside.next()) else {
        return None;
    };
    if each.latches().len() != 1 {
        return None;
    }
    let Terminator::Branch {
        condition,
        then,
        otherwise,
        ..
    } = &function.block(head).terminator
    else {
        return None;
    };
    if !each.contains(*then) || each.contains(*otherwise) {
        return None;
    }
    if leaves_early_to(function, cfg, each, *otherwise) {
        return None;
    }
    (head_test(unit, function, cfg, entry, head, condition, taken) == Some(true))
        .then_some(ShownToRun { head })
}

/// Whether an edge leaving `each` from a block other than its head reaches
/// anything `past` reaches, `past` being the block the head's test leaves
/// the loop for.
///
/// **Anything `past` reaches, not `past` alone**: a `goto` from the body to
/// a label after the loop reaches what follows it without passing `past`,
/// and carries the first turn's skipped write there all the same.
fn leaves_early_to(function: &Function, cfg: &Cfg, each: &Loop, past: BlockId) -> bool {
    let mut after = vec![false; function.blocks().len()];
    let mut stack = vec![past];
    let mut successors = Vec::new();
    while let Some(block) = stack.pop() {
        if std::mem::replace(&mut after[block.index()], true) {
            continue;
        }
        successors.clear();
        function.block(block).terminator.successors(&mut successors);
        stack.extend(successors.iter().copied());
    }
    let mut seen = vec![false; function.blocks().len()];
    for &block in cfg.order() {
        if block == each.head() || !each.contains(block) {
            continue;
        }
        successors.clear();
        function.block(block).terminator.successors(&mut successors);
        stack.extend(successors.iter().copied().filter(|&to| !each.contains(to)));
    }
    while let Some(block) = stack.pop() {
        if after[block.index()] {
            return true;
        }
        if std::mem::replace(&mut seen[block.index()], true) {
            continue;
        }
        successors.clear();
        function.block(block).terminator.successors(&mut successors);
        stack.extend(successors.iter().copied());
    }
    false
}

/// What the head's test answers on the way in from `entry`: `Some(true)`
/// where it is shown true, and `None` wherever any value it reads is not a
/// constant this can show.
///
/// The head's elements are run forward, each local they write taking the
/// value written, and a local they read before writing it takes the
/// constant [`constant_on_the_way_in`] finds. **Only an `int` holds one**:
/// the IR carries no conversion, so `char c = 200;` holds 200 here and not
/// what C stores, and a `char` is never taken to hold a constant.
///
/// **A local whose address is taken holds nothing, even after the head
/// wrote it**, since `*p = 5` writes it as a write to `p`: `(i = 0, *p = 5,
/// i < 1)` was shown to run. **And a local the head writes twice holds
/// nothing after the second write**: two writes C leaves unsequenced are
/// undefined (C17 6.5 p2) and the IR lowers them in an order C did not
/// choose, so `(i = 0) < (i = 3)` was shown to run by that order. A second
/// write the head does sequence, `(i = 0, i = 1, i < 2)`, is refused with
/// it, which costs a doubt.
fn head_test(
    unit: &TranslationUnit,
    function: &Function,
    cfg: &Cfg,
    entry: BlockId,
    head: BlockId,
    condition: &Operand,
    taken: &Bits<LocalIdx>,
) -> Option<bool> {
    let holds_int = |local: LocalId| matches!(unit.ty(function.local(local)), Ty::Int);
    let mut written: Vec<(LocalId, Option<i128>)> = Vec::new();
    let value = |written: &[(LocalId, Option<i128>)], operand: &Operand| match operand {
        Operand::Constant(c) => small(*c),
        Operand::Copy(place) if place.projection.is_empty() => {
            if taken.get(LocalIdx::of(place.local)) || !holds_int(place.local) {
                return None;
            }
            match written
                .iter()
                .rev()
                .find(|(local, _)| *local == place.local)
            {
                Some(&(_, known)) => known,
                None => constant_on_the_way_in(function, cfg, entry, place.local),
            }
        }
        Operand::Copy(_) => None,
    };
    for element in &function.block(head).elements {
        match element {
            Element::Assign(operation) => {
                let again = written
                    .iter()
                    .any(|(local, _)| *local == operation.place.local);
                let known = if again {
                    None
                } else if operation.place.projection.is_empty() {
                    match &operation.value {
                        Rvalue::Use(operand) => value(&written, operand),
                        Rvalue::Binary { op, lhs, rhs } => {
                            match (value(&written, lhs), value(&written, rhs)) {
                                (Some(lhs), Some(rhs)) => compared(*op, lhs, rhs),
                                _ => None,
                            }
                        }
                        Rvalue::Unary { .. } | Rvalue::Address(_) => None,
                    }
                } else {
                    None
                };
                written.push((operation.place.local, known));
            }
            // A local made again holds nothing. `UnsequencedAccess` names
            // only a pointer today (ADR-0052), which holds no constant here
            // anyway; the second write above is what refuses an `int`.
            Element::StorageLive { local, .. } | Element::UnsequencedAccess { local, .. } => {
                written.push((*local, None));
            }
            Element::Evaluate { .. }
            | Element::StorageDead { .. }
            | Element::Sequenced { .. }
            | Element::ArgumentsEvaluated { .. } => {}
        }
    }
    value(&written, condition).map(|c| c != 0)
}

/// The constant `local` holds on every path from `entry` back, where those
/// paths are one: `entry` and the blocks before it while each has one
/// predecessor and leaves only for the next. `None` past that, or where the
/// last write is not a constant.
fn constant_on_the_way_in(
    function: &Function,
    cfg: &Cfg,
    entry: BlockId,
    local: LocalId,
) -> Option<i128> {
    let mut block = entry;
    let mut successors = Vec::new();
    // One step per block at most: a chain of single predecessors that comes
    // round again is a loop nothing enters, and it answers nothing.
    for _ in 0..function.blocks().len() {
        let here = function.block(block);
        if matches!(
            &here.terminator,
            Terminator::Call { destination: Some(place), .. } if place.local == local
        ) {
            return None;
        }
        for element in here.elements.iter().rev() {
            match element {
                Element::Assign(operation) if operation.place.local == local => {
                    return match (&operation.value, operation.place.projection.is_empty()) {
                        (Rvalue::Use(Operand::Constant(c)), true) => small(*c),
                        _ => None,
                    };
                }
                Element::StorageLive { local: made, .. } if *made == local => return None,
                _ => {}
            }
        }
        let [before] = cfg.predecessors(block) else {
            return None;
        };
        successors.clear();
        function
            .block(*before)
            .terminator
            .successors(&mut successors);
        if successors.as_slice() != [block] {
            return None;
        }
        block = *before;
    }
    None
}

/// `c`, where it is a value every integer type here holds alike.
///
/// **Not negative and no wider than `int`**, because the IR carries no
/// conversion: `-1 < 2u` is false in C (6.3.1.8), and a constant stored into
/// a `char` may not be the value written. Between zero and `INT_MAX` a
/// comparison answers the same whatever the operands' types are.
fn small(c: i128) -> Option<i128> {
    (0..=i128::from(i32::MAX)).contains(&c).then_some(c)
}

/// What a comparison of two [`small`] values gives, and `None` for every
/// other operator: arithmetic could leave the range where types agree.
fn compared(op: BinOp, lhs: i128, rhs: i128) -> Option<i128> {
    let holds = match op {
        BinOp::Lt => lhs < rhs,
        BinOp::Gt => lhs > rhs,
        BinOp::Le => lhs <= rhs,
        BinOp::Ge => lhs >= rhs,
        BinOp::Eq => lhs == rhs,
        BinOp::Ne => lhs != rhs,
        BinOp::Mul
        | BinOp::Div
        | BinOp::Rem
        | BinOp::Add
        | BinOp::Sub
        | BinOp::Shl
        | BinOp::Shr
        | BinOp::BitAnd
        | BinOp::BitXor
        | BinOp::BitOr => return None,
    };
    Some(i128::from(holds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Block, Origin};
    use crate::source::{SourceMap, Span};
    use crate::target::Target;

    /// The edges of a loop whose head tests the constant 1, built by hand:
    /// blocks 0 to `blocks - 1`, the entry `0` going to the head `1`, the
    /// head going to `2` or to `past`, and `edges` giving every other
    /// block's successors, one for a `Goto`, two for a `Branch`, none for a
    /// `Return`. Answers whether the loop at `1` is shown to run.
    ///
    /// **By hand because no C reaches these shapes yet**: the frontend reads
    /// neither `continue`, `break` nor `goto`, and each refusal they need
    /// would otherwise be held by nothing until it did.
    fn runs(blocks: usize, past: usize, edges: &[(usize, &[usize])]) -> bool {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "f");
        let at = Span::new(file, 0, 1);
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(at, int, []);
        let ids: Vec<BlockId> = (0..blocks).map(|_| function.reserve_block()).collect();
        let branch = |then: usize, otherwise: usize| Terminator::Branch {
            condition: Operand::Constant(1),
            then: ids[then],
            otherwise: ids[otherwise],
            origin: Origin::Written(at),
        };
        let mut terminators: Vec<Option<Terminator>> = (0..blocks).map(|_| None).collect();
        terminators[0] = Some(Terminator::Goto(ids[1]));
        terminators[1] = Some(branch(2, past));
        for &(block, to) in edges {
            terminators[block] = Some(match to {
                [] => Terminator::Return,
                [to] => Terminator::Goto(ids[*to]),
                [then, otherwise] => branch(*then, *otherwise),
                _ => unreachable!("a block has at most two successors here"),
            });
        }
        for (id, terminator) in ids.iter().zip(terminators) {
            function.fill_block(
                *id,
                Block {
                    elements: vec![],
                    terminator: terminator.expect("every block is given its edges"),
                },
            );
        }
        let taken = Bits::new(function.locals().len());
        let cfg = Cfg::of(&function);
        let each = cfg
            .loops()
            .into_iter()
            .find(|each| each.head() == ids[1])
            .expect("block 1 heads a loop");
        shown_to_run(&unit, &function, &cfg, &each, &taken).is_some()
    }

    /// The control: one latch, and the only way out is the head's test.
    /// Mutation: `shown_to_run` answers `false` once its other checks pass;
    /// this fails, and so do the corpus's loops counted by constants.
    #[test]
    fn a_loop_whose_test_holds_on_the_way_in_is_shown_to_run() {
        assert!(runs(4, 3, &[(2, &[1]), (3, &[])]));
    }

    /// `continue`: a second back edge, from a turn that did not finish.
    /// Mutation: delete the latch count's refusal; this fails.
    #[test]
    fn a_loop_with_two_latches_is_not_shown_to_run() {
        assert!(!runs(5, 4, &[(2, &[1, 3]), (3, &[1]), (4, &[])]));
    }

    /// `break`: an edge from the body to where the head's test leaves.
    /// Mutation: delete the early exit's refusal; this fails.
    #[test]
    fn a_loop_left_from_its_body_for_where_its_test_leaves_is_not_shown_to_run() {
        assert!(!runs(5, 4, &[(2, &[4, 3]), (3, &[1]), (4, &[])]));
    }

    /// `goto out;` with `out:` after the loop: the body reaches what follows
    /// the loop without passing where the test leaves. Found by review.
    /// Mutation: refuse only an early exit that reaches `past` itself; this
    /// fails.
    #[test]
    fn a_loop_left_from_its_body_for_what_follows_it_is_not_shown_to_run() {
        assert!(!runs(6, 4, &[(2, &[5, 3]), (3, &[1]), (4, &[5]), (5, &[])]));
    }

    /// An arm that returns from the body leaves for nothing after the loop.
    /// Mutation: refuse every early exit; this fails, and so do the corpus
    /// cases whose arm returns from a loop counted by constants.
    #[test]
    fn a_loop_left_from_its_body_by_a_return_is_shown_to_run() {
        assert!(runs(6, 4, &[(2, &[5, 3]), (3, &[1]), (4, &[]), (5, &[])]));
    }
}
