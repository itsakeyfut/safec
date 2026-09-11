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
//! **One shape escapes that and is known.** A pointer taken in one iteration of
//! a loop and read in the next is answered, not stopped: identity here is a
//! frame's generation, and a frame does not change when a block inside it is
//! entered again. C17 6.2.4 p6 makes each entry a new object, so that read is
//! undefined and this says a number for it. Telling the two instances apart
//! needs an identity per entry rather than per call, which #86 is.
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
//! A scope is caught the same way and by a different mechanism. The IR says
//! where a local's storage began and ended, so `{ int x; p = &x; } return *p;`
//! reaches a slot with no storage rather than one the frame still holds, and
//! the run stops there. See ADR-0012. Reading storage that is gone and reading
//! storage nothing has written are two sentences, because they are two things a
//! C program did.
//!
//! Nothing but tests calls this. `docs/roadmap.md` asks for no flag, and a
//! `--run` would be a second way to execute a program that Phase 3's backend
//! makes redundant.

use crate::ir::{
    BinOp, BlockId, Element, FuncId, LocalId, Operand, Place, Projection, Rvalue, Terminator,
    TranslationUnit, Ty, UnOp,
};
use crate::source::Span;
use crate::target::Integer;

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
    /// An integer, held wider than any type a target names.
    ///
    /// The carrier is wide so that the rules can be applied to it rather than
    /// suffered from it: an operation is computed here and then asked whether
    /// the type it is written into holds the result, which is C17 6.5 p5's
    /// question and not a question about this machine. `INT_MAX + 1` stops
    /// where `int` is 32 bits, and would answer where it is 64.
    ///
    /// So a value in flight can be outside the range of the type it came from
    /// or is going to. What keeps that honest is that every edge converts or
    /// checks: `rvalue` at an assignment, `enter` at a parameter, and `fits` at
    /// an operation. ADR-0013 is where the widths come from.
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
    /// No storage, because the scope that declares it has closed.
    ///
    /// Not before it opens: [`enter`] starts every local `Unwritten`, and why
    /// that is both permissive and unobservable is written there.
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
                    // What the destination is worth on this unit's target,
                    // which is what decides both the width an operation may
                    // overflow at and the type an assignment converts to.
                    // `None` for a pointer, which has neither question.
                    let ty = unit
                        .place_ty(function, &operation.place)
                        .and_then(|ty| unit.integer(ty));
                    let value = rvalue(&frames, &operation.value, ty)
                        .map_err(|trap| trap.at(operation.origin.span()))?;
                    let at = resolve(&frames, current, &operation.place)
                        .map_err(|trap| trap.at(operation.origin.span()))?;
                    store(&mut frames, at, value)
                        .map_err(|trap| trap.at(operation.origin.span()))?;
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
                            store(&mut frames, at, answer)?;
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

    // Every local starts with storage and nothing in it, whatever its scope.
    // A nested one is therefore `Unwritten` rather than `Dead` until its scope
    // has opened and closed once, which is a more permissive start than the IR
    // strictly says and is unobservable: C17 6.2.1 p7 starts a name's scope at
    // its declarator, so nothing can name the local before the `StorageLive`
    // that follows it. `goto` into a block is what ends that, and ADR-0012
    // names it as the change this moves with.
    let mut locals: Vec<Slot> = function.locals().map(|_| Slot::Unwritten).collect();
    for (parameter, value) in function.parameters().zip(arguments) {
        // C17 6.5.2.2 p7: for a prototyped function "the arguments are
        // implicitly converted, as if by assignment, to the types of the
        // corresponding parameters". As if by assignment is 6.3.1.3, which is
        // what `Rvalue::Use` already does at the other edge where a value
        // crosses into a differently typed object. Both edges or neither: a
        // parameter left holding a value its type cannot represent carries it
        // into arithmetic that then stops somewhere the value could not have
        // reached.
        let converted = match (value, unit.integer(function.local(parameter))) {
            (Value::Int(value), Some(ty)) => Value::Int(ty.convert(*value)),
            (value, _) => value.clone(),
        };
        locals[parameter.index()] = Slot::Held(converted);
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

/// What an operation computes, as the type it is written into.
///
/// `ty` is what the destination place is worth on the target, and the two kinds
/// of `Rvalue` ask two different things of it.
///
/// An arithmetic result is **checked**. C17 6.5 p5: "if the result is not
/// mathematically defined or not in the range of representable values for its
/// type, the behavior is undefined", and this stops rather than answering, the
/// same as it does for a division by zero. The type to check against is the
/// destination's, which [`crate::ir::Operation`] says is the type C performs
/// the operation at; that is an obligation on whoever built the unit, stated
/// there rather than assumed here.
///
/// A `Use` is **converted**. C17 6.5.16.1 p2 converts the right operand of an
/// assignment to the type of the assignment expression, and 6.3.1.3 says what
/// that gives. Nothing is undefined there, so nothing stops.
fn rvalue(frames: &[Frame], value: &Rvalue, ty: Option<Integer>) -> Result<Value, Trap> {
    let current = frames.len() - 1;

    Ok(match value {
        Rvalue::Use(from) => match (operand(frames, current, from)?, ty) {
            (Value::Int(value), Some(ty)) => Value::Int(ty.convert(value)),
            (value, _) => value,
        },
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
            let computed = match op {
                UnOp::Neg => value
                    .checked_neg()
                    .ok_or_else(|| Trap::new("a negation whose result does not fit"))?,
                // C17 6.5.3.3 p5: `!x` is 1 where `x` compares equal to zero.
                UnOp::Not => i128::from(value == 0),
                UnOp::BitNot => !value,
            };
            Value::Int(fits(computed, ty)?)
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
                _ => Value::Int(binary(*op, integer(lhs)?, integer(rhs)?, ty)?),
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

/// Whether a result is one the type it is written into can hold.
///
/// C17 6.5 p5 leaves an operation undefined whose result "is not in the range of
/// representable values for its type", so this stops rather than wrapping. A
/// destination with no width to check against, which is a pointer, is nothing
/// to ask about: only `Rvalue::Address` writes one and it computes no number.
fn fits(value: i128, ty: Option<Integer>) -> Result<i128, Trap> {
    match ty {
        Some(ty) if !ty.holds(value) => Err(Trap::new(format!(
            "an arithmetic result of {value}, which {} bits {} cannot hold",
            ty.bits(),
            if ty.signed() { "signed" } else { "unsigned" },
        ))),
        _ => Ok(value),
    }
}

/// What an operator does to two numbers, as the type it is written into.
fn binary(op: BinOp, lhs: i128, rhs: i128, ty: Option<Integer>) -> Result<i128, Trap> {
    let overflow = || Trap::new("an arithmetic result that does not fit");
    let computed = match op {
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
        // the promoted left operand, is undefined. The promoted left operand is
        // what the result is written into, so `ty` is that width. 6.5.7 p4 is
        // the other half and is answered below by `fits`: a left shift whose
        // value is not representable in the result type is undefined too, which
        // is what `1 << 31` is at 32 bits.
        BinOp::Shl | BinOp::Shr => {
            let width = ty.map_or(i128::BITS, Integer::bits);
            if rhs < 0 || rhs >= i128::from(width) {
                return Err(Trap::new(format!(
                    "a shift by {rhs}, where the left operand is {width} bits"
                )));
            }
            let by = u32::try_from(rhs).expect("checked against the width above");
            match op {
                BinOp::Shl => lhs.checked_shl(by).ok_or_else(overflow)?,
                _ => lhs.checked_shr(by).ok_or_else(overflow)?,
            }
        }
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
    };

    fits(computed, ty)
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

/// Write a value where a location says, or a stop saying why it cannot be.
///
/// The same two questions [`load`] asks, in the same order and for the same
/// reasons. [`resolve`] checks a frame on each step it loads *through* and not
/// on the location it hands back, so a write has to ask both itself: whether
/// the frame the pointer was taken in is still the frame at that depth, and
/// whether the local still has storage.
///
/// Neither is hypothetical. `int *leak(void) { int x; int *q; q = &x; return
/// q; } ... *p = 5;` names a depth the stack no longer has, and before this
/// asked, it was an index out of bounds rather than a stop. `{ int x; p = &x; }
/// *p = 1;` names a live frame and a dead slot, and a write let through would
/// put a value where the next scope at that depth is about to keep one, so the
/// program that pays for it is not the one that did it.
fn store(frames: &mut [Frame], at: Location, value: Value) -> Result<(), Trap> {
    let frame = live(frames, at)?;
    let slot = &mut frames[frame].locals[at.local.index()];
    if matches!(slot, Slot::Dead) {
        return Err(Trap::new(
            "a write to a local whose scope has ended, through a pointer that outlived it",
        ));
    }
    *slot = Slot::Held(value);
    Ok(())
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
    use crate::target::Target;

    /// A target, for a test that is not about the machine.
    ///
    /// Named rather than defaulted, because `TranslationUnit::new` takes one on
    /// purpose: ADR-0013 says a unit nobody said the target of is one whose
    /// `int` has no width. A test that *is* about the machine names its own.
    fn a_target() -> Target {
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple")
    }
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

        let mut unit = TranslationUnit::new(a_target());
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

        let mut unit = TranslationUnit::new(a_target());
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

        let mut unit = TranslationUnit::new(a_target());
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

    /// An unsigned destination overflows too, and says so in its own words.
    ///
    /// Built by hand, because the frontend cannot reach this shape: every
    /// arithmetic expression it lowers lands in a temporary of the promoted
    /// type, and C17 6.3.1.1 p2 promotes every integer type it has to `int`,
    /// which is signed on every row of `Target::ALL`. So the unsigned half of
    /// `fits` has no producer above it and is reachable only from a unit like
    /// this one, which is exactly what a Clang adapter or a later IR pass would
    /// build.
    ///
    /// The sentence matters as much as the stop. "8 bits signed" for an
    /// unsigned type would send a reader looking for a sign bit that is not
    /// there.
    ///
    /// Mutation: check only signed types, `ty.signed() && !ty.holds(value)`.
    /// Nothing else in the workspace fails and this does. Mutation: say
    /// "signed" whatever the type. The last assertion fails.
    #[test]
    fn an_unsigned_destination_overflows_in_its_own_words() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual(
            "t.c",
            "int f(void);
",
        );
        let at = Span::new(file, 4, 5);

        // One of the two rows of `Target::ALL` where `char` is unsigned, so
        // that `holds` has a lower bound of zero rather than of a negative
        // number.
        let mut unit = TranslationUnit::new(
            Target::from_triple("aarch64-unknown-linux-gnu").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let character = unit.push_type(Ty::Char);

        let mut function = Function::new(at, int, []);
        let narrow = function.push_local(character);
        function.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(narrow),
                // 0 - 1, which an unsigned `char` cannot hold. Written as an
                // operation rather than as a `Use`, because a `Use` would
                // convert it to 255 and that is the defined half.
                value: Rvalue::Binary {
                    op: BinOp::Sub,
                    lhs: Operand::Constant(0),
                    rhs: Operand::Constant(1),
                },
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        let id = unit.push_function(function);

        let Err(trap) = run(&unit, id, &[]) else {
            panic!("an unsigned type cannot hold -1");
        };
        assert!(trap.why.contains("8 bits unsigned"), "{trap:?}");
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

        let mut unit = TranslationUnit::new(a_target());
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

        let mut unit = TranslationUnit::new(a_target());
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

        let mut unit = TranslationUnit::new(a_target());
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
