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
    Block, BlockId, Element, FuncId, Function, LocalId, Operand, Operation, Origin, Place,
    Projection, Rvalue, Terminator, TranslationUnit, Ty, TyId,
};
use safec_ir::memory::{Finding, Kind, check};
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

/// Six call sites and the two names the check reads.
struct Names {
    free: Span,
    malloc: Span,
    function: Span,
    /// In the order they appear in the file, so `at[0]` is earlier than `at[1]`
    /// by the rule the join orders spans with.
    at: [Span; 6],
}

/// A file whose bytes the check can read a callee's name out of.
///
/// The check recognises `free` and `malloc` by the text their declaration's
/// name span covers, so a test that wants one has to put the word in a file.
/// The single letters after them are call sites: `Origin` carries a span, and
/// the rule that ends the walk orders them, so two frees have to be in two
/// places for that ordering to be about anything.
fn sources() -> (SourceMap, Names) {
    let mut map = SourceMap::new();
    let file = map.add_virtual("t.c", "free malloc f a b c d e g\n");

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
        ],
    };

    (map, names)
}

struct Callees {
    free: FuncId,
    malloc: FuncId,
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

fn returns() -> Block {
    Block {
        elements: vec![],
        terminator: Terminator::Return,
    }
}

/// What the check concluded about one function.
fn findings(mut unit: TranslationUnit, sources: &SourceMap, function: Function) -> Vec<Finding> {
    unit.push_function(function);
    check(sources, &unit)
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
    function.fill_block(second, free(&callees, held, names.at[2], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::FreedTwice);
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
    function.fill_block(second, free(&callees, held, names.at[3], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

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
    function.fill_block(second, free(&callees, held, names.at[1], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

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
    function.fill_block(free_two, free(&callees, second_held, names.at[3], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

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

    function.fill_block(
        entry,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: first_arm,
                otherwise: second_arm,
            },
        },
    );
    // The later span is on the arm the solver reaches first, so a rule that
    // kept whichever arrived would answer differently from one that orders
    // them. Without that the test would pass under both.
    function.fill_block(first_arm, free(&callees, held, names.at[1], joined));
    function.fill_block(second_arm, free(&callees, held, names.at[0], joined));
    function.fill_block(joined, free(&callees, held, names.at[2], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

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
    function.fill_block(free_two, free(&callees, other, names.at[3], pick));

    function.fill_block(
        pick,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: take_one,
                otherwise: take_two,
            },
        },
    );
    function.fill_block(take_one, copy(held, one, names.at[4], joined));
    function.fill_block(take_two, copy(held, other, names.at[4], joined));

    function.fill_block(joined, free(&callees, held, names.at[5], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

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
    function.fill_block(
        header,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: first_arm,
                otherwise: second_arm,
            },
        },
    );
    function.fill_block(first_arm, free(&callees, held, names.at[0], joined));
    function.fill_block(second_arm, free(&callees, held, names.at[1], joined));
    function.fill_block(
        joined,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: header,
                otherwise: exit,
            },
        },
    );
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

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
    function.fill_block(release, free(&callees, held, names.at[2], dangling));
    function.fill_block(dangling, read(value, held, names.at[3], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UsedAfterFree);
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
    function.fill_block(dangling, write(held, value, names.at[2], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, Kind::UsedAfterFree);
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

    let found = findings(unit, &sources, function);

    assert!(found.is_empty(), "{found:?}");
}

/// Taking the address of a dereference is not a dereference.
///
/// C17 6.5.3.2 p3: where the operand of `&` is the result of a unary `*`,
/// neither operator is evaluated and the result is as if both were omitted. So
/// `&*p` reads nothing through `p`, and reporting it would be a use of a freed
/// value in a program that never touched one.
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
    function.fill_block(address, address_of_deref(taken, held, names.at[2], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

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
    let branch = function.reserve_block();
    let release = function.reserve_block();
    let joined = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(allocate, malloc(&callees, held, names.at[0], branch));
    function.fill_block(
        branch,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: release,
                otherwise: joined,
            },
        },
    );
    function.fill_block(release, free(&callees, held, names.at[1], joined));
    function.fill_block(joined, read(value, held, names.at[2], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

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
/// Mutation: in `verdict`, keep the first `made` span rather than collapsing
/// two that disagree. `made` becomes the first arm's `malloc` and this fails on
/// that field.
#[test]
fn a_use_after_two_allocations_names_no_allocation() {
    let (sources, names) = sources();
    let (unit, mut function, int, callees) = a_unit(&names, 0);
    let held = function.push_local(int);
    let value = function.push_local(int);

    let branch = function.reserve_block();
    let first_arm = function.reserve_block();
    let first_free = function.reserve_block();
    let second_arm = function.reserve_block();
    let second_free = function.reserve_block();
    let joined = function.reserve_block();
    let exit = function.reserve_block();

    function.fill_block(
        branch,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: first_arm,
                otherwise: second_arm,
            },
        },
    );
    function.fill_block(first_arm, malloc(&callees, held, names.at[0], first_free));
    function.fill_block(first_free, free(&callees, held, names.at[1], joined));
    function.fill_block(second_arm, malloc(&callees, held, names.at[2], second_free));
    function.fill_block(second_free, free(&callees, held, names.at[3], joined));
    function.fill_block(joined, read(value, held, names.at[4], exit));
    function.fill_block(exit, returns());

    let found = findings(unit, &sources, function);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].conclusion, Conclusion::Unsafe);
    assert_eq!(found[0].freed, Some(names.at[1]), "the earlier of the two");
    assert_eq!(found[0].made, None, "the two arms do not agree");
}
