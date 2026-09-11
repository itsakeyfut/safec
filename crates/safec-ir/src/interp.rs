//! Running the Safety IR, so that a program can be tested without a backend.
//!
//! `docs/architecture.md` asks for this before LLVM and says why: a compiler
//! whose only way to run a program is to generate native code cannot be tested
//! without one. What this gives back is what the program computed, which is the
//! difference between "the IR looks right" and "the program still means what it
//! meant".
//!
//! **It stops rather than answering, wherever it cannot be honest.** Two kinds
//! of thing reach [`Trap`] and they are deliberately one type: what C leaves
//! undefined, and what this interpreter has not implemented. Answering a number
//! for the first would make this compiler the one that decided what `1 / 0`
//! means; skipping the second would make a wrong answer look like a right one.
//!
//! **A pointer is a local in a frame, resolved where the address was taken.**
//! Every place in this IR is rooted at a local, so there is nothing else for a
//! pointer to point at until #74, and no heap and no addresses-as-numbers are
//! needed to run one. Resolving at `&` rather than at each use is what makes
//! `int *r = &*p; p = &b;` leave `r` pointing where it pointed, which is what C
//! says and what an aliasing analysis will be checked against.
//!
//! **A frame that returns is gone, and a pointer into it says so.** The stack
//! is a stack: frames are popped, so the depth is what it costs and a run of
//! ten million calls costs one frame. What tells a stale pointer from a live
//! one is the generation stamped on the slot it names, which is why a returned
//! frame does not have to be kept to be recognised. That catches the defect
//! `docs/roadmap.md`'s Phase 6 exists to catch, caught here by running.
//!
//! What it cannot catch is a scope: the IR has no statement that says a local's
//! storage ended, so `{ int x; p = &x; }` leaves `x` alive until the function
//! returns and a read through `p` answers. #74 is the issue for that, and until
//! it lands this interpreter is honest about a frame and silent about a block.
//!
//! Nothing but tests calls this. `docs/roadmap.md` asks for no flag, and a
//! `--run` would be a second way to execute a program that Phase 3's backend
//! makes redundant.

use crate::ir::{
    BinOp, BlockId, Element, FuncId, LocalId, Operand, Place, Projection, Rvalue, Terminator,
    TranslationUnit, Ty, UnOp,
};
use crate::source::Span;

/// How deep a call stack may go before the run is stopped.
///
/// `int f(void) { return f(); }` lowers, and running it grows the stack until
/// the machine gives out. A number is what makes that a report rather than a
/// crash, and `parser::MAX_NESTING` is the same shape one phase up: a bound
/// that turns a resource nobody chose into a sentence somebody wrote.
///
/// A depth and not a count of calls. A loop that calls a function a million
/// times is a program with an answer, and refusing it would be this refusing
/// to run ordinary C.
pub const MAX_FRAMES: usize = 1 << 16;

/// How many blocks a run may enter before it is stopped.
///
/// `while (1) { }` never calls anything, so [`MAX_FRAMES`] never looks at it,
/// and a run of one is a run that does not end. Whether a program terminates is
/// not a question anything can answer, so this answers a different one: whether
/// it terminated within a bound written down here.
///
/// The cost is that a program which would have answered after more steps than
/// this is stopped instead. That is the trade a test instrument should make,
/// because the alternative is a suite that hangs rather than fails, and a
/// hanging test says nothing at all.
///
/// The number is chosen from both ends. A loop of a hundred thousand turns is
/// a few hundred thousand blocks, so an ordinary test program is nowhere near
/// it; and a test that does hit it pays for every step, measured here at about
/// two and a half million a second, so a bound sixteen times this one turned a
/// suite that ran in a tenth of a second into one that took seven.
pub const MAX_STEPS: usize = 1 << 20;

/// What a local holds while a program runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// An integer, as wide as the IR's own constants.
    ///
    /// No narrowing to a target's width: `ir::Ty` says it holds none, and
    /// `docs/architecture.md` puts widths in the phase that lowers to LLVM.
    ///
    /// So a program whose answer depends on a width gets this machine's answer
    /// rather than the target's, and gets it without being told: `INT_MAX + 1`
    /// is 2147483648 here and -2147483648 under a compiler that knows `int` is
    /// 32 bits. C17 6.5 p5 leaves that undefined, so no answer is wrong, but
    /// this one is a different program's answer and `docs/frontend.md` records
    /// it beside the rest. What [`Trap`] catches is only what an `i128` cannot
    /// hold, which is a bound of this machine rather than a rule of C.
    Int(i128),
    /// A pointer to a local of a frame.
    ///
    /// A resolved place rather than the expression that named one: `&*p` is
    /// where `p` pointed when the address was taken, and C17 6.5.3.2 p1 makes
    /// that the object, not the way it was reached.
    Pointer(Location),
}

/// Which local, in which frame, and of which call.
///
/// The generation is what makes a pointer into a frame that has returned
/// recognisable after the slot has been taken by another call: the depth is
/// reused, the number is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    /// How deep in the stack the frame sits.
    depth: usize,
    /// Which call filled that depth.
    generation: u64,
    /// Which local of it.
    local: LocalId,
}

/// Why a run stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What happened, as a sentence a reader can act on.
    pub why: String,
    /// Where the operation was, where the operation had a span.
    ///
    /// A terminator other than a call carries none, so this is `None` more
    /// often than a diagnostic would like. What the IR knows is in `ir.rs`.
    pub at: Option<Span>,
}

impl Trap {
    /// A trap with nowhere to point.
    fn new(why: impl Into<String>) -> Self {
        Self {
            why: why.into(),
            at: None,
        }
    }

    /// The same, pointing at the operation that stopped the run.
    fn at(self, at: Span) -> Self {
        Self {
            at: Some(at),
            ..self
        }
    }
}

/// What one local holds, or why it holds nothing.
///
/// Three states rather than an `Option`, because "nobody has written this" and
/// "this has no storage" are two different things a C program did and this
/// exists to say which. C17 6.2.4 p2 makes the second undefined outright: "if
/// an object is referred to outside of its lifetime, the behavior is
/// undefined". The first is an indeterminate value, which is undefined for its
/// own reasons and is not the same sentence.
#[derive(Clone, Debug)]
enum Slot {
    /// No storage. Before the scope that declares it opens, or after it closes.
    Dead,
    /// Storage, and nothing written into it.
    Unwritten,
    /// What is in it.
    Held(Value),
}

/// One call, while it is running.
struct Frame {
    /// Which function this is running.
    function: FuncId,
    /// Which call this is, counted over the whole run.
    ///
    /// Two calls at one depth are two generations, so a pointer taken in the
    /// first and read in the second is caught rather than answered.
    generation: u64,
    /// One slot per local.
    locals: Vec<Slot>,
    /// Which block is running.
    block: BlockId,
    /// Where this frame's callee writes its answer.
    destination: Option<Place>,
    /// Which block this frame resumes at when its callee returns.
    resume: Option<BlockId>,
}

/// Run `entry`, and answer what it returned.
///
/// The arguments are bound to the entry function's parameters in order; a
/// program's `main` takes none. What comes back is the value in the return
/// place, or the reason the run stopped.
///
/// # Panics
///
/// If the IR names a local or a block that its function does not have, which
/// only a hand-built [`TranslationUnit`] can do: the ids are the IR's own and
/// `ir.rs` documents the same class of panic on the accessors this uses.
pub fn run(unit: &TranslationUnit, entry: FuncId, arguments: &[Value]) -> Result<Value, Trap> {
    let mut generation = 0;
    let mut frames = vec![enter(unit, entry, arguments, generation)?];
    let mut steps = 0;

    loop {
        steps += 1;
        if steps > MAX_STEPS {
            return Err(Trap::new(format!(
                "a run that entered more than {MAX_STEPS} blocks without ending"
            )));
        }

        // A loop over a stack of frames rather than a recursive call per C
        // call: a recursion here dies of a stack overflow that nothing can
        // catch, on a program whose depth the program itself chooses. RK-008 in
        // the review knowledge bank is the entry, one layer down.
        let current = frames.len() - 1;
        let function = unit.function(frames[current].function);
        let block = function.block(frames[current].block);
        let elements = block.elements.clone();
        let terminator = block.terminator.clone();

        for element in &elements {
            // Written out rather than `..`, so that a field added to a storage
            // marker is `error[E0027]` here rather than something the run
            // quietly ignores. RK-018 is the entry.
            match element {
                Element::Assign(operation) => {
                    let value = rvalue(&frames, &operation.value)
                        .map_err(|trap| trap.at(operation.origin.span()))?;
                    let at = resolve(&frames, current, &operation.place)
                        .map_err(|trap| trap.at(operation.origin.span()))?;
                    store(&mut frames, at, value);
                }
                // Storage, and nothing in it. Entering the block again is what
                // C17 6.2.4 p6 makes a fresh lifetime, so this is a write and
                // not a check: whatever the last iteration left is gone.
                Element::StorageLive { local, origin: _ } => {
                    frames[current].locals[local.index()] = Slot::Unwritten;
                }
                Element::StorageDead { local, origin: _ } => {
                    frames[current].locals[local.index()] = Slot::Dead;
                }
            }
        }

        match terminator {
            Terminator::Goto(to) => frames[current].block = to,
            Terminator::Branch {
                condition,
                then,
                otherwise,
            } => {
                // C17 6.8.4.1 p2: the substatement runs if the expression
                // compares unequal to zero. A pointer here is a place, and a
                // place is an object, so it is never the null pointer 6.3.2.3
                // p3 makes a zero constant into: `if (p)` holds.
                let taken = match operand(&frames, current, &condition)? {
                    Value::Int(0) => otherwise,
                    Value::Int(_) | Value::Pointer(_) => then,
                };
                frames[current].block = taken;
            }
            Terminator::Call {
                callee,
                arguments,
                destination,
                then,
                origin,
            } => {
                let mut passed = Vec::new();
                for argument in &arguments {
                    passed.push(
                        operand(&frames, current, argument)
                            .map_err(|trap| trap.at(origin.span()))?,
                    );
                }

                frames[current].destination = destination;
                frames[current].resume = Some(then);

                if frames.len() >= MAX_FRAMES {
                    return Err(
                        Trap::new(format!("a call stack deeper than {MAX_FRAMES} frames"))
                            .at(origin.span()),
                    );
                }

                generation += 1;
                frames.push(
                    enter(unit, callee, &passed, generation)
                        .map_err(|trap| trap.at(origin.span()))?,
                );
            }
            Terminator::Return => {
                let answer = match frames[current].locals[0].clone() {
                    Slot::Held(value) => Some(value),
                    Slot::Unwritten | Slot::Dead => None,
                };
                // The frame is gone, and a pointer into it is stale from here
                // on: the generation on the slot is what says so once another
                // call takes the same depth.
                let returning = frames.pop().expect("the running frame");

                let Some(below) = frames.len().checked_sub(1) else {
                    // C17 5.1.2.2.3 p1: reaching the `}` that terminates `main`
                    // returns zero. The entry is what the caller is running as
                    // a program, so that is the function the rule is about, and
                    // a `main` with no `return` in it is ordinary C rather than
                    // a program with no answer.
                    return Ok(answer.unwrap_or(Value::Int(0)));
                };

                if let Some(destination) = frames[below].destination.clone() {
                    // A `void` function answers nothing, and the lowering gives
                    // its call a destination all the same, so the answer is
                    // only owed where the callee had one to give.
                    let returns = unit.function(returning.function);
                    let returns_something =
                        unit.ty(returns.local(returns.return_place())) != Ty::Void;

                    match answer {
                        Some(answer) => {
                            let at = resolve(&frames, below, &destination)?;
                            store(&mut frames, at, answer);
                        }
                        None if returns_something => {
                            return Err(Trap::new(
                                "a function returned without writing its return place",
                            ));
                        }
                        None => {}
                    }
                }

                frames[below].block = frames[below].resume.expect("a caller resumes somewhere");
            }
            // ADR-0010 put this in the IR before anything produced one, and
            // nothing does. An interpreter that guessed what it means would be
            // answering for unwinding this project has not designed.
            Terminator::Abnormal { .. } => {
                return Err(Trap::new(
                    "an edge no statement produced, which nothing builds yet",
                ));
            }
        }
    }
}

/// A frame for a call, with the arguments bound to the parameters.
fn enter(
    unit: &TranslationUnit,
    id: FuncId,
    arguments: &[Value],
    generation: u64,
) -> Result<Frame, Trap> {
    let function = unit.function(id);
    if !function.is_defined() {
        return Err(Trap::new(
            "a call to a function whose body is not in this translation unit",
        ));
    }

    // A caller that passes the wrong number is a caller with a mistake in it,
    // and `zip` would take the shorter of the two and say nothing. `types.rs`
    // checks this for every call a C program makes, so what reaches here is a
    // hand-built unit or a caller of `run`, and both would rather be told.
    if function.parameters().count() != arguments.len() {
        return Err(Trap::new(format!(
            "a call passing {} arguments to a function taking {}",
            arguments.len(),
            function.parameters().count()
        )));
    }

    // A local starts with storage and nothing in it. The ones whose scope is
    // narrower than the function are put back to `Dead` by the `StorageLive`
    // that opens their scope, which is the first thing that runs in it.
    let mut locals: Vec<Slot> = function.locals().map(|_| Slot::Unwritten).collect();
    for (parameter, value) in function.parameters().zip(arguments) {
        locals[parameter.index()] = Slot::Held(value.clone());
    }

    Ok(Frame {
        function: id,
        generation,
        locals,
        block: function.entry(),
        destination: None,
        resume: None,
    })
}

/// What an operation computes.
fn rvalue(frames: &[Frame], value: &Rvalue) -> Result<Value, Trap> {
    let current = frames.len() - 1;

    Ok(match value {
        Rvalue::Use(from) => operand(frames, current, from)?,
        // The one operation that turns a place into a value, and the place is
        // resolved here rather than remembered: what `&p[i]` names is the
        // object it reached, and reaching it again later could reach another.
        Rvalue::Address(place) => Value::Pointer(resolve(frames, current, place)?),
        // C17 6.5.3.3 p5 makes `!E` mean `(0 == E)`, which is defined for a
        // pointer and needs no width: a place is an object, so `!p` is zero.
        Rvalue::Unary {
            op: UnOp::Not,
            operand: from,
        } if matches!(operand(frames, current, from)?, Value::Pointer(_)) => Value::Int(0),
        Rvalue::Unary { op, operand: from } => {
            let value = integer(operand(frames, current, from)?)?;
            Value::Int(match op {
                UnOp::Neg => value
                    .checked_neg()
                    .ok_or_else(|| Trap::new("a negation whose result does not fit"))?,
                // C17 6.5.3.3 p5: `!x` is 1 where `x` compares equal to zero.
                UnOp::Not => i128::from(value == 0),
                UnOp::BitNot => !value,
            })
        }
        Rvalue::Binary { op, lhs, rhs } => {
            let lhs = operand(frames, current, lhs)?;
            let rhs = operand(frames, current, rhs)?;

            // Whether two pointers name one object is defined, and so is
            // whether a pointer is null. Neither counts elements, so neither
            // needs the width that pointer arithmetic does.
            match (op, &lhs, &rhs) {
                (BinOp::Eq | BinOp::Ne, Value::Pointer(_), _)
                | (BinOp::Eq | BinOp::Ne, _, Value::Pointer(_)) => {
                    Value::Int(compared(*op, &lhs, &rhs)?)
                }
                _ => Value::Int(binary(*op, integer(lhs)?, integer(rhs)?)?),
            }
        }
    })
}

/// The number a value holds, or a stop.
///
/// Arithmetic on a pointer is what this refuses. `p + 1` counts elements, which
/// takes a width, and `ir::Ty` holds none.
fn integer(value: Value) -> Result<i128, Trap> {
    match value {
        Value::Int(value) => Ok(value),
        Value::Pointer(_) => Err(Trap::new(
            "arithmetic on a pointer, which counts elements and so needs a width",
        )),
    }
}

/// Whether two values name the same thing, where one of them is a pointer.
///
/// C17 6.5.9 p6: two pointers compare equal where they point at the same
/// object. There is no null in this model, because a pointer is a place and a
/// place is an object, so a pointer compared with a zero constant is unequal,
/// which is what 6.3.2.3 p3 makes that constant mean. Comparing a pointer with
/// any other integer is a constraint 6.5.9 p2 does not allow, and nothing
/// before this stage checks it, so this stops.
fn compared(op: BinOp, lhs: &Value, rhs: &Value) -> Result<i128, Trap> {
    let same = match (lhs, rhs) {
        (Value::Pointer(lhs), Value::Pointer(rhs)) => lhs == rhs,
        (Value::Pointer(_), Value::Int(0)) | (Value::Int(0), Value::Pointer(_)) => false,
        _ => {
            return Err(Trap::new(
                "a comparison of a pointer with a number that is not zero",
            ));
        }
    };

    Ok(i128::from(match op {
        BinOp::Eq => same,
        _ => !same,
    }))
}

/// What an operator does to two numbers.
fn binary(op: BinOp, lhs: i128, rhs: i128) -> Result<i128, Trap> {
    let overflow = || Trap::new("an arithmetic result that does not fit");
    Ok(match op {
        BinOp::Mul => lhs.checked_mul(rhs).ok_or_else(overflow)?,
        // C17 6.5.5 p5 leaves division and remainder by zero undefined, and
        // 6.5 p5 leaves an overflowing signed result undefined. Answering a
        // number for either would be this compiler deciding what they mean.
        BinOp::Div => lhs
            .checked_div(rhs)
            .ok_or_else(|| Trap::new("a division by zero"))?,
        BinOp::Rem => lhs
            .checked_rem(rhs)
            .ok_or_else(|| Trap::new("a remainder by zero"))?,
        BinOp::Add => lhs.checked_add(rhs).ok_or_else(overflow)?,
        BinOp::Sub => lhs.checked_sub(rhs).ok_or_else(overflow)?,
        // 6.5.7 p3: a shift by a negative amount, or by at least the width of
        // the promoted left operand, is undefined. The width is not here, so
        // what is checked is what this machine can answer.
        BinOp::Shl => u32::try_from(rhs)
            .ok()
            .and_then(|by| lhs.checked_shl(by))
            .ok_or_else(|| Trap::new("a shift by a negative or enormous amount"))?,
        BinOp::Shr => u32::try_from(rhs)
            .ok()
            .and_then(|by| lhs.checked_shr(by))
            .ok_or_else(|| Trap::new("a shift by a negative or enormous amount"))?,
        // 6.5.8 p6 and 6.5.9 p3: a comparison yields 1 or 0, with type `int`.
        BinOp::Lt => i128::from(lhs < rhs),
        BinOp::Gt => i128::from(lhs > rhs),
        BinOp::Le => i128::from(lhs <= rhs),
        BinOp::Ge => i128::from(lhs >= rhs),
        BinOp::Eq => i128::from(lhs == rhs),
        BinOp::Ne => i128::from(lhs != rhs),
        BinOp::BitAnd => lhs & rhs,
        BinOp::BitXor => lhs ^ rhs,
        BinOp::BitOr => lhs | rhs,
    })
}

/// What an operand reads.
fn operand(frames: &[Frame], current: usize, operand: &Operand) -> Result<Value, Trap> {
    match operand {
        Operand::Constant(value) => Ok(Value::Int(*value)),
        Operand::Copy(place) => {
            let at = resolve(frames, current, place)?;
            load(frames, at)
        }
    }
}

/// Which local, in which frame, a place ends up naming.
///
/// A place is a local and a walk away from it, and every step of that walk goes
/// through a pointer, which is itself a location. So following one is following
/// a chain that can cross frames, and this is where a pointer into a frame that
/// has returned is caught.
///
/// The loop is bounded by the length of a place's projection list, which the
/// parser bounds when it reads the declarator the place came from. It is not
/// bounded by anything the program chooses while it runs, which is what makes a
/// walk safe here and not in [`run`].
fn resolve(frames: &[Frame], current: usize, place: &Place) -> Result<Location, Trap> {
    let mut at = Location {
        depth: current,
        generation: frames[current].generation,
        local: place.local,
    };

    for projection in &place.projection {
        match projection {
            Projection::Deref => match load(frames, at)? {
                Value::Pointer(pointee) => at = pointee,
                Value::Int(_) => {
                    return Err(Trap::new(
                        "a dereference of something that is not a pointer",
                    ));
                }
            },
            // Nothing builds one: an array is a type the lowering refuses, and
            // a subscript is lowered as the arithmetic C17 6.5.2.1 p2 defines
            // it as. Whoever gives the IR arrays gives this an answer.
            Projection::Index(_) => {
                return Err(Trap::new("an index, which nothing builds yet"));
            }
        }
    }

    Ok(at)
}

/// What a location holds, or a stop saying why it holds nothing.
///
/// The frame is checked before the slot, so a pointer into a call that has
/// returned is reported as that rather than as an uninitialised read: a slot
/// another call is using now holds somebody else's value, and answering it
/// would be the worst kind of right-looking answer.
fn load(frames: &[Frame], at: Location) -> Result<Value, Trap> {
    let frame = live(frames, at)?;
    match frames[frame].locals[at.local.index()].clone() {
        Slot::Held(value) => Ok(value),
        Slot::Unwritten => Err(Trap::new("a read of a local nothing has written")),
        // A different sentence from the one above on purpose. This is the
        // defect the lifetime axis of `docs/safety-model.md` exists to catch,
        // and a compiler that said "nothing has written it" would be describing
        // the wrong program.
        Slot::Dead => Err(Trap::new(
            "a read of a local whose scope has ended, through a pointer that outlived it",
        )),
    }
}

/// Write a value where a location says.
///
/// A location that has been resolved has already been checked, which is why
/// this cannot fail: [`resolve`] is the only way to make one, and the frame it
/// names cannot return between the two without the run passing through a
/// terminator.
fn store(frames: &mut [Frame], at: Location, value: Value) {
    frames[at.depth].locals[at.local.index()] = Slot::Held(value);
}

/// Which frame a location names, or a stop saying it names none.
fn live(frames: &[Frame], at: Location) -> Result<usize, Trap> {
    match frames.get(at.depth) {
        Some(frame) if frame.generation == at.generation => Ok(at.depth),
        // The depth is either past the top of the stack or holds another call
        // now. Both are the same thing to the program that kept the pointer:
        // the function it pointed into has returned.
        _ => Err(Trap::new("a pointer into a function that has returned")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Block, Function, Operation, Origin, Ty};
    use crate::source::SourceMap;

    // Every unit here is built by hand, and that is the point rather than a
    // convenience: this crate has no frontend to compile C with, so a test
    // that runs is a test that the IR is runnable on its own. The programs
    // that go through the lexer, the parser, sema and the lowering before they
    // run are in `safec`'s `tests/interp.rs`, which is the only side that can
    // compile one. ADR-0011 is the boundary they are on opposite sides of.

    /// A function built by hand, run with no frontend in it.
    ///
    /// This is the third clause of Phase 2's Done-when, and the reason it is a
    /// clause: an IR that can only be produced by this compiler's own frontend
    /// is one a Clang adapter cannot reach, and a test that needs C to say
    /// anything is a test the adapter cannot borrow.
    ///
    /// Mutation: have `enter` bind the arguments to the locals from zero rather
    /// than from the first parameter. The answer stops being seven.
    #[test]
    fn a_function_built_by_hand_runs() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int twice(int n);\n");
        let at = Span::new(file, 4, 9);

        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let mut twice = Function::new(at, int, [int]);
        let n = twice.parameters().next().expect("one parameter");
        twice.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(twice.return_place()),
                value: Rvalue::Binary {
                    op: BinOp::Add,
                    lhs: Operand::Copy(Place::local(n)),
                    rhs: Operand::Copy(Place::local(n)),
                },
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        let id = unit.push_function(twice);

        assert_eq!(run(&unit, id, &[Value::Int(7)]), Ok(Value::Int(14)));
    }

    /// A call passing the wrong number of arguments stops the run.
    ///
    /// `types.rs` checks this for every call a C program makes, so what reaches
    /// here is a hand-built unit or a caller of `run`. Binding what was passed
    /// and saying nothing about the rest would leave a parameter holding
    /// whatever the last call left, which is the worst kind of answer.
    ///
    /// Mutation: bind the arguments with `zip` and no check. The run answers 1
    /// rather than stopping and this fails.
    #[test]
    fn a_call_with_the_wrong_number_of_arguments_stops_the_run() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int twice(int n);\n");
        let at = Span::new(file, 4, 9);

        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let mut twice = Function::new(at, int, [int]);
        twice.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(twice.return_place()),
                value: Rvalue::Use(Operand::Constant(1)),
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        let id = unit.push_function(twice);

        let Err(trap) = run(&unit, id, &[]) else {
            panic!("a call with no arguments to a function with one parameter");
        };
        assert!(trap.why.contains("0 arguments"), "{trap:?}");
    }

    /// A dereference of something that is not a pointer stops the run.
    ///
    /// Nothing the frontend accepts reaches this, because `types.rs` refuses an
    /// indirection through an `int`. A hand-built unit reaches it, and so will
    /// a Clang adapter the day one exists.
    ///
    /// Mutation: read the local through the projection anyway. The run answers
    /// what the local holds and this fails.
    #[test]
    fn a_dereference_of_a_number_stops_the_run() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int f(void);\n");
        let at = Span::new(file, 4, 5);

        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(at, int, []);
        let holding = function.push_local(int);

        function.push_block(Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(holding),
                    value: Rvalue::Use(Operand::Constant(4)),
                    origin: Origin::Written(at),
                }),
                Element::Assign(Operation {
                    place: Place::local(function.return_place()),
                    value: Rvalue::Use(Operand::Copy(Place {
                        local: holding,
                        projection: vec![Projection::Deref],
                    })),
                    origin: Origin::Written(at),
                }),
            ],
            terminator: Terminator::Return,
        });
        let id = unit.push_function(function);

        let Err(trap) = run(&unit, id, &[]) else {
            panic!("a dereference of a number");
        };
        assert!(trap.why.contains("not a pointer"), "{trap:?}");
    }

    /// A call that writes nowhere runs for what it does.
    ///
    /// The lowering gives every call a destination, so this shape only reaches
    /// the interpreter from a hand-built unit today. It is what `free(p);` will
    /// be, and `ir::Terminator::Call`'s doc comment says so.
    ///
    /// Mutation: demand a destination. The run stops on a call that asked for
    /// nothing and this fails.
    #[test]
    fn a_call_that_writes_nowhere_runs() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "void nothing(void);\n");
        let at = Span::new(file, 5, 12);

        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let void = unit.push_type(Ty::Void);

        let mut callee = Function::new(at, void, []);
        callee.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        let callee = unit.push_function(callee);

        let mut caller = Function::new(at, int, []);
        let entry = caller.reserve_block();
        let after = caller.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(caller.return_place()),
                value: Rvalue::Use(Operand::Constant(7)),
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        caller.fill_block(
            entry,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Call {
                    callee,
                    arguments: Vec::new(),
                    destination: None,
                    then: after,
                    origin: Origin::Written(at),
                },
            },
        );
        let id = unit.push_function(caller);

        assert_eq!(run(&unit, id, &[]), Ok(Value::Int(7)));
    }

    /// A projection this cannot follow stops the run.
    ///
    /// Nothing builds a `Projection::Index`: an array is a type the lowering
    /// refuses, and a subscript is lowered as the arithmetic C17 6.5.2.1 p2
    /// defines it as. So the only way to reach this is to build the place by
    /// hand, which is what the interpreter's caller after #74 will do for real.
    ///
    /// Mutation: treat an index as no step at all. The read answers what the
    /// local holds, a program that indexed something gets the thing itself, and
    /// this fails.
    #[test]
    fn a_projection_this_cannot_follow_stops_the_run() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int f(void);\n");
        let at = Span::new(file, 4, 5);

        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(at, int, []);
        let holding = function.push_local(int);

        function.push_block(Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(holding),
                    value: Rvalue::Use(Operand::Constant(9)),
                    origin: Origin::Written(at),
                }),
                Element::Assign(Operation {
                    place: Place::local(function.return_place()),
                    value: Rvalue::Use(Operand::Copy(Place {
                        local: holding,
                        projection: vec![Projection::Index(Operand::Constant(0))],
                    })),
                    origin: Origin::Written(at),
                }),
            ],
            terminator: Terminator::Return,
        });
        let id = unit.push_function(function);

        let Err(trap) = run(&unit, id, &[]) else {
            panic!("an index nothing can follow");
        };
        assert!(trap.why.contains("an index"), "{trap:?}");
    }

    /// An edge no statement produced stops the run rather than being followed.
    ///
    /// ADR-0010 put the variant in the IR before anything produced one, and
    /// this is what answering for it looks like in a machine that runs: an
    /// interpreter that guessed would be answering for unwinding nobody has
    /// designed.
    ///
    /// Mutation: follow the edge like a `Goto`. The run answers rather than
    /// stopping and this fails.
    #[test]
    fn an_abnormal_edge_stops_the_run() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int f(void);\n");
        let at = Span::new(file, 4, 5);

        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(at, int, []);

        // The entry is the block whose id came first, so it is the one that has
        // to take the abnormal edge: a handler reached from nowhere would leave
        // the run answering 1 and saying nothing.
        let entry = function.reserve_block();
        let handler = function.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(function.return_place()),
                value: Rvalue::Use(Operand::Constant(1)),
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        function.fill_block(
            entry,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Abnormal { to: handler },
            },
        );
        assert_eq!(function.entry(), entry);

        let id = unit.push_function(function);
        let Err(trap) = run(&unit, id, &[]) else {
            panic!("an abnormal edge");
        };
        assert!(trap.why.contains("no statement produced"), "{trap:?}");
    }
}
