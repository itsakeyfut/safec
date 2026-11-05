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

/// Whether `each` is shown to run its body at least once, and to leave by
/// its head only after a whole turn.
///
/// `taken` is every local whose address is taken, which `assigned::address_taken`
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
pub(super) fn shown_to_run(
    unit: &TranslationUnit,
    function: &Function,
    cfg: &Cfg,
    each: &Loop,
    taken: &Bits<LocalIdx>,
) -> bool {
    let head = each.head();
    let mut outside = cfg
        .predecessors(head)
        .iter()
        .filter(|&&from| !each.contains(from));
    let (Some(&entry), None) = (outside.next(), outside.next()) else {
        return false;
    };
    if each.latches().len() != 1 {
        return false;
    }
    let Terminator::Branch {
        condition,
        then,
        otherwise,
        ..
    } = &function.block(head).terminator
    else {
        return false;
    };
    if !each.contains(*then) || each.contains(*otherwise) {
        return false;
    }
    if leaves_early_to(function, cfg, each, *otherwise) {
        return false;
    }
    head_test(unit, function, cfg, entry, head, condition, taken) == Some(true)
}

/// Whether an edge leaving `each` from a block other than its head reaches
/// `past`, the block the head's test leaves the loop for.
fn leaves_early_to(function: &Function, cfg: &Cfg, each: &Loop, past: BlockId) -> bool {
    let mut seen = vec![false; function.blocks().len()];
    let mut stack = Vec::new();
    let mut successors = Vec::new();
    for &block in cfg.order() {
        if block == each.head() || !each.contains(block) {
            continue;
        }
        successors.clear();
        function.block(block).terminator.successors(&mut successors);
        stack.extend(successors.iter().copied().filter(|&to| !each.contains(to)));
    }
    while let Some(block) = stack.pop() {
        if block == past {
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
            match written
                .iter()
                .rev()
                .find(|(local, _)| *local == place.local)
            {
                Some(&(_, known)) => known,
                None if taken.get(LocalIdx::of(place.local)) || !holds_int(place.local) => None,
                None => constant_on_the_way_in(function, cfg, entry, place.local),
            }
        }
        Operand::Copy(_) => None,
    };
    for element in &function.block(head).elements {
        match element {
            Element::Assign(operation) => {
                let known =
                    if operation.place.projection.is_empty() && holds_int(operation.place.local) {
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
            // A local made again holds nothing, and one two operands write
            // unsequenced holds what C does not define.
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
