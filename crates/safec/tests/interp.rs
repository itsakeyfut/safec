//! Programs that are compiled before they are run.
//!
//! The interpreter and its own tests are in `safec_ir`, which has no frontend
//! and cannot compile C: what it can say is that a `TranslationUnit` built by
//! hand runs, which is the property ADR-0011 exists to keep. These are the
//! other half. Each one starts from C, so it is a test of the whole pipeline
//! and of the interpreter at once, and it can only live on this side of the
//! boundary because only this side can lex.
//!
//! That is also why they say something the unit tests cannot: that a program
//! still means what it meant after five stages have had it.

use safec::diagnostics::DiagnosticSink;
use safec::lexer::lex;
use safec::lowering::lower;
use safec::parser::parse;
// Renamed, because the interpreter has a `resolve` of its own and the two
// answer different questions: one resolves a name, the other a place.
use safec::sema::resolve as resolve_names;
use safec::types::check;
use safec_ir::interp::{Trap, Value, run};
use safec_ir::source::SourceMap;

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
    let resolution = resolve_names(&sources, &ast, &mut diagnostics);
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

    let declared = ran("int elsewhere(void);\n\nint main(void) {\n    return elsewhere();\n}\n");
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

/// Two calls at one depth are two calls.
///
/// A frame stack that never popped made the caller of a returning frame
/// the frame below it in the arena, which is the previous callee once one
/// has returned. `p = malloc(...); free(p);` is this shape, and so is every
/// loop that calls anything.
///
/// Mutation: keep frames after they return and find the caller by
/// subtracting one from the running frame. The second call resumes a frame
/// that has returned, and this fails.
#[test]
fn two_calls_at_one_depth_both_return() {
    let sequential = ran(
        "int inc(int x) {\n    return x + 1;\n}\n\nint main(void) {\n    int a;\n    int b;\n    a = inc(1);\n    b = inc(2);\n    return a + b;\n}\n",
    );
    assert_eq!(sequential, Ok(Value::Int(5)));

    // A thousand calls at depth one, which a bound on the number of calls
    // ever made would refuse and a bound on the depth does not.
    let looped = ran(
        "int one(void) {\n    return 1;\n}\n\nint main(void) {\n    int n;\n    int total;\n    n = 0;\n    total = 0;\n    while (n < 1000) {\n        total = total + one();\n        n = n + 1;\n    }\n    return total;\n}\n",
    );
    assert_eq!(looped, Ok(Value::Int(1000)));
}

/// A pointer is where it pointed, not the expression that made it.
///
/// C17 6.5.3.2 p1 makes `&*p` the object `p` points at, so changing `p`
/// afterwards does not move it. An interpreter that kept the expression
/// answers `b` here, which is the aliasing axis of `docs/safety-model.md`
/// answered wrongly and in silence.
///
/// Mutation: have `Rvalue::Address` keep the place rather than resolve it.
/// This answers 2 and fails.
#[test]
fn a_pointer_holds_where_it_pointed() {
    let answer = ran(
        "int main(void) {\n    int a;\n    int b;\n    int *p;\n    int *r;\n    a = 1;\n    b = 2;\n    p = &a;\n    r = &*p;\n    p = &b;\n    return *r;\n}\n",
    );
    assert_eq!(answer, Ok(Value::Int(1)));

    // The same shape with nothing behind it: `&*p` before `p` holds
    // anything is a read of a local nothing wrote, and used to be a walk
    // that followed itself until the stack gave out.
    let circular = ran("int main(void) {\n    int *p;\n    p = &*p;\n    return *p;\n}\n");
    let Err(trap) = circular else {
        panic!("{circular:?}");
    };
    assert!(trap.why.contains("nothing has written"), "{trap:?}");
}

/// A function that returns nothing is called for what it does.
///
/// The lowering gives every call a destination, so a `void` callee never
/// writes one and a run that demanded an answer refused `free(p);` before
/// Phase 5 could ask for it.
///
/// Mutation: demand an answer whatever the callee returns. This fails.
#[test]
fn a_call_to_a_void_function_is_not_owed_an_answer() {
    let answer = ran(
        "void set(int *q) {\n    *q = 5;\n}\n\nint main(void) {\n    int x;\n    x = 1;\n    set(&x);\n    return x;\n}\n",
    );
    assert_eq!(answer, Ok(Value::Int(5)));
}

/// What C defines about a pointer without needing a width.
///
/// A branch on one, C17 6.8.4.1 p2 with 6.3.2.3 p3; `!p`, 6.5.3.3 p5; and
/// whether two of them name one object, 6.5.9 p6. None of these counts
/// elements, so refusing them for want of a width was refusing programs
/// this can answer.
///
/// Mutation: send a pointer down the arithmetic path again. Each of these
/// stops instead of answering and this fails.
#[test]
fn what_c_defines_about_a_pointer_is_answered() {
    let branched = ran(
        "int main(void) {\n    int x;\n    int *p;\n    x = 1;\n    p = &x;\n    if (p) {\n        return 1;\n    }\n    return 0;\n}\n",
    );
    assert_eq!(branched, Ok(Value::Int(1)));

    let negated = ran(
        "int main(void) {\n    int x;\n    int *p;\n    x = 1;\n    p = &x;\n    return !p;\n}\n",
    );
    assert_eq!(negated, Ok(Value::Int(0)));

    let same = ran(
        "int main(void) {\n    int x;\n    int *p;\n    int *q;\n    x = 1;\n    p = &x;\n    q = &x;\n    return p == q;\n}\n",
    );
    assert_eq!(same, Ok(Value::Int(1)));

    let other = ran(
        "int main(void) {\n    int x;\n    int y;\n    int *p;\n    int *q;\n    x = 1;\n    y = 2;\n    p = &x;\n    q = &y;\n    return p != q;\n}\n",
    );
    assert_eq!(other, Ok(Value::Int(1)));

    // A place is an object, so a pointer to one is never the null a zero
    // constant means.
    let null = ran(
        "int main(void) {\n    int x;\n    int *p;\n    x = 1;\n    p = &x;\n    return p == 0;\n}\n",
    );
    assert_eq!(null, Ok(Value::Int(0)));
}

/// Reaching the end of `main` is a value C names.
///
/// C17 5.1.2.2.3 p1 says zero, which makes `int main(void) { }` an ordinary
/// program rather than one with no answer.
///
/// Mutation: demand an answer from the entry frame. This fails.
#[test]
fn a_main_that_falls_off_the_end_answers_zero() {
    assert_eq!(ran("int main(void) {\n}\n"), Ok(Value::Int(0)));
}

/// Every operator computes what C says it computes.
///
/// The lowering has its own table saying which `BinOp` a spelling becomes,
/// and that says nothing about what the operator then does: a `Mul` that
/// added would pass it. The expected side here is written out rather than
/// derived, for the reason RK-001 gives.
///
/// Mutation: change any one arm of `binary` or of the unary match. The
/// spelling that moved fails, and the message names it.
#[test]
fn every_operator_computes_what_c_says() {
    for (spelling, expected) in [
        ("7 * 6", 42),
        ("-7 / 2", -3),
        ("-7 % 2", -1),
        ("7 + 6", 13),
        ("7 - 6", 1),
        ("1 << 5", 32),
        ("-8 >> 1", -4),
        ("7 < 6", 0),
        ("7 > 6", 1),
        ("6 <= 6", 1),
        ("6 >= 7", 0),
        ("6 == 6", 1),
        ("6 != 6", 0),
        ("12 & 10", 8),
        ("12 ^ 10", 6),
        ("12 | 10", 14),
        ("-7", -7),
        ("!7", 0),
        ("!0", 1),
        ("~7", -8),
        ("+7", 7),
    ] {
        // Through a local, so that the operands are read rather than folded
        // by anything on the way: nothing folds today and this stops that
        // from being what the test depends on.
        let program =
            format!("int main(void) {{\n    int n;\n    n = {spelling};\n    return n;\n}}\n");
        assert_eq!(ran(&program), Ok(Value::Int(expected)), "{spelling}");
    }
}

/// A shift nobody can answer stops the run.
///
/// C17 6.5.7 p3 leaves a shift by a negative amount undefined, and the
/// width that would decide the rest of it is not here.
///
/// Mutation: let a negative shift through to `checked_shl`. It answers
/// rather than stopping and this fails.
#[test]
fn a_shift_by_a_negative_amount_stops_the_run() {
    let shifted = ran("int main(void) {\n    int by;\n    by = 0 - 1;\n    return 1 << by;\n}\n");
    let Err(trap) = shifted else {
        panic!("{shifted:?}");
    };
    assert!(trap.why.contains("negative or enormous"), "{trap:?}");
}

/// A callee that answers nothing where the caller asked stops the run.
///
/// C17 6.9.1 p12 leaves that undefined where the caller uses the value, and
/// the caller asking is exactly what a destination is.
///
/// Mutation: answer zero instead. The run answers a number for a program
/// that computed none, and this fails.
#[test]
fn a_callee_with_no_answer_stops_the_caller() {
    let missing = ran(
        "int nothing(void) {\n    int x;\n    x = 1;\n}\n\nint main(void) {\n    return nothing();\n}\n",
    );
    let Err(trap) = missing else {
        panic!("{missing:?}");
    };
    assert!(
        trap.why.contains("without writing its return place"),
        "{trap:?}"
    );
}

/// A run that does not end is stopped rather than left running.
///
/// `MAX_FRAMES` never looks at a loop, because a loop calls nothing. A test
/// instrument that hangs says nothing at all, and a suite that hangs says
/// less than one that fails.
///
/// Mutation: remove the step bound. This test runs until somebody kills it,
/// which is what the bound is written down to prevent.
#[test]
fn a_run_that_does_not_end_is_stopped() {
    let forever = ran(
        "int main(void) {\n    int n;\n    n = 1;\n    while (n) {\n        n = 1;\n    }\n    return n;\n}\n",
    );
    let Err(trap) = forever else {
        panic!("{forever:?}");
    };
    assert!(trap.why.contains("without ending"), "{trap:?}");
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
    let resolution = resolve_names(&sources, &ast, &mut diagnostics);
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

/// A trap on a call points at the call.
///
/// The operation traps carry the operation's origin; a call carries its
/// own, and it is the only terminator that does.
///
/// Mutation: drop the span the call arm supplies. The trap has nowhere to
/// point and this fails.
#[test]
fn a_trap_on_a_call_points_at_the_call() {
    let mut sources = SourceMap::new();
    let text = "int elsewhere(void);\n\nint main(void) {\n    return elsewhere();\n}\n";
    let file = sources.add_virtual("t.c", text);
    let mut diagnostics = DiagnosticSink::new();

    let tokens = lex(file, sources.file(file), &mut diagnostics);
    let mut ast = parse(file, &tokens, &mut diagnostics);
    let resolution = resolve_names(&sources, &ast, &mut diagnostics);
    let types = check(&sources, &mut ast, &resolution, &mut diagnostics);
    let unit = lower(&sources, &ast, &resolution, &types, &mut diagnostics);

    let main = unit
        .functions()
        .find(|id| sources.snippet(unit.function(*id).name) == "main")
        .expect("a main");
    let Err(trap) = run(&unit, main, &[]) else {
        panic!("a call to a function with no body");
    };

    let at = trap.at.expect("the call had a span");
    assert_eq!(sources.snippet(at), "elsewhere()");
}

/// A pointer that outlived the scope it pointed into stops the run.
///
/// The defect the lifetime axis of `docs/safety-model.md` exists to catch, and
/// the reason ADR-0012 put a storage marker in the block: before it, this
/// program and the same one without the braces were the same IR, so the
/// interpreter answered 42 for both. Now the read finds a slot with no storage
/// and stops, which is what a compiler that cannot prove the program should do
/// rather than answer.
///
/// Mutation: treat `Element::StorageDead` as a no-op in `interp::run`. The run
/// answers 42 and this fails. Mutation: drop the `Slot::Dead` arm from `load`
/// and answer the value anyway; the same.
#[test]
fn a_read_through_a_pointer_to_dead_storage_stops_the_run() {
    let dangling = ran("int main(void) { int *p; { int x; x = 42; p = &x; } return *p; }\n");
    let Err(trap) = dangling else {
        panic!("a read of storage whose scope ended: {dangling:?}");
    };
    assert!(trap.why.contains("scope has ended"), "{trap:?}");

    // The same program without the braces is defined, and still answers.
    assert_eq!(
        ran("int main(void) { int *p; int x; x = 42; p = &x; return *p; }\n"),
        Ok(Value::Int(42)),
    );
}

/// Storage that is gone and storage nothing has written are two sentences.
///
/// A compiler that exists to say what a program did wrong should not describe
/// a dangling read as an uninitialised one. C17 6.2.4 p2 makes the first
/// undefined by referring to an object outside its lifetime; the second is an
/// indeterminate value, which is a different rule and a different fix.
///
/// Mutation: give `Slot::Dead` the sentence `Slot::Unwritten` has, or merge the
/// two states back into an `Option`. This fails.
#[test]
fn dead_storage_and_uninitialised_storage_are_not_one_sentence() {
    let dangling = ran("int main(void) { int *p; { int x; x = 42; p = &x; } return *p; }\n");
    let unwritten = ran("int main(void) { int x; return x; }\n");

    let (Err(dangling), Err(unwritten)) = (dangling, unwritten) else {
        panic!("both stop");
    };
    assert_ne!(dangling.why, unwritten.why);
    assert!(
        unwritten.why.contains("nothing has written"),
        "{unwritten:?}"
    );
}

/// A local in a loop body is a new object on each iteration.
///
/// C17 6.2.4 p6 ends its lifetime when the block does and begins it again on
/// the next entry, so the second iteration writes to storage that is there.
/// Without the `StorageLive` half of the pair this program stops on the second
/// iteration instead of answering.
///
/// Mutation: stop emitting `StorageLive` in the lowering, or make the
/// interpreter ignore it. The run stops and this fails.
#[test]
fn a_loop_body_that_declares_something_runs_more_than_once() {
    assert_eq!(
        ran(
            "int main(void) {\n    int n; int s;\n    n = 0; s = 0;\n    while (n < 3) { int x; x = n; s = s + x; n = n + 1; }\n    return s;\n}\n"
        ),
        Ok(Value::Int(3)),
    );
}

/// A write through a pointer to dead storage stops the run too.
///
/// C17 6.2.4 p2 makes referring to an object outside its lifetime undefined,
/// and does not say "reading". A write that was let through would be worse than
/// a read that was: it puts a value into a slot the next scope at that depth is
/// about to use, so the program that pays for it is not the one that did it.
///
/// It is also what makes `StorageLive` observable at all. Without this check a
/// write revives a dead slot, so the second iteration of a loop works whether or
/// not anything said its storage began again, and
/// `a_loop_body_that_declares_something_runs_more_than_once` guards nothing.
///
/// Mutation: drop the `Slot::Dead` arm from `store`. This fails, and so does
/// the loop test once `StorageLive` is made a no-op.
#[test]
fn a_write_through_a_pointer_to_dead_storage_stops_the_run() {
    let dangling = ran("int main(void) { int *p; { int x; x = 1; p = &x; } *p = 2; return 0; }\n");
    let Err(trap) = dangling else {
        panic!("a write to storage whose scope ended: {dangling:?}");
    };
    assert!(trap.why.contains("a write to a local"), "{trap:?}");
    assert!(trap.why.contains("scope has ended"), "{trap:?}");
}

/// A write through a pointer into a function that has returned stops the run.
///
/// The read form of this was already answered, because `load` asks which frame
/// a location names before it reads the slot. `store` did not ask, and indexed
/// the frame stack directly: this program was an `index out of bounds` panic
/// rather than a stop, on C the frontend accepts and lowers without a word.
///
/// `resolve` is not where the check belongs. It validates each frame it loads
/// *through* while following a projection, and hands back the location it
/// reached without asking about that one, which is exactly the location being
/// written.
///
/// Mutation: drop the `live` call from `store` and index `frames[at.depth]`.
/// This panics rather than failing, which still counts: the suite goes red.
#[test]
fn a_write_through_a_pointer_into_a_returned_function_stops_the_run() {
    let escaped = ran(
        "int *leak(void) { int x; int *q; x = 1; q = &x; return q; }\nint main(void) { int *p; p = leak(); *p = 5; return 0; }\n",
    );
    let Err(trap) = escaped else {
        panic!("a write into a frame that has returned: {escaped:?}");
    };
    assert!(trap.why.contains("has returned"), "{trap:?}");
}
