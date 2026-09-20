//! The memory check, against IR built by hand.
//!
//! `safec_ir::memory` answers a `Finding` rather than a diagnostic so that it
//! can be read without a frontend, and this is the file that spends that. Every
//! program below is written as blocks and terminators rather than as C, so what
//! is under test is the lattice rather than the lowering: a case that goes
//! through the parser proves both at once and says which of them broke only by
//! where the diff lands.
//!
//! The corpus under `crates/safec/tests/cases/` holds the same shapes written
//! as C, which is the other half. RK-033 in the review knowledge bank is why
//! both exist: a feature guarded at one stage is unguarded at every stage that
//! reads it.

use safec_ir::analysis::Conclusion;
use safec_ir::ir::{
    BinOp, Block, BlockId, Element, FuncId, Function, LocalId, Operand, Operation, Origin, Place,
    Projection, Rvalue, Terminator, TranslationUnit, Ty, TyId,
};
use safec_ir::memory::{self, Kind, Unproven};
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

/// Six call sites and the two names the check reads.
struct Names {
    free: Span,
    malloc: Span,
    function: Span,
    /// In the order they appear in the file, so `at[0]` is earlier than `at[1]`
    /// by the rule the join orders spans with.
    at: [Span; 7],
    /// What a branch's controlling expression is, where one is needed and
    /// nothing asserts on it.
    ///
    /// `Terminator::Branch` carries a span so that a dereference in a condition
    /// has somewhere to point. `a_dereference_in_a_condition_is_a_use` is the
    /// one test that reaches it, and it uses an `at` because it asserts on
    /// where the caret lands. Everywhere else the condition is a constant and
    /// the field still has to be filled, and filling it with one of `at` would
    /// read as if it were under test.
    asked: Span,
    /// A callee that is neither `free` nor `malloc`.
    ///
    /// What it is called does not matter beyond not being one of those two,
    /// and that is the whole of what makes it opaque to this check.
    helper: Span,
}

/// A file whose bytes the check can read a callee's name out of.
///
/// The check recognises `free` and `malloc` by the text their declaration's
/// name span covers, so a test that wants one has to put the word in a file.
/// The single letters after them are call sites: `Origin` carries a span, and
/// the rule that ends the walk orders them, so two frees have to be in two
/// places for that ordering to be about anything. `helper` is last because
/// adding a word anywhere else moves every span after it.
fn sources() -> (SourceMap, Names) {
    let mut map = SourceMap::new();
    let file = map.add_virtual("t.c", "free malloc f a b c d e g h i helper\n");

    let names = Names {
        free: Span::new(file, 0, 4),
        malloc: Span::new(file, 5, 11),
        function: Span::new(file, 12, 13),
        at: [
            Span::new(file, 14, 15),
            Span::new(file, 16, 17),
            Span::new(file, 18, 19),
            Span::new(file, 20, 21),
            Span::new(file, 22, 23),
            Span::new(file, 24, 25),
            Span::new(file, 28, 29),
        ],
        asked: Span::new(file, 26, 27),
        helper: Span::new(file, 30, 36),
    };

    (map, names)
}

struct Callees {
    free: FuncId,
    malloc: FuncId,
    helper: FuncId,
}

/// A unit, the function under test, its `int`, and the callees it can name.
fn a_unit(names: &Names, parameters: usize) -> (TranslationUnit, Function, TyId, Callees) {
    let mut unit = TranslationUnit::new(
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
    );
    let int = unit.push_type(Ty::Int);
    let void = unit.push_type(Ty::Void);

    let callees = Callees {
        free: unit.push_function(Function::declaration(names.free, void, [int])),
        malloc: unit.push_function(Function::declaration(names.malloc, int, [])),
        helper: unit.push_function(Function::declaration(names.helper, void, [int])),
    };

    let function = Function::new(names.function, int, vec![int; parameters]);
    (unit, function, int, callees)
}

/// `free(local);`, ending the block.
///
/// `destination: None`, which is what the IR means by a call that returns
/// nothing. The frontend writes `Some` into a `void` temporary instead, which
/// is #135, and the corpus cases are what read that shape.
fn free(callees: &Callees, local: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![],
        terminator: Terminator::Call {
            callee: callees.free,
            arguments: vec![Operand::Copy(Place::local(local))],
            destination: None,
            then,
            origin: Origin::Written(at),
        },
    }
}

/// `local = malloc();`, ending the block.
fn malloc(callees: &Callees, local: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![],
        terminator: Terminator::Call {
            callee: callees.malloc,
            arguments: vec![],
            destination: Some(Place::local(local)),
            then,
            origin: Origin::Written(at),
        },
    }
}

/// `helper(local);`, ending the block.
///
/// A call this check cannot read, which is how a test makes what a local holds
/// unproven without taking the local's address. The two are not
/// interchangeable: an address taken is a standing fact about the local, so
/// `Known::reached_by` answers `Reached::Lost` for it ever after, while a call
/// marks the sites it was handed and nothing else.
fn helper(callees: &Callees, local: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![],
        terminator: Terminator::Call {
            callee: callees.helper,
            arguments: vec![Operand::Copy(Place::local(local))],
            destination: None,
            then,
            origin: Origin::Written(at),
        },
    }
}

/// `to = from;`, in a block that falls through to `then`.
fn copy(to: LocalId, from: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(to),
            value: Rvalue::Use(Operand::Copy(Place::local(from))),
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Goto(then),
    }
}

/// A branch on a constant, in a block with nothing else in it.
///
/// The condition is a constant wherever what a test is about is where two
/// paths meet rather than what decides which is taken, which is every test
/// here but `a_dereference_in_a_condition_is_a_use`. `asked` is the span the
/// terminator carries for a diagnostic to point at, and nothing reaching this
/// builder asserts on it.
fn branch(then: BlockId, otherwise: BlockId, asked: Span) -> Block {
    branch_on(Operand::Constant(1), then, otherwise, asked)
}

/// The same, where what decides is worth writing out.
fn branch_on(condition: Operand, then: BlockId, otherwise: BlockId, asked: Span) -> Block {
    Block {
        elements: vec![],
        terminator: Terminator::Branch {
            condition,
            then,
            otherwise,
            origin: Origin::Written(asked),
        },
    }
}

fn returns() -> Block {
    Block {
        elements: vec![],
        terminator: Terminator::Return,
    }
}

/// The same block, with a sequence point in front of it.
///
/// Every free in these programs is its own full expression, so C17 6.8 p4
/// puts a sequence point between it and whatever comes next, and the C
/// frontend emits an [`Element::Sequenced`] at the head of the block the call
/// flows into. A free this check has not seen sequenced is not yet a proof,
/// because C17 6.5 p3 leaves the rest of a full expression unordered against
/// the call; see ADR-0022.
///
/// **So an IR built without these gets suspicions where it would have had
/// proofs.** That is the bias working rather than failing: `docs/c-family.md`
/// asks that another frontend be able to build this IR, and one that forgets
/// where C sequences should lose its certainty rather than keep it.
///
/// **The boundary *before* a free counts too, and it did not used to.** A read
/// with no marker between it and a later free is a read that free is unordered
/// against, so it is carried forwards and reported unproven, which is ADR-0023
/// and is the same bias seen from the other side. A program whose read is in a
/// statement of its own therefore needs one here as well, and a test that
/// leaves it out is asking about an IR where C has ordered nothing.
fn after_the_statement(at: Span, mut block: Block) -> Block {
    block.elements.insert(
        0,
        Element::Sequenced {
            origin: Origin::Generated(at),
        },
    );
    block
}

/// What the check concluded about one function.
fn concluded(
    mut unit: TranslationUnit,
    sources: &SourceMap,
    function: Function,
) -> Vec<memory::Finding> {
    unit.push_function(function);
    memory::findings(sources, &unit)
}

/// A value freed twice is proved unsafe, and both frees can be named.
///
/// The headline of `docs/safety-model.md`'s memory axis, written as IR: a
/// call to `malloc` whose result reaches two calls to `free`.
///
/// Mutation: have a `free` leave the sites it reaches alone. Nothing is
/// reported and this fails, which is the direction that matters: it is the
/// compiler going quiet about the defect it was built to find.
///
/// Mutation: report the span of the second free as the earlier one. `freed`
/// stops being the first call and this fails on that field.
#[test]
fn a_value_freed_twice_is_unsafe() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);

    let allocate = function.reserve_block();
    let first = function.reserve_block();
    let second = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], first));
    function.fill_block(first, free(&callees, held, names.at[1], second));
    function.fill_block(
        second,
        after_the_statement(names.at[1], free(&callees, held, names.at[2], exit)),
    );
    function.fill_block(exit, after_the_statement(names.at[2], returns()));

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::DoubleFree);
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].at, names.at[2]);
    assert_eq!(found[0].freed, Some(names.at[1]));
    assert_eq!(found[0].made, Some(names.at[0]), "where the `malloc` was");
}

/// Freeing through a copy is the same allocation, and is proved rather than
/// suspected.
///
/// This is what keying on the allocation rather than on the local buys: both
/// locals reach the one site the `malloc` made, so the second free is an error
/// and not a warning. A check that gave each local its own answer could only
/// say it might be one.
///
/// Mutation: give a copy's destination a fresh site instead of the source's
/// sites. Nothing is reported and this fails.
#[test]
fn a_value_freed_through_a_copy_is_unsafe() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let alias = function.push_local(int);

    let allocate = function.reserve_block();
    let aliased = function.reserve_block();
    let first = function.reserve_block();
    let second = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], aliased));
    function.fill_block(aliased, copy(alias, held, names.at[1], first));
    function.fill_block(first, free(&callees, alias, names.at[2], second));
    function.fill_block(
        second,
        after_the_statement(names.at[2], free(&callees, held, names.at[3], exit)),
    );
    function.fill_block(exit, after_the_statement(names.at[3], returns()));

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].freed, Some(names.at[2]));
}

/// A parameter is an allocation site of its own.
///
/// Nothing in the function allocates, so without this there is no site to mark
/// and `void f(int *p) { free(p); free(p); }` is a double free reported by
/// nobody. That is the bottom row of the failure list in `CLAUDE.md`, which is
/// why the site list is calls **and** parameters rather than calls alone.
///
/// Mutation: leave the parameters out of `on_entry`. Nothing is reported and
/// this fails.
#[test]
fn a_parameter_is_an_allocation_site() {
    let (sources, names) = sources();
    let (unit, mut function, _int, callees) = a_unit(&names, 1);
    let held = function.parameters().next().expect("one parameter");

    let first = function.reserve_block();
    let second = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(first, free(&callees, held, names.at[0], second));
    function.fill_block(
        second,
        after_the_statement(names.at[0], free(&callees, held, names.at[1], exit)),
    );
    function.fill_block(exit, after_the_statement(names.at[1], returns()));

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
}

/// Two allocations are two sites, so freeing each once says nothing.
///
/// The case the design was sent back over: a check that answered about locals
/// had to make one free poison every other pointer to stay sound, and this
/// program is what that costs. It is how a C program holding two things is
/// written every time.
///
/// Mutation: have a free move every site to freed rather than the ones its
/// arguments reach. This reports a warning on correct code and fails.
#[test]
fn two_allocations_are_two_sites() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let first_held = function.push_local(int);
    let second_held = function.push_local(int);

    let one = function.reserve_block();
    let two = function.reserve_block();
    let free_one = function.reserve_block();
    let free_two = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(one, malloc(&callees, first_held, names.at[0], two));
    function.fill_block(two, malloc(&callees, second_held, names.at[1], free_one));
    function.fill_block(free_one, free(&callees, first_held, names.at[2], free_two));
    function.fill_block(
        free_two,
        after_the_statement(names.at[2], free(&callees, second_held, names.at[3], exit)),
    );
    function.fill_block(exit, after_the_statement(names.at[3], returns()));

    let found = concluded(unit, &sources, function);

    assert!(found.is_empty(), "{found:?}");
}

/// Where two frees meet, the diagnostic names the earlier of them.
///
/// The rule ADR-0016 asked somebody to write down. It is not what makes the
/// walk end here, which `a_free_that_may_not_be_the_first_is_unproven` says;
/// it is what makes the answer the same whichever order the solver happened to
/// reach the two arms in.
///
/// Mutation: in `SiteState::joined`, keep `other`'s span rather than the
/// earlier one. `freed` becomes the second arm's call and this fails on that
/// field, with nothing else in the suite noticing.
#[test]
fn a_join_names_the_earlier_free() {
    let (sources, names) = sources();
    let (unit, mut function, _int, callees) = a_unit(&names, 1);
    let held = function.parameters().next().expect("one parameter");

    let entry = function.reserve_block();
    let first_arm = function.reserve_block();
    let second_arm = function.reserve_block();
    let joined = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(entry, branch(first_arm, second_arm, names.asked));
    // The later span is on the arm the solver reaches first, so a rule that
    // kept whichever arrived would answer differently from one that orders
    // them. Without that the test would pass under both.
    function.fill_block(first_arm, free(&callees, held, names.at[1], joined));
    function.fill_block(second_arm, free(&callees, held, names.at[0], joined));
    function.fill_block(
        joined,
        after_the_statement(names.at[1], free(&callees, held, names.at[2], exit)),
    );
    function.fill_block(exit, after_the_statement(names.at[2], returns()));

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].at, names.at[2]);
    assert_eq!(found[0].freed, Some(names.at[0]), "the earlier of the two");
}

/// Where a local holds either of two allocations and both were freed, the
/// earlier free is the one named.
///
/// Different from `a_join_names_the_earlier_free`, which orders two spans
/// inside one site: this orders across two sites, which is a second place the
/// same rule has to be applied and which nothing else reaches. Every other case
/// that joins two allocations frees them at one call, so the two spans are
/// equal and the ordering would be about nothing.
///
/// Both allocations are freed before the branch, so both sites are freed on
/// every path. **That is what makes this an error rather than a warning**: the
/// value joins which site a local may hold separately from what is known about
/// each site, so a program that frees one allocation on each arm loses the
/// correlation between the two and comes back `Unknown`. Sound, and less than
/// a reader might expect; this shape is the one that keeps a proof.
///
/// Mutation: in `reported`, keep the last freed span rather than the earlier
/// one. This fails on `freed` and the rest of the suite does not notice.
#[test]
fn a_free_of_either_of_two_allocations_names_the_earlier() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let one = function.push_local(int);
    let other = function.push_local(int);

    let allocate_one = function.reserve_block();
    let allocate_two = function.reserve_block();
    let free_one = function.reserve_block();
    let free_two = function.reserve_block();
    let pick = function.reserve_block();
    let take_one = function.reserve_block();
    let take_two = function.reserve_block();
    let joined = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(
        allocate_one,
        malloc(&callees, one, names.at[0], allocate_two),
    );
    function.fill_block(allocate_two, malloc(&callees, other, names.at[1], free_one));
    function.fill_block(free_one, free(&callees, one, names.at[2], free_two));
    function.fill_block(
        free_two,
        after_the_statement(names.at[2], free(&callees, other, names.at[3], pick)),
    );

    function.fill_block(
        pick,
        after_the_statement(names.at[3], branch(take_one, take_two, names.asked)),
    );
    function.fill_block(take_one, copy(held, one, names.at[4], joined));
    function.fill_block(take_two, copy(held, other, names.at[4], joined));

    function.fill_block(joined, free(&callees, held, names.at[5], exit));
    function.fill_block(exit, after_the_statement(names.at[5], returns()));

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].at, names.at[5]);
    assert_eq!(
        found[0].freed,
        Some(names.at[2]),
        "the earlier of the two frees"
    );
}

/// A free reaching a place where the value was not freed is neither proved nor
/// disproved.
///
/// The shape ADR-0016 measured as a hang, built to see whether this lattice has
/// the same problem. **It does not**, and that is worth a test rather than an
/// assumption: `Unknown` is a top per site, so the span that would have
/// alternated is absorbed before it can, and keeping whichever span arrived
/// leaves this passing. `a_join_names_the_earlier_free` is what holds the rule
/// that record asked for; this holds the answer.
///
/// Mutation: make `Live` joined with `Freed` answer `Freed` rather than
/// `Unknown`. Both arms then claim the program is wrong on a path where it may
/// not be, and this fails on the conclusion.
#[test]
fn a_free_that_may_not_be_the_first_is_unproven() {
    let (sources, names) = sources();
    let (unit, mut function, _int, callees) = a_unit(&names, 1);
    let held = function.parameters().next().expect("one parameter");

    let entry = function.reserve_block();
    let header = function.reserve_block();
    let first_arm = function.reserve_block();
    let second_arm = function.reserve_block();
    let joined = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(
        entry,
        Block {
            elements: vec![],
            terminator: Terminator::Goto(header),
        },
    );
    function.fill_block(header, branch(first_arm, second_arm, names.asked));
    function.fill_block(first_arm, free(&callees, held, names.at[0], joined));
    function.fill_block(second_arm, free(&callees, held, names.at[1], joined));
    function.fill_block(
        joined,
        after_the_statement(names.at[0], branch(header, exit, names.asked)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    // Both arms free something the other arm may already have freed, and which
    // of them did is what the loop makes unanswerable. Two calls, two findings,
    // neither of them a claim that the program is wrong.
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .all(|found| found.conclusion == Conclusion::Unknown),
        "{found:?}"
    );
}

/// `to = *through;`, in a block that falls through to `then`.
fn read(to: LocalId, through: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(to),
            value: Rvalue::Use(Operand::Copy(deref(through))),
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Goto(then),
    }
}

/// `*through = from;`, in a block that falls through to `then`.
fn write(through: LocalId, from: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![Element::Assign(Operation {
            place: deref(through),
            value: Rvalue::Use(Operand::Copy(Place::local(from))),
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Goto(then),
    }
}

/// `to = &of;`, in a block that falls through to `then`.
///
/// The one shape that makes a local's sites unknown: anything holding the
/// address can write a different allocation into it, and this check does not
/// follow what a pointer points at.
fn address_of(to: LocalId, of: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(to),
            value: Rvalue::Address(Place::local(of)),
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Goto(then),
    }
}

/// `to = &*through;`, in a block that falls through to `then`.
fn address_of_deref(to: LocalId, through: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(to),
            value: Rvalue::Address(deref(through)),
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Goto(then),
    }
}

/// `*local`, as a place.
fn deref(local: LocalId) -> Place {
    Place {
        local,
        projection: vec![Projection::Deref],
    }
}

/// Reading through a pointer after it was freed is proved unsafe, and reading
/// through it before is not reported at all.
///
/// Both halves in one function because the second is what stops the first from
/// being met by reporting every dereference: a check that answered `Unsafe` for
/// any `*p` would pass a test holding only the free half.
///
/// Mutation: report a dereference whose sites are all live. The first read is
/// reported too and the length assertion fails.
///
/// Mutation: answer nothing for what an `Rvalue::Use` reads. Nothing is
/// reported and this fails, which is the direction that matters.
#[test]
fn a_read_through_a_freed_pointer_is_unsafe() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let live = function.reserve_block();
    let release = function.reserve_block();
    let dangling = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], live));
    function.fill_block(live, read(value, held, names.at[1], release));
    // The read is a statement of its own, so C17 6.8 p4 orders it before the
    // free and this says so. Without it the free is unordered against the read
    // above and reports it unproven, which is the right answer to an IR that
    // has not said where C sequences and the wrong subject for this test.
    function.fill_block(
        release,
        after_the_statement(names.at[1], free(&callees, held, names.at[2], dangling)),
    );
    function.fill_block(
        dangling,
        after_the_statement(names.at[2], read(value, held, names.at[3], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UseAfterFree);
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].at, names.at[3]);
    assert_eq!(found[0].freed, Some(names.at[2]));
    assert_eq!(found[0].made, Some(names.at[0]));
}

/// So is writing through it, which is a different place in the IR.
///
/// A read puts the projection in an operand and a write puts it in the
/// operation's destination, so one rule covering both is two pieces of code,
/// and a test that only read would leave half of it unguarded. Writing is also
/// the half that corrupts the heap rather than only observing it.
///
/// Mutation: answer nothing for an operation's destination. Nothing is
/// reported and this fails.
#[test]
fn a_write_through_a_freed_pointer_is_unsafe() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let release = function.reserve_block();
    let dangling = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], release));
    function.fill_block(release, free(&callees, held, names.at[1], dangling));
    function.fill_block(
        dangling,
        after_the_statement(names.at[1], write(held, value, names.at[2], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UseAfterFree);
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].at, names.at[2]);
}

/// A dereference of a pointer this check follows no allocation for says
/// nothing.
///
/// The asymmetry with the free half, which answers `Unknown` for the same
/// silence. A pointer with no allocation behind it is an uninitialised pointer
/// or one into storage that is not the heap, and both are defects with checks
/// of their own that do not exist. Warning here would fire on every `*p` whose
/// pointer came from anywhere this does not follow, which is most of them.
///
/// Mutation: answer `Unknown` where a dereferenced place reaches no site, as
/// the free half does. This warns about a program it knows nothing about and
/// fails.
#[test]
fn a_dereference_of_a_pointer_with_no_allocation_says_nothing() {
    let (sources, names) = sources();
    let (unit, mut function, int, _callees) = a_unit(&names, 0);
    let value = function.push_local(int);
    let other = function.push_local(int);

    let entry = function.reserve_block();
    let exit = function.reserve_block();

    // Nothing has allocated into `value`, and the function has no parameters,
    // so it reaches no site at all rather than reaching one that is live.
    function.fill_block(entry, read(other, value, names.at[0], exit));
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert!(found.is_empty(), "{found:?}");
}

/// Taking the address of a dereference is not a dereference.
///
/// C17 6.5.3.2 p3: where the operand of `&` is the result of a unary `*`,
/// neither operator is evaluated and the result is as if both were omitted. So
/// `&*p` reads nothing through `p`, and reporting it would be a use of a freed
/// value in a program that never touched one.
///
/// **The C frontend no longer builds this**, because it applies the same clause
/// one layer up and folds `&*p` to `p`. The IR still expresses an address of a
/// projected place, another frontend may build one, and the check has to
/// answer for it, so the guard stays where the shape can still be written.
///
/// Mutation: have `Rvalue::Address` answer the place whose address is taken.
/// This reports a use and fails.
#[test]
fn taking_the_address_of_a_dereference_is_not_a_use() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let taken = function.push_local(int);

    let allocate = function.reserve_block();
    let release = function.reserve_block();
    let address = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], release));
    function.fill_block(release, free(&callees, held, names.at[1], address));
    function.fill_block(
        address,
        after_the_statement(
            names.at[1],
            address_of_deref(taken, held, names.at[2], exit),
        ),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert!(found.is_empty(), "{found:?}");
}

/// A use after a free on one path only is suspected rather than proved.
///
/// `docs/safety-model.md`'s middle conclusion, on this check: the program may
/// be correct and this cannot say that it is, so `--deny-unknown` is what turns
/// it into a refusal.
///
/// Mutation: treat a site that is `Unknown` as freed. This becomes an error
/// about a program the check proved nothing about, which is the false-positive
/// direction, and fails on the conclusion.
#[test]
fn a_use_after_a_free_on_one_path_is_unproven() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let decide = function.reserve_block();
    let release = function.reserve_block();
    let joined = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], decide));
    function.fill_block(decide, branch(release, joined, names.asked));
    function.fill_block(release, free(&callees, held, names.at[1], joined));
    function.fill_block(
        joined,
        after_the_statement(names.at[1], read(value, held, names.at[2], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[2]);
    // Neither span is carried, because there is no one free that every
    // execution reaching here went through, and so no one allocation beside it.
    assert_eq!(found[0].freed, None);
    assert_eq!(found[0].made, None);
}

/// Where a use follows two allocations that were both freed, the free is named
/// and the allocation is not.
///
/// `points_to` is a may-set, so two sites both freed proves the use is unsafe
/// while proving nothing about which allocation it was. Naming one of them puts
/// a caret on a line only one path reached, and a caret in the wrong place is
/// worse than none.
///
/// Mutation: in `SiteState::joined`, keep `here`'s `made` rather than
/// collapsing two that disagree. `made` becomes the first arm's `malloc` and
/// this fails on that field. The join rather than the fold in `verdict`,
/// measured: a site is the local a call writes into, so two `malloc`s into one
/// local are one site and the two spans meet at the join. What guards the same
/// rule in the fold is the corpus case
/// `a_free_of_either_of_two_locals_names_no_allocation`, where two locals reach
/// one free and the two spans meet across two sites instead. That was
/// `a_branch_that_allocates_either_way` here until a review measured it and
/// found it holds under `made = from`, because one of its two sites is
/// `Live(None)` by the time the fold runs and the rule has nothing to do.
#[test]
fn a_use_after_two_allocations_names_no_allocation() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let decide = function.reserve_block();
    let first_arm = function.reserve_block();
    let first_free = function.reserve_block();
    let second_arm = function.reserve_block();
    let second_free = function.reserve_block();
    let joined = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(decide, branch(first_arm, second_arm, names.asked));
    function.fill_block(first_arm, malloc(&callees, held, names.at[0], first_free));
    function.fill_block(first_free, free(&callees, held, names.at[1], joined));
    function.fill_block(second_arm, malloc(&callees, held, names.at[2], second_free));
    function.fill_block(second_free, free(&callees, held, names.at[3], joined));
    function.fill_block(
        joined,
        after_the_statement(names.at[1], read(value, held, names.at[4], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].freed, Some(names.at[1]), "the earlier of the two");
    assert_eq!(found[0].made, None, "the two arms do not agree");
}

/// A free reaching one site that was freed and one the check gave up on is
/// unproven, not proved.
///
/// The other half of the rule `a_free_where_one_of_two_allocations_is_live`
/// holds. That one has a site this check proved is live; this one has a site it
/// proved nothing about, and the two have to answer the same way for the same
/// reason: `points_to` is a may-set, so what is true of one member of it is not
/// true of the value. Nothing else in the suite reaches the combination,
/// measured, because every other case that leaves a site `Unknown` leaves every
/// site the free reaches `Unknown`, and then there is no span to name and the
/// answer is the same either way.
///
/// Mutation: in `verdict`, `let proved = !live;`. This becomes an error naming
/// a free the program may never reach, and nothing else in the suite fails.
#[test]
fn a_free_where_one_site_is_unknown_is_unproven() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 1);
    let given = function.parameters().next().expect("one parameter");
    let held = function.push_local(int);
    let either = function.push_local(int);

    let allocate = function.reserve_block();
    let release = function.reserve_block();
    let decide = function.reserve_block();
    let arm = function.reserve_block();
    let from_the_arm = function.reserve_block();
    let otherwise = function.reserve_block();
    let joined = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], release));
    function.fill_block(release, free(&callees, held, names.at[1], decide));
    function.fill_block(
        decide,
        after_the_statement(names.at[1], branch(arm, otherwise, names.asked)),
    );
    // One arm frees the parameter and the other does not, which is what leaves
    // its site `Unknown` where they meet.
    function.fill_block(arm, free(&callees, given, names.at[2], from_the_arm));
    function.fill_block(
        from_the_arm,
        after_the_statement(names.at[2], copy(either, given, names.at[3], joined)),
    );
    function.fill_block(otherwise, copy(either, held, names.at[4], joined));
    function.fill_block(joined, free(&callees, either, names.at[5], exit));
    function.fill_block(exit, after_the_statement(names.at[5], returns()));

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].at, names.at[5]);
    assert_eq!(
        found[0].conclusion,
        Conclusion::Unknown,
        "one site was freed and the other was given up on"
    );
    assert_eq!(found[0].freed, None);
    assert_eq!(found[0].made, None);
}

/// Where one site is allocated on two arms, the join names neither.
///
/// The `Live` half of [`same`]'s rule, which its `Freed` half is guarded for by
/// `a_use_after_two_allocations_names_no_allocation`. The frontend cannot write
/// this: a site is the local a call's result lands in and each call gets its
/// own temporary, so two allocations reaching one site is a shape only IR built
/// by hand reaches. The rule answers for it anyway because the type allows it,
/// and a rule nothing exercises is a rule that is wrong the day something does.
///
/// Mutation: in `SiteState::joined`, `(Self::Live(here), Self::Live(_there)) =>
/// Self::Live(here)`. `made` becomes the first arm's `malloc` and this fails on
/// that field, with nothing else in the suite noticing.
#[test]
fn a_site_allocated_on_two_arms_names_no_allocation() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);

    let decide = function.reserve_block();
    let first_arm = function.reserve_block();
    let second_arm = function.reserve_block();
    let joined = function.reserve_block();
    let again = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(decide, branch(first_arm, second_arm, names.asked));
    // One local, two calls, so one site carrying two origins where they meet.
    function.fill_block(first_arm, malloc(&callees, held, names.at[0], joined));
    function.fill_block(second_arm, malloc(&callees, held, names.at[1], joined));
    function.fill_block(joined, free(&callees, held, names.at[2], again));
    function.fill_block(
        again,
        after_the_statement(names.at[2], free(&callees, held, names.at[3], exit)),
    );
    function.fill_block(exit, after_the_statement(names.at[3], returns()));

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].freed, Some(names.at[2]));
    assert_eq!(found[0].made, None, "the two arms do not agree");
}

/// Freeing a site twice does not lose where it was allocated.
///
/// The transfer rewrites a site's state at every free, so the second one reads
/// the state the first wrote. Taking `made` from a `Freed` as well as from a
/// `Live` is what keeps the third thing to reach that site able to name the
/// allocation, and nothing reached a site three times until this.
///
/// Mutation: in the `Callee::Frees` arm, `SiteState::Freed { .. } => None`.
/// The use loses its `allocated here` and this fails on `made`.
#[test]
fn a_second_free_keeps_where_the_allocation_was() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let release = function.reserve_block();
    let again = function.reserve_block();
    let dangling = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], release));
    function.fill_block(release, free(&callees, held, names.at[1], again));
    function.fill_block(
        again,
        after_the_statement(names.at[1], free(&callees, held, names.at[2], dangling)),
    );
    function.fill_block(
        dangling,
        after_the_statement(names.at[2], read(value, held, names.at[3], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    // The second free is one finding and the read after it is the other.
    assert_eq!(found.len(), 2, "{found:?}");
    let used = found
        .iter()
        .find(|finding| finding.kind == Kind::UseAfterFree)
        .expect("the read is reported");
    assert_eq!(used.at, names.at[3]);
    assert_eq!(used.made, Some(names.at[0]), "still the `malloc`");
}

/// A dereference in a controlling expression is a use, read out of the
/// terminator rather than out of an element.
///
/// The corpus holds this shape written as C, four times over, because four
/// lowering paths reach it. This holds the half that belongs to this crate: a
/// condition that is a place with a projection is read from the branch's own
/// `Origin`, with no frontend between. RK-033 in the review knowledge bank is
/// why both exist, and a review found this one missing: every other condition
/// in this file is a constant, so the arm was exercised only through the
/// corpus, which proves the lowering and the check at once and says which of
/// them broke only by where the diff lands.
///
/// Mutation: answer `None` for a `Branch` in `dereferenced_in_terminator`.
/// This fails, and so does every corpus case with a dereference in a condition.
#[test]
fn a_dereference_in_a_condition_is_a_use() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);

    let allocate = function.reserve_block();
    let release = function.reserve_block();
    let decide = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], release));
    function.fill_block(release, free(&callees, held, names.at[1], decide));
    // Both arms go to the same block: what is under test is the condition, and
    // where control goes afterwards is not part of it.
    function.fill_block(
        decide,
        after_the_statement(
            names.at[1],
            branch_on(Operand::Copy(deref(held)), exit, exit, names.at[2]),
        ),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UseAfterFree);
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].at, names.at[2], "the controlling expression");
    assert_eq!(found[0].freed, Some(names.at[1]));
    assert_eq!(found[0].made, Some(names.at[0]));
}

/// `*local;`, in a block that falls through to `then`.
fn evaluate(local: LocalId, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![Element::Evaluate {
            place: deref(local),
            origin: Origin::Written(at),
        }],
        terminator: Terminator::Goto(then),
    }
}

/// A place evaluated for its side effects is a use.
///
/// The corpus holds this written as C six times over, because six spellings
/// reach it. This holds the half that belongs to this crate: an element saying
/// a place was evaluated is read by the check with no frontend between, and a
/// corpus case proves the lowering and the check at once and says which of them
/// broke only by where the diff lands. RK-033 in the review knowledge bank is
/// why both exist, and a review of the sibling found this half missing.
///
/// Mutation: answer `None` for an `Element::Evaluate` in
/// `dereferenced_in_element`. This fails, and so does every corpus case that
/// throws a dereference away.
#[test]
fn a_place_evaluated_for_nothing_is_a_use() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);

    let allocate = function.reserve_block();
    let release = function.reserve_block();
    let discarded = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], release));
    function.fill_block(release, free(&callees, held, names.at[1], discarded));
    function.fill_block(
        discarded,
        after_the_statement(names.at[1], evaluate(held, names.at[2], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UseAfterFree);
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].at, names.at[2]);
    assert_eq!(found[0].freed, Some(names.at[1]));
    assert_eq!(found[0].made, Some(names.at[0]));
}

/// Evaluating a place moves no site, so a free after one is still the first.
///
/// The transfer moves nothing for this element and that is the answer rather
/// than an omission: evaluating a place writes nowhere. What it does do is
/// record the read, which every element does and which the marker below orders
/// before the frees. Nothing else in the suite reaches the arm, because every
/// other test that builds one is about what the element is read for rather than
/// about what it does to the sites.
///
/// Mutation: have the transfer clear the sites of the place it evaluates, which
/// is what an arm written by copying its neighbour would do. The free stops
/// reaching a site, the double free is not found, and this fails.
#[test]
fn evaluating_a_place_moves_no_site() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);

    let allocate = function.reserve_block();
    let discarded = function.reserve_block();
    let first = function.reserve_block();
    let second = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], discarded));
    function.fill_block(discarded, evaluate(held, names.at[1], first));
    // `*p;` is a statement, so the read is ordered before the first free. See
    // `after_the_statement`, whose doc comment says what leaving it out asks
    // about instead.
    function.fill_block(
        first,
        after_the_statement(names.at[1], free(&callees, held, names.at[2], second)),
    );
    function.fill_block(
        second,
        after_the_statement(names.at[2], free(&callees, held, names.at[3], exit)),
    );
    function.fill_block(exit, after_the_statement(names.at[3], returns()));

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::DoubleFree);
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].freed, Some(names.at[2]));
    assert_eq!(
        found[0].made,
        Some(names.at[0]),
        "the evaluation lost nothing"
    );
}

/// A proof at a caret replaces the suspicion already standing there.
///
/// **Two dereferences of one place can share a caret**, because both operands
/// of a `&&` or a `||` are written into one temporary at the whole
/// expression's span, and `used` collapses such a pair into one report. Which
/// of the two survives is the question: the first read here is unproven,
/// because a call this check cannot read was handed the pointer and may have
/// freed it, and the second is proved, because a `free` writes `Freed` over an
/// unknown site. Keeping the first was a warning and exit 0 on a use of a freed
/// value this check had proved.
///
/// **The call is what makes the first read unproven, and an address taken
/// would not do.** It was written that way until `Known::reached_by` landed:
/// an address taken is a standing fact about the local, so both reads answer
/// `Reached::Lost` and there is no proof left to replace the suspicion with.
/// A call marks the sites it was handed, and a later `free` writes over them.
///
/// **Written as IR rather than as C on purpose.** The C that reaches this
/// today does so because the lowering gives both operands of a `||` the whole
/// expression's span, and #147 is open to narrow exactly that. A guard a
/// caret-precision change can retire is not a guard for a rule about what this
/// check is allowed to be silent about, and `docs/c-family.md` asks that
/// another frontend be able to build this IR with no C frontend present.
///
/// **An unrelated report comes first on purpose.** What `said` carries for a
/// caret is where in `concluded` the report standing there is, which is not the
/// caret's own position in `said`: anything reported before the pair sits
/// between the two numbers. The double free of `other` is that anything, and
/// without it both numbers are zero and taking the wrong one is invisible.
///
/// Mutation: have `supersedes` answer `false` for `(Unknown, Unsafe)`. The
/// conclusion stays `Unknown` and this fails.
///
/// Mutation: read the position in `said` rather than the index it carries,
/// `let index = standing;`. The proof overwrites the double free of `other`
/// instead of the suspicion, so a finding is destroyed and this fails on the
/// length.
#[test]
fn a_proof_replaces_the_suspicion_at_one_caret() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let other = function.push_local(int);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let allocate_other = function.reserve_block();
    let release_other = function.reserve_block();
    let again = function.reserve_block();
    let allocate = function.reserve_block();
    let handed = function.reserve_block();
    let suspect = function.reserve_block();
    let release = function.reserve_block();
    let proof = function.reserve_block();
    let exit = function.reserve_block();

    // A defect of its own, reported before the pair below and nothing to do
    // with it. It is here so that the report standing at the pair's caret is
    // not the first thing in `concluded`.
    function.fill_block(
        allocate_other,
        malloc(&callees, other, names.at[0], release_other),
    );
    function.fill_block(release_other, free(&callees, other, names.at[1], again));
    function.fill_block(
        again,
        after_the_statement(names.at[1], free(&callees, other, names.at[2], allocate)),
    );

    function.fill_block(
        allocate,
        after_the_statement(names.at[2], malloc(&callees, held, names.at[3], handed)),
    );
    function.fill_block(handed, helper(&callees, held, names.at[4], suspect));
    function.fill_block(suspect, read(value, held, names.at[5], release));
    function.fill_block(release, free(&callees, held, names.at[6], proof));
    // The same span as the unproven read, which is what makes the two one
    // report and is the whole of what this test is about.
    function.fill_block(
        proof,
        after_the_statement(names.at[6], read(value, held, names.at[5], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    // Three, and the first has to survive untouched. Freeing a site nothing
    // can prove anything about is itself a `DoubleFree` this check cannot rule
    // out, which is the third; that is correct and is not what this test is
    // for, and asserting on it rather than filtering it away is what says so.
    assert_eq!(found.len(), 3, "{found:?}");

    assert_eq!(found[0].kind, Kind::DoubleFree, "not the one replaced");
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].at, names.at[2]);

    assert_eq!(found[1].kind, Kind::UseAfterFree);
    assert_eq!(
        found[1].conclusion,
        Conclusion::Unsafe,
        "the proof survives"
    );
    assert_eq!(found[1].at, names.at[5], "one caret, not two");
    assert_eq!(found[1].freed, Some(names.at[6]));
    assert_eq!(
        found[1].made, None,
        "the call lost where the allocation came from"
    );

    assert_eq!(found[2].kind, Kind::DoubleFree);
    assert_eq!(found[2].conclusion, Conclusion::Unknown);
    assert_eq!(found[2].at, names.at[6]);
}

/// A suspicion does not displace the proof already standing at a caret.
///
/// The other direction of the same rule, and the one a collapse gets wrong by
/// replacing whenever the pair disagrees rather than only when the new report
/// proves what the standing one could not. RK-038 in the review knowledge bank
/// is why both are written: a rule that collapses two disagreeing values has
/// two mutations, and a case that reaches one of them says nothing about the
/// other.
///
/// Mutation: have `supersedes` answer `true` for `(Unsafe, Unknown)`. The
/// conclusion drops to `Unknown` and the spans it carries go with it, so this
/// fails three times over.
#[test]
fn a_suspicion_does_not_displace_the_proof_at_one_caret() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let escape = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let release = function.reserve_block();
    let proof = function.reserve_block();
    let escaped = function.reserve_block();
    let suspect = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], release));
    function.fill_block(release, free(&callees, held, names.at[1], proof));
    function.fill_block(
        proof,
        after_the_statement(names.at[1], read(value, held, names.at[2], escaped)),
    );
    function.fill_block(escaped, address_of(escape, held, names.at[3], suspect));
    function.fill_block(suspect, read(value, held, names.at[2], exit));
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UseAfterFree);
    assert_eq!(found[0].conclusion, Conclusion::Unsafe, "the proof stands");
    assert_eq!(found[0].at, names.at[2]);
    assert_eq!(found[0].freed, Some(names.at[1]), "and keeps its spans");
    assert_eq!(found[0].made, Some(names.at[0]));
}
/// `to = from + by;`, in a block that falls through to `then`.
///
/// The addition in a local of its own, which is the shape the C frontend
/// builds for a subscript whose offset is not zero. At zero it builds nothing:
/// see [`an_unfolded_zero_offset_is_a_shape_this_check_does_not_follow`].
fn stepped_by(to: LocalId, from: LocalId, by: i128, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(to),
            value: Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(Place::local(from)),
                rhs: Operand::Constant(by),
            },
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Goto(then),
    }
}

/// An offset of zero that survived into the IR is a shape this check does not
/// follow.
///
/// **A boundary, not a goal.** The program below is a use after free and this
/// answers nothing about it, under every flag. What makes that tolerable is
/// that no IR the C frontend builds has this shape: ADR-0021 folds a zero
/// pointer offset away where the IR is built, so `pp[0]` and `*pp` arrive as
/// one shape and this arrives from nowhere. `docs/c-family.md` is where that
/// obligation is written down, because the reader who needs it is whoever
/// writes the second frontend.
///
/// This is here so the boundary has a name. A frontend that stops folding, or
/// a check taught to read the zero, fails a named test rather than passing
/// quietly, and whoever deletes it has to say which of those two they did.
///
/// Mutation: none from inside this file. What it holds is the absence of a
/// rule rather than a rule. The fold it rests on is guarded in the corpus, by
/// `a_subscript_write_is_the_write_it_is_defined_as` for the pointer case and
/// `a_zero_added_to_an_integer_keeps_its_operation` for the type test, and
/// this test is what fails if that fold is ever moved back in here.
#[test]
fn an_unfolded_zero_offset_is_a_shape_this_check_does_not_follow() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let p = function.push_local(int);
    let q = function.push_local(int);
    let pp = function.push_local(int);
    let stepped = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let alias = function.reserve_block();
    let unfolded = function.reserve_block();
    let store = function.reserve_block();
    let release = function.reserve_block();
    let after = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, q, names.at[0], alias));
    function.fill_block(alias, address_of(pp, p, names.at[1], unfolded));
    // The addition the lowering folds away, written out.
    function.fill_block(unfolded, stepped_by(stepped, pp, 0, names.at[2], store));
    function.fill_block(store, write(stepped, q, names.at[3], release));
    function.fill_block(release, free(&callees, q, names.at[4], after));
    function.fill_block(
        after,
        after_the_statement(names.at[4], read(value, p, names.at[5], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    // Nothing at all. The write through `stepped` is not followed, so nothing
    // records that `p` holds what `q` held, and the read after the free asks
    // about an allocation this check is not carrying. Written `*pp = q;`, the
    // same program is one `UseAfterFree`, unproven because `p`'s address
    // escaped and ADR-0017 keeps it out of a proof. Unproven is still an error
    // under `--deny-unknown`; this is silence under every flag.
    assert!(found.is_empty(), "{found:?}");
}

/// `*through = from + by;`, ending the block.
///
/// The arithmetic written straight into a place, which the C frontend never
/// builds: it puts the addition in a temporary and copies it out, so the write
/// it emits is a copy. `docs/c-family.md` asks that another frontend be able to
/// build this IR, and this is the shape one would.
fn offset_into(through: LocalId, from: LocalId, by: i128, at: Span, then: BlockId) -> Block {
    Block {
        elements: vec![Element::Assign(Operation {
            place: deref(through),
            value: Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(Place::local(from)),
                rhs: Operand::Constant(by),
            },
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Goto(then),
    }
}

/// An offset that moves the pointer does not carry where a write through it
/// lands.
///
/// **The one place this question is asked that no C program reaches.** `*pp =
/// qq + 7;` puts a pointer seven past `z` into `p`, so a later write through
/// `p` does not land in `z`. The arm that answers this carried the edge
/// through any arithmetic at all, while the arm one level up had just been
/// taught to drop it, and the two disagreeing was invisible because only a
/// hand-built IR reaches this one.
///
/// **The check does not tell one offset from another.** Every
/// `Rvalue::Binary` drops the edge, a zero included; this name says what its
/// own program does, not what the rule distinguishes. Where the distinction
/// lives is the lowering, and what is lost without it is
/// [`an_unfolded_zero_offset_is_a_shape_this_check_does_not_follow`].
///
/// **Observed through `q` rather than through `z`.** `z` had its address
/// taken, so ADR-0017 answers `Reached::Lost` for it wherever a report is
/// made and it can never be the subject of a proof; a test that asked about
/// `z` could not tell the two answers apart. `q` escaped nowhere, so what it
/// holds is provable, and carrying the edge is what would put `q`'s
/// allocation into `z` for `free(z)` to take.
///
/// Mutation: drop the `reached.writes_to.fill(false);` from the
/// `Rvalue::Binary` arm of the `written` match in `Allocations::element`.
/// `free(z)` then frees what `q` holds, the read through `q` is a proved use
/// after free, and this fails on the kind.
#[test]
fn an_offset_that_moves_the_pointer_carries_no_edge() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let z = function.push_local(int);
    let p = function.push_local(int);
    let q = function.push_local(int);
    let pp = function.push_local(int);
    let qq = function.push_local(int);
    let value = function.push_local(int);

    let hold = function.reserve_block();
    let alias = function.reserve_block();
    let offset = function.reserve_block();
    let allocate = function.reserve_block();
    let store = function.reserve_block();
    let release = function.reserve_block();
    let after = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(hold, address_of(qq, z, names.at[0], alias));
    function.fill_block(alias, address_of(pp, p, names.at[1], offset));
    // Seven past `z`, which is not `z`.
    function.fill_block(offset, offset_into(pp, qq, 7, names.at[2], allocate));
    function.fill_block(allocate, malloc(&callees, q, names.at[3], store));
    function.fill_block(store, write(p, q, names.at[4], release));
    function.fill_block(release, free(&callees, z, names.at[5], after));
    function.fill_block(
        after,
        after_the_statement(names.at[5], read(value, q, names.at[6], exit)),
    );
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    // One finding, and which one is the whole of this test. `free(z)` frees
    // something this check was not following, which it cannot rule out; what
    // it must not do is free the allocation `q` holds, because nothing put
    // that allocation in `z`.
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::DoubleFree, "not a use after free");
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[5]);
}

/// Nothing, in a block that falls through to `then`.
///
/// So that [`after_the_statement`] has somewhere to put a sequence point that
/// is the only thing in its block, which is what one arm of a branch needs in
/// order to differ from the other in that alone.
fn nothing(then: BlockId) -> Block {
    Block {
        elements: vec![],
        terminator: Terminator::Goto(then),
    }
}

/// A free sequenced on one arm and not on the other is not a proof.
///
/// **The one place the join of two frees is asked this, and no C program
/// reaches it.** The frontend ends every full expression with a sequence
/// point, so a free always meets one before any branch can join two of them,
/// and the two flags always agree by the time they meet. The IR allows them to
/// differ and `docs/c-family.md` asks that another frontend be able to build
/// it, so the rule is answered here rather than left to the arm nobody
/// exercises.
///
/// A path that reached here with the order still open is a path on which this
/// is not a proof, so the conjunction is the only answer that keeps the
/// weaker path's doubt. RK-044 is the entry: an invariant on a lattice value
/// has to hold after the join, and a flag that grew on one side would be a
/// proof assembled out of two halves neither of which had one.
///
/// Mutation: make `Freeing::joined` answer `self.sequenced || other.sequenced`.
/// The read becomes a proved use after free and this fails on the conclusion.
#[test]
fn a_free_sequenced_on_one_arm_only_is_not_a_proof() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let decide = function.reserve_block();
    let left = function.reserve_block();
    let sequenced = function.reserve_block();
    let right = function.reserve_block();
    let open = function.reserve_block();
    let join = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], decide));
    function.fill_block(decide, branch(left, right, names.asked));

    // One arm ends the full expression its free is in, the way the frontend
    // does.
    function.fill_block(left, free(&callees, held, names.at[1], sequenced));
    function.fill_block(sequenced, after_the_statement(names.at[1], nothing(join)));

    // The other does not, which is the only difference between them.
    function.fill_block(right, free(&callees, held, names.at[2], open));
    function.fill_block(open, nothing(join));

    function.fill_block(join, read(value, held, names.at[3], exit));
    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UseAfterFree);
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(
        found[0].unproven,
        Some(Unproven::Unsequenced),
        "the order is what is open"
    );
    // The earlier of the two, which is the rule the span half of the join
    // keeps while the flag half takes the weaker answer.
    assert_eq!(found[0].freed, Some(names.at[1]));
}

/// A call writing straight into a local whose address escaped proves nothing.
///
/// **The one place an escape is applied that no C program reaches.** Taking a
/// local's address makes whatever it is given afterwards unproven, and a local
/// is given something in three places: a copy, pointer arithmetic, and a
/// call's destination. The frontend writes every call into a fresh temporary
/// and copies it out, so a C program always arrives through the copy; the IR
/// says a call may write anywhere, and `docs/c-family.md` asks that another
/// frontend be able to build this without the C one present.
///
/// The free is what makes the omission visible: a free of an unknown site is
/// reported, and a free of a live one is not, so dropping the rule turns this
/// from one finding into none.
///
/// **Freed through a second local that shares the allocation**, because
/// freeing `held` itself would prove nothing about this rule. `held`'s address
/// escaped, so `Known::reached_by` answers `Reached::Lost` for it however the
/// sites are marked, and the report would stand with the rule deleted. What
/// the rule is about is the allocation rather than the local, and only a
/// holder that did not escape can ask about the allocation alone.
///
/// Mutation: drop `unproved` from the call's destination in the terminator's
/// transfer. The site stays live, the free through `shared` is an ordinary
/// one, nothing is reported at all, and this fails on the length. That
/// direction is the point: what the rule is holding off here is silence, not a
/// wrong answer.
#[test]
fn a_call_into_a_local_whose_address_escaped() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let escape = function.push_local(int);
    let shared = function.push_local(int);

    let allocate = function.reserve_block();
    let escaped = function.reserve_block();
    let again = function.reserve_block();
    let share = function.reserve_block();
    let release = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], escaped));
    function.fill_block(escaped, address_of(escape, held, names.at[1], again));
    // Straight into `held`, which is the shape the frontend never builds.
    function.fill_block(again, malloc(&callees, held, names.at[2], share));
    function.fill_block(share, copy(shared, held, names.at[3], release));
    function.fill_block(release, free(&callees, shared, names.at[4], exit));
    function.fill_block(exit, after_the_statement(names.at[4], returns()));

    let found = concluded(unit, &sources, function);

    // One free of one allocation, and it is still not something this check can
    // rule out, because anything holding the address could have put a freed
    // pointer there between the call and here. Without the escape outliving
    // the call, the site is live, the free is ordinary, and nothing is
    // reported at all.
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::DoubleFree);
    assert_eq!(
        found[0].conclusion,
        Conclusion::Unknown,
        "the escape outlived the call that wrote over it"
    );
    assert_eq!(found[0].at, names.at[4]);
}

/// A read of what a site used to name is not a read of what it names now.
///
/// **No C program reaches this and a frontend can build it.** A site is the
/// local a call writes into, and the lowering gives every call a fresh
/// temporary, so two allocations in one full expression are two sites; here
/// they are one, which is what makes the read above the second call name a
/// site the second call has taken over. Left standing, the free below would be
/// reported against a read of an allocation it has nothing to do with, and the
/// caret would sit on a line that read something else.
///
/// Nothing separates the read from the free, so the reads carried forwards are
/// what this is about: with the rule the entry names a site that is gone and is
/// dropped with it, and the free is an ordinary first free of a live
/// allocation.
///
/// Mutation: leave `Known::pending` alone in `Known::reborn`. The read is still
/// carried, the free meets it, and an unproven use after free is reported that
/// no execution can perform, so the count below is 1 and this fails.
#[test]
fn a_read_of_a_site_handed_to_a_second_allocation_is_not_carried_to_its_free() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let live = function.reserve_block();
    let again = function.reserve_block();
    let release = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], live));
    function.fill_block(live, read(value, held, names.at[1], again));
    // The same local, so the same site: this is where it stops naming the
    // allocation the read above went through.
    function.fill_block(again, malloc(&callees, held, names.at[2], release));
    function.fill_block(release, free(&callees, held, names.at[3], exit));
    function.fill_block(exit, after_the_statement(names.at[3], returns()));

    let found = concluded(unit, &sources, function);

    assert!(found.is_empty(), "{found:?}");
}

/// A read in a call's own argument, with no marker, is a suspicion this check
/// keeps.
///
/// **A boundary, not a goal.** `free(p + *p)` is a program C defines: C17
/// 6.5.2.2 p10's first sentence puts a sequence point after the arguments and
/// before the call, so the read is ordered before the free and there is
/// nothing to report. This answers `Unknown` about it, because the IR below
/// says nothing about that point.
///
/// ADR-0026 is what the C frontend emits so that it does:
/// `Element::ArgumentsEvaluated`, between the argument operations and the
/// call. `docs/c-family.md` is where the obligation is written down, because
/// the reader who needs it is whoever writes the second frontend, and this is
/// here so the boundary has a name. The direction is the tolerable one, a
/// suspicion rather than a proof, which is the same bias
/// [`after_the_statement`] documents for the other marker.
///
/// Mutation: emit an `Element::ArgumentsEvaluated` before the call, which is
/// what the frontend does. Nothing is reported and this fails, which is the
/// half that says the element is what the answer turns on.
#[test]
fn a_read_in_an_argument_with_no_marker_is_reported_unproven() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let allocate = function.reserve_block();
    let argument = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], argument));

    // The read and the free in one block with nothing between them, which is
    // the shape `free(p + *p)` has: the argument is computed by an element and
    // handed to the call that ends the block.
    let mut evaluated = free(&callees, held, names.at[2], exit);
    evaluated.elements = read(value, held, names.at[1], exit).elements;
    function.fill_block(argument, evaluated);

    function.fill_block(exit, returns());

    let found = concluded(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UseAfterFree);
    assert_eq!(found[0].conclusion, Conclusion::Unknown);
    assert_eq!(found[0].at, names.at[1], "the read, not the call");
    assert_eq!(
        found[0].unproven,
        Some(Unproven::Unsequenced),
        "and what is open is the order C in fact settled"
    );
}
