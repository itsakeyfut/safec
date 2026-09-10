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
//! **A pointer is a place in a frame.** Every place in this IR is rooted at a
//! local, so there is nothing else for a pointer to point at until #74, and no
//! heap and no addresses-as-numbers are needed to run one. Carrying the frame
//! is what lets a read through a pointer into a function that has returned be
//! reported rather than answered, which is the defect `docs/roadmap.md`'s
//! Phase 6 exists to catch, caught here by running.
//!
//! Nothing but tests calls this. `docs/roadmap.md` asks for no flag, and a
//! `--run` would be a second way to execute a program that Phase 3's backend
//! makes redundant.

use crate::ir::{
    BinOp, BlockId, FuncId, Operand, Place, Projection, Rvalue, Terminator, TranslationUnit, UnOp,
};
use crate::source::Span;

/// How deep a call stack may go before the run is stopped.
///
/// `int f(void) { return f(); }` lowers, and running it grows the frame stack
/// until the machine gives out. A number is what makes that a report rather
/// than a crash, and `parser::MAX_NESTING` is the same shape one phase up: a
/// bound that exists to turn a resource nobody chose into a diagnostic somebody
/// wrote.
pub const MAX_FRAMES: usize = 1 << 16;

/// What a local holds while a program runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// An integer, as wide as the IR's own constants.
    ///
    /// No narrowing to a target's width: `ir::Ty` says it holds none, and
    /// `docs/architecture.md` puts widths in the phase that lowers to LLVM.
    /// What that costs is that this cannot answer what a program does when a
    /// result stops fitting in an `int`, and [`Trap`] is where it says so.
    Int(i128),
    /// A pointer: which place, in which frame.
    Pointer {
        /// The frame the place belongs to, as an index into the call stack.
        frame: usize,
        /// The place itself, in that frame's function.
        place: Place,
    },
}

/// Why a run stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What happened, as a sentence a reader can act on.
    pub why: String,
    /// Where the operation was, where the operation had a span.
    ///
    /// A terminator other than a call carries none, and neither does the read
    /// of a local, so this is `None` more often than a diagnostic would like.
    /// What the IR knows is in `ir.rs`, and #74 is the issue for the rest.
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

/// One call, while it is running.
struct Frame {
    /// Which function this is running.
    function: FuncId,
    /// One slot per local, empty until something writes it. Reading an empty
    /// one is what C leaves indeterminate, and this stops there.
    locals: Vec<Option<Value>>,
    /// Which block is running.
    block: BlockId,
    /// Where this frame's callee writes its answer.
    destination: Option<Place>,
    /// Which block this frame resumes at when its callee returns.
    resume: Option<BlockId>,
    /// Whether this frame has returned.
    ///
    /// A frame is kept after it returns rather than dropped, so that a pointer
    /// into it can be told apart from a pointer into whatever would have taken
    /// its place.
    live: bool,
}

/// Run `entry`, and answer what it returned.
///
/// The arguments are bound to the entry function's parameters in order; a
/// program's `main` takes none. What comes back is the value in the return
/// place, or the reason the run stopped.
pub fn run(unit: &TranslationUnit, entry: FuncId, arguments: &[Value]) -> Result<Value, Trap> {
    let mut frames = vec![enter(unit, entry, arguments)?];
    let mut current = 0;

    loop {
        // A loop over a stack of frames rather than a recursive call per C
        // call: a recursion here dies of a stack overflow that nothing can
        // catch, on a program whose depth the program itself chooses. RK-008 in
        // the review knowledge bank is the entry, one layer down.
        let function = unit.function(frames[current].function);
        let block = function.block(frames[current].block);
        let operations = block.operations.clone();
        let terminator = block.terminator.clone();

        for operation in &operations {
            let value = rvalue(&frames, current, &operation.value)
                .map_err(|trap| trap.at(operation.origin.span()))?;
            let (frame, local) = resolve(&frames, current, &operation.place)
                .map_err(|trap| trap.at(operation.origin.span()))?;
            frames[frame].locals[local] = Some(value);
        }

        match terminator {
            Terminator::Goto(to) => frames[current].block = to,
            Terminator::Branch {
                condition,
                then,
                otherwise,
            } => {
                // C17 6.8.4.1 p2: the substatement runs if the expression
                // compares unequal to zero.
                let taken = match operand(&frames, current, &condition)? {
                    Value::Int(0) => otherwise,
                    Value::Int(_) => then,
                    Value::Pointer { .. } => {
                        return Err(Trap::new("a branch on a pointer, which needs a width"));
                    }
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

                frames.push(enter(unit, callee, &passed).map_err(|trap| trap.at(origin.span()))?);
                current = frames.len() - 1;
            }
            Terminator::Return => {
                let answer = frames[current].locals[0].clone();
                frames[current].live = false;

                let Some(below) = current.checked_sub(1) else {
                    return answer.ok_or_else(|| {
                        Trap::new("a function returned without writing its return place")
                    });
                };

                // A call whose value is discarded writes nowhere, so a callee
                // that returned nothing is only a problem where somebody asked
                // for the answer.
                if let Some(destination) = frames[below].destination.clone() {
                    let answer = answer.ok_or_else(|| {
                        Trap::new("a function returned without writing its return place")
                    })?;
                    let (frame, local) = resolve(&frames, below, &destination)?;
                    frames[frame].locals[local] = Some(answer);
                }

                frames[below].block = frames[below].resume.expect("a caller resumes somewhere");
                current = below;
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
fn enter(unit: &TranslationUnit, id: FuncId, arguments: &[Value]) -> Result<Frame, Trap> {
    let function = unit.function(id);
    if !function.is_defined() {
        return Err(Trap::new(
            "a call to a function whose body is not in this translation unit",
        ));
    }

    let mut locals: Vec<Option<Value>> = function.locals().map(|_| None).collect();
    for (parameter, value) in function.parameters().zip(arguments) {
        locals[parameter.index()] = Some(value.clone());
    }

    Ok(Frame {
        function: id,
        locals,
        block: function.entry(),
        destination: None,
        resume: None,
        live: true,
    })
}

/// What an operation computes.
fn rvalue(frames: &[Frame], current: usize, value: &Rvalue) -> Result<Value, Trap> {
    Ok(match value {
        Rvalue::Use(from) => operand(frames, current, from)?,
        // The one operation that turns a place into a value, and here that is
        // literal: the value is the place, and the frame it belongs to.
        Rvalue::Address(place) => Value::Pointer {
            frame: current,
            place: place.clone(),
        },
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
            let lhs = integer(operand(frames, current, lhs)?)?;
            let rhs = integer(operand(frames, current, rhs)?)?;
            Value::Int(binary(*op, lhs, rhs)?)
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
        Value::Pointer { .. } => Err(Trap::new(
            "arithmetic on a pointer, which counts elements and so needs a width",
        )),
    }
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
            let (frame, local) = resolve(frames, current, place)?;
            frames[frame].locals[local]
                .clone()
                .ok_or_else(|| Trap::new("a read of a local nothing has written"))
        }
    }
}

/// Which frame and which local a place ends up naming.
///
/// A place is a local and a walk away from it, and every step of that walk goes
/// through a pointer, which is itself a place in a frame. So following one is
/// following a chain that can cross frames, and this is where a pointer into a
/// frame that has returned is caught.
///
/// The recursion is bounded by the length of a place's projection list, which
/// the parser bounds when it builds the declarator the place came from. It is
/// not bounded by anything the program chooses at run time, which is what makes
/// a recursion safe here and not in `run`.
fn resolve(frames: &[Frame], current: usize, place: &Place) -> Result<(usize, usize), Trap> {
    let mut frame = current;
    let mut local = place.local.index();

    for projection in &place.projection {
        match projection {
            Projection::Deref => {
                let value = frames[frame].locals[local]
                    .clone()
                    .ok_or_else(|| Trap::new("a read of a local nothing has written"))?;
                let Value::Pointer {
                    frame: at,
                    place: pointee,
                } = value
                else {
                    return Err(Trap::new(
                        "a dereference of something that is not a pointer",
                    ));
                };
                if !frames[at].live {
                    return Err(Trap::new(
                        "a read through a pointer into a function that has returned",
                    ));
                }

                let (at, inner) = resolve(frames, at, &pointee)?;
                frame = at;
                local = inner;
            }
            // Nothing builds one: an array is a type the lowering refuses, and
            // a subscript is lowered as the arithmetic C17 6.5.2.1 p2 defines
            // it as. Whoever gives the IR arrays gives this an answer.
            Projection::Index(_) => {
                return Err(Trap::new("an index, which nothing builds yet"));
            }
        }
    }

    Ok((frame, local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::DiagnosticSink;
    use crate::ir::{Block, Function, Operation, Origin, Ty};
    use crate::lexer::lex;
    use crate::lowering::lower;
    use crate::parser::parse;
    use crate::sema::resolve;
    use crate::source::SourceMap;
    use crate::types::check;

    /// Compile a program the way the driver does, then run its `main`.
    ///
    /// The whole frontend is in here on purpose: what this is for is saying
    /// that a program still means what it meant after five stages have had it,
    /// and a test that started from IR could not say that.
    fn ran(text: &str) -> Result<Value, Trap> {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();

        let tokens = lex(file, sources.file(file), &mut diagnostics);
        let mut ast = parse(file, &tokens, &mut diagnostics);
        let resolution = resolve(&sources, &ast, &mut diagnostics);
        let types = check(&sources, &mut ast, &resolution, &mut diagnostics);
        let unit = lower(&sources, &ast, &resolution, &types, &mut diagnostics);
        assert!(!diagnostics.has_errors(), "the program did not compile");

        let main = unit
            .functions()
            .find(|id| sources.snippet(unit.function(*id).name) == "main")
            .expect("a function called main");

        run(&unit, main, &[])
    }

    /// The MVP program of `docs/roadmap.md`, run.
    ///
    /// This is the second clause of Phase 2's Done-when, and the number is the
    /// point: the program says 3, so the compiler has to say 3 after lowering
    /// it into another shape entirely.
    ///
    /// Mutation: swap the operands of `BinOp::Sub`, which is what the first
    /// program below is for. Mutation: have `Return` answer the local after the
    /// return place. Either fails.
    #[test]
    fn the_mvp_program_produces_three() {
        let answer = ran(
            "int add(int a, int b) {\n    return a + b;\n}\n\nint main(void) {\n    return add(1, 2);\n}\n",
        );
        assert_eq!(answer, Ok(Value::Int(3)));

        // Subtraction rather than addition, because `a + b` and `b + a` are the
        // same number and a swapped operand would go unnoticed.
        let ordered = ran(
            "int less(int a, int b) {\n    return a - b;\n}\n\nint main(void) {\n    return less(10, 4);\n}\n",
        );
        assert_eq!(ordered, Ok(Value::Int(6)));
    }

    /// Control flow, run rather than read.
    ///
    /// Mutation: have `Branch` take the `then` edge when the condition is zero.
    /// The loop runs zero times or forever, and this fails either way.
    #[test]
    fn a_loop_and_a_branch_run() {
        let summed = ran(
            "int main(void) {\n    int n;\n    int total;\n    n = 5;\n    total = 0;\n    while (n > 0) {\n        total = total + n;\n        n = n - 1;\n    }\n    return total;\n}\n",
        );
        assert_eq!(summed, Ok(Value::Int(15)));

        let chosen = ran(
            "int main(void) {\n    int n;\n    n = 2;\n    if (n > 1) {\n        n = 10;\n    } else {\n        n = 20;\n    }\n    return n;\n}\n",
        );
        assert_eq!(chosen, Ok(Value::Int(10)));

        // A short circuit is a branch, so `0 && (1 / 0)` answers rather than
        // trapping: C17 6.5.13 p4 says the right operand is not evaluated.
        let short = ran("int main(void) {\n    int z;\n    z = 0;\n    return z && (1 / z);\n}\n");
        assert_eq!(short, Ok(Value::Int(0)));
    }

    /// A pointer is a place, and writing through one writes the place.
    ///
    /// Mutation: have `Rvalue::Address` answer a copy of what the place holds.
    /// The write through the pointer lands somewhere else and this fails.
    #[test]
    fn a_write_through_a_pointer_reaches_the_place_it_names() {
        let answer = ran(
            "int main(void) {\n    int x;\n    int *p;\n    x = 1;\n    p = &x;\n    *p = 7;\n    return x;\n}\n",
        );
        assert_eq!(answer, Ok(Value::Int(7)));
    }

    /// A function that returned takes its locals with it.
    ///
    /// `docs/roadmap.md`'s Phase 6 is the analysis that will refuse this
    /// program without running it. Until then, running it says so, which is
    /// what makes this interpreter a test instrument for the analyses and not
    /// only for the frontend.
    ///
    /// Mutation: drop the `live` flag, or stop checking it. The read answers
    /// whatever is in that slot, the program looks fine, and this fails.
    #[test]
    fn a_pointer_into_a_returned_function_is_not_read() {
        let escaped = ran(
            "int *escaping(void) {\n    int x;\n    x = 1;\n    return &x;\n}\n\nint main(void) {\n    int *p;\n    p = escaping();\n    return *p;\n}\n",
        );
        let Err(trap) = escaped else {
            panic!("{escaped:?}");
        };
        assert!(trap.why.contains("has returned"), "{trap:?}");
    }

    /// What C leaves undefined stops the run.
    ///
    /// Answering a number here would make this compiler the one that decided
    /// what these programs mean.
    ///
    /// Mutation: answer `Value::Int(0)` for a read of a local nothing wrote, or
    /// for a division by zero. The matching case fails.
    #[test]
    fn undefined_behaviour_stops_and_says_why() {
        for (program, expected) in [
            (
                "int main(void) {\n    int x;\n    return x;\n}\n",
                "nothing has written",
            ),
            (
                "int main(void) {\n    int z;\n    z = 0;\n    return 1 / z;\n}\n",
                "division by zero",
            ),
            (
                "int main(void) {\n    int z;\n    z = 0;\n    return 1 % z;\n}\n",
                "remainder by zero",
            ),
        ] {
            let Err(trap) = ran(program) else {
                panic!("{program}");
            };
            assert!(trap.why.contains(expected), "{program}: {trap:?}");
        }
    }

    /// An operation this interpreter does not implement stops the run.
    ///
    /// A skipped operation makes a wrong answer look like a right one, which is
    /// the third acceptance criterion of #72 and the reason a trap covers both
    /// what C leaves undefined and what is not built yet.
    ///
    /// Mutation: treat arithmetic on a pointer as arithmetic on whatever number
    /// happens to be there, or answer zero. This fails.
    #[test]
    fn an_operation_this_does_not_implement_stops_the_run() {
        let arithmetic = ran(
            "int main(void) {\n    int x;\n    int *p;\n    x = 1;\n    p = &x;\n    return (p + 1) == 0;\n}\n",
        );
        let Err(trap) = arithmetic else {
            panic!("{arithmetic:?}");
        };
        assert!(trap.why.contains("arithmetic on a pointer"), "{trap:?}");

        let declared =
            ran("int elsewhere(void);\n\nint main(void) {\n    return elsewhere();\n}\n");
        let Err(trap) = declared else {
            panic!("{declared:?}");
        };
        assert!(
            trap.why.contains("body is not in this translation unit"),
            "{trap:?}"
        );
    }

    /// A recursion deeper than the bound is reported rather than fatal.
    ///
    /// A recursive interpreter would die of a stack overflow here, which is not
    /// a panic anything can catch, so the test would take the process with it.
    /// The frames are a `Vec` and the bound is a number, which is what makes
    /// this a value a test can assert on.
    ///
    /// Mutation: remove the check against `MAX_FRAMES`. The run allocates
    /// frames until the machine gives out, which is why the bound is written
    /// down rather than left to the operating system.
    #[test]
    fn a_recursion_without_an_end_is_stopped() {
        let forever =
            ran("int f(void) {\n    return f();\n}\n\nint main(void) {\n    return f();\n}\n");
        let Err(trap) = forever else {
            panic!("{forever:?}");
        };
        assert!(trap.why.contains("call stack deeper than"), "{trap:?}");
    }

    /// A trap points at the operation that stopped the run, where the IR knows
    /// one.
    ///
    /// Mutation: drop the `at` the operation's origin supplies. The span goes
    /// and this fails.
    #[test]
    fn a_trap_points_at_what_stopped_it() {
        let mut sources = SourceMap::new();
        let text = "int main(void) {\n    int z;\n    z = 0;\n    return 1 / z;\n}\n";
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();

        let tokens = lex(file, sources.file(file), &mut diagnostics);
        let mut ast = parse(file, &tokens, &mut diagnostics);
        let resolution = resolve(&sources, &ast, &mut diagnostics);
        let types = check(&sources, &mut ast, &resolution, &mut diagnostics);
        let unit = lower(&sources, &ast, &resolution, &types, &mut diagnostics);

        let main = unit
            .functions()
            .find(|id| sources.snippet(unit.function(*id).name) == "main")
            .expect("a main");
        let Err(trap) = run(&unit, main, &[]) else {
            panic!("a division by zero");
        };

        let at = trap.at.expect("the operation had a span");
        assert_eq!(sources.snippet(at), "1 / z");
    }

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
            operations: vec![Operation {
                place: Place::local(twice.return_place()),
                value: Rvalue::Binary {
                    op: BinOp::Add,
                    lhs: Operand::Copy(Place::local(n)),
                    rhs: Operand::Copy(Place::local(n)),
                },
                origin: Origin::Written(at),
            }],
            terminator: Terminator::Return,
        });
        let id = unit.push_function(twice);

        assert_eq!(run(&unit, id, &[Value::Int(7)]), Ok(Value::Int(14)));
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
            operations: vec![Operation {
                place: Place::local(function.return_place()),
                value: Rvalue::Use(Operand::Constant(1)),
                origin: Origin::Written(at),
            }],
            terminator: Terminator::Return,
        });
        function.fill_block(
            entry,
            Block {
                operations: Vec::new(),
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
