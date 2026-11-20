use super::*;

/// The type a place reaches is the one its projections lead to.
///
/// `p` and `*p` are two places and two types, which is the distinction
/// `Place`'s own doc comment says an analysis depends on. The interpreter
/// asks this to know what width an operation happens at, so answering the
/// pointer's own type would check an `int`'s arithmetic against a pointer.
///
/// Mutation: answer `function.local(place.local)` and ignore the
/// projections. This fails on the second assertion.
/// C17 6.5 p7 lets an object be reached through its own type or the signed
/// or unsigned type corresponding to it, the one of the same rank (6.2.5 p6),
/// and through no other integer type: an `int` through an `unsigned int`, not
/// through a `long`.
///
/// Mutation: answer identity alone in `same_or_counterpart`; the second
/// assertion fails. Mutation: answer every pair of integer types; the third
/// does.
#[test]
fn an_integer_is_reached_through_its_counterpart_and_no_other_rank() {
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);
    let unsigned = unit.push_type(Ty::Integer {
        rank: Rank::Int,
        sign: Sign::Unsigned,
    });
    let long = unit.push_type(Ty::Integer {
        rank: Rank::Long,
        sign: Sign::Signed,
    });
    let pointer = unit.push_type(Ty::Pointer(int));

    assert!(unit.same_or_counterpart(int, int));
    assert!(unit.same_or_counterpart(unsigned, int));
    assert!(!unit.same_or_counterpart(long, int));
    assert!(!unit.same_or_counterpart(pointer, int));
}

#[test]
fn the_type_a_place_reaches_follows_its_projections() {
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);
    let pointer = unit.push_type(Ty::Pointer(int));

    let (_sources, at) = spans();
    let mut function = Function::new(at, int, [pointer]);
    let p = function.parameters().next().expect("one parameter");

    assert_eq!(unit.place_ty(&function, &Place::local(p)), Some(pointer));
    assert_eq!(
        unit.place_ty(&function, &Place::new(p, vec![Projection::Deref])),
        Some(int)
    );

    // A projection that does not fit is not a panic: the interpreter asks
    // this before it resolves anything, and stopping with a sentence is
    // what it promises. Only a hand-built unit gets here.
    let holding = function.push_local(int);
    assert_eq!(
        unit.place_ty(&function, &Place::new(holding, vec![Projection::Deref])),
        None
    );
}

/// A pointer is a pointer, and an array of pointers is not: no whole array
/// is a value (C17 6.3.2.1 p3), and no parameter is one (6.7.6.3 p7).
///
/// Mutation: answer `true` for `Ty::Array` in `Ty::is_pointer`. The
/// assertion on `pointers` fails, and no case in the corpus does: what an
/// array would be asked as a pointer for, nothing reaches yet.
#[test]
fn a_pointer_is_a_pointer_and_an_array_of_pointers_is_not() {
    let int = TyId(0);
    assert!(Ty::Pointer(int).is_pointer());
    let pointers = Ty::Array {
        element: int,
        length: 2,
    };
    assert!(!pointers.is_pointer());
    for scalar in [Ty::INT, Ty::CHAR, Ty::Void] {
        assert!(!scalar.is_pointer());
    }
}

/// What a type is worth is the target's answer, and `void` has none.
///
/// Mutation: answer `Some` for a pointer or for `void`. The last two
/// assertions fail, and the interpreter would start checking a pointer's
/// arithmetic against a width nothing measured.
#[test]
fn what_a_type_is_worth_is_the_target_s_to_say() {
    let mut unit = TranslationUnit::new(
        Target::from_triple("aarch64-unknown-linux-gnu").expect("a known triple"),
    );
    let int = unit.push_type(Ty::INT);
    let character = unit.push_type(Ty::CHAR);
    let void = unit.push_type(Ty::Void);
    let pointer = unit.push_type(Ty::Pointer(int));

    assert_eq!(unit.integer(int).expect("an integer").bits(), 32);
    assert!(unit.integer(int).expect("an integer").signed());
    // One of the two rows of `Target::ALL` where `char` is unsigned, which
    // is what makes this a question about the machine rather than about C.
    assert!(!unit.integer(character).expect("an integer").signed());

    assert_eq!(unit.integer(void), None);
    assert_eq!(unit.integer(pointer), None);
}

/// A target, for a test that is not about the machine.
///
/// Named rather than defaulted, because `TranslationUnit::new` takes one on
/// purpose: ADR-0013 says a unit nobody said the target of is one whose
/// `int` has no width. A test that *is* about the machine names its own.
fn a_target() -> Target {
    Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple")
}
use crate::source::SourceMap;

/// A source map with one file, so that a span can be built at all.
///
/// What these tests are about is the shape, so the file is a formality:
/// nothing here reads the text back.
fn spans() -> (SourceMap, Span) {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", "int add(int a, char b) { return a + b; }\n");
    let span = Span::new(file, 31, 36);
    (sources, span)
}

/// `int add(int a, char b) { return a + b; }`, built by hand and read back.
///
/// This is the phase's third Done-when clause arriving before the first:
/// there is no frontend in this test, and #73 is what will make that
/// mechanical rather than true by accident.
///
/// Mutation: have `Function::new` push its parameters before the return
/// type. Local 0 stops being the return place and this fails, on the type
/// of local 1.
///
/// The second parameter is a `char` for that mutation's sake alone. With
/// `int add(int, int)` every local holds one type, the reordering is
/// invisible to every assertion here, and the mutation above passes: the
/// ids `return_place` and `parameters` hand back are computed from a count
/// rather than read from where the types went.
#[test]
fn a_function_is_built_and_read_back() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);

    let character = unit.push_type(Ty::CHAR);

    let mut add = Function::new(at, int, [int, character]);
    let [a, b] = add.parameters().collect::<Vec<_>>()[..] else {
        panic!("two parameters");
    };

    add.push_block(Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(add.return_place()),
            value: Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(Place::local(a)),
                rhs: Operand::Copy(Place::local(b)),
            },
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Return,
    });

    let id = unit.push_function(add);
    let add = unit.function(id);

    assert_eq!(add.name, at);
    assert_eq!(unit.functions().len(), 1);
    assert_eq!(add.return_place(), LocalId(0));
    assert_eq!(
        add.parameters().collect::<Vec<_>>(),
        [LocalId(1), LocalId(2)]
    );
    assert_eq!(add.locals().len(), 3);
    assert_eq!(add.local(add.return_place()), int);
    assert_eq!(add.local(LocalId(1)), int);
    assert_eq!(add.local(LocalId(2)), character);

    let [block] = add.blocks().collect::<Vec<_>>()[..] else {
        panic!("one block");
    };
    assert_eq!(block.terminator, Terminator::Return);
    assert_eq!(
        block.elements,
        [Element::Assign(Operation {
            place: Place::local(LocalId(0)),
            value: Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(Place::local(LocalId(1))),
                rhs: Operand::Copy(Place::local(LocalId(2))),
            },
            origin: Origin::Written(at),
        })]
    );
}

/// An edge no statement produced can be built, which is what ADR-0010
/// decided and what it is confirmed by.
///
/// Mutation: delete `Terminator::Abnormal`. This stops compiling, which is
/// the strongest form the guard can take and the reason the variant is
/// here before anything produces one.
#[test]
fn an_edge_no_statement_produced_can_be_built() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let void = unit.push_type(Ty::Void);
    let mut function = Function::new(at, void, []);

    let handler = function.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    let body = function.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Abnormal { to: handler },
    });

    assert_eq!(
        function.block(body).terminator,
        Terminator::Abnormal { to: handler }
    );
}

/// Where control can go from each way a block can end.
///
/// The expected lists are written out rather than derived, because a table
/// built the way the code builds one compares the code with itself.
///
/// **The order is asserted, not only the set**, because
/// `dataflow::Analysis::edge` names an edge by its index into this list.
/// Mutation: have the `Branch` arm push `otherwise` before `then`. This
/// fails, and so does every test that reads the order rather than the set,
/// which is a wider set than it looks and reaches the `safec-c` crate,
/// because the lowering builds an `if`'s arms in this order too.
/// `each_arm_of_a_branch_is_told_something_different` in
/// `crates/safec-ir/tests/written.rs` is the one worth following from
/// here, because it is the one that says what an analysis loses when the
/// order moves.
///
/// Mutation: add a terminator kind. `Terminator::successors` stops
/// compiling with `error[E0004]`, and so does every other walk over one,
/// which is what ADR-0010 is for. Mutation: have the `Branch` arm push
/// only `then`. This fails.
///
/// A call that does not return goes nowhere (ADR-0051). Mutation: have the
/// `Call` arm push `BlockId(0)` where `then` is `None`. This fails.
#[test]
fn every_terminator_says_where_control_can_go() {
    let (_sources, at) = spans();
    let one = BlockId(1);
    let two = BlockId(2);

    for (terminator, expected) in [
        (Terminator::Goto(one), vec![one]),
        (
            Terminator::Branch {
                condition: Operand::Constant(0),
                then: one,
                otherwise: two,
                origin: Origin::Written(at),
            },
            vec![one, two],
        ),
        (
            Terminator::Call {
                callee: FuncId(0),
                arguments: Vec::new(),
                destination: Some(Place::local(LocalId(0))),
                then: Some(two),
                origin: Origin::Written(at),
            },
            vec![two],
        ),
        (
            Terminator::Call {
                callee: FuncId(0),
                arguments: Vec::new(),
                destination: None,
                then: None,
                origin: Origin::Written(at),
            },
            Vec::new(),
        ),
        (Terminator::Return, Vec::new()),
        (Terminator::Abnormal { to: one }, vec![one]),
    ] {
        let mut successors = Vec::new();
        terminator.successors(&mut successors);

        assert_eq!(successors, expected, "{terminator:?}");
    }
}

/// `p` and `*p` are two places.
///
/// An analysis that treated them as one would say a pointer is live when
/// what it points at is not, which is the memory axis of
/// `docs/safety-model.md` answering the wrong question.
///
/// Mutation: give `Place` an equality that compares its local alone. This
/// fails.
#[test]
fn a_place_is_not_the_place_it_points_at() {
    let p = Place::local(LocalId(1));
    let pointee = Place::new(LocalId(1), vec![Projection::Deref]);

    assert_ne!(p, pointee);
    assert_eq!(pointee.local, p.local);
}

/// An id keeps naming its block after more are pushed.
///
/// ADR-0008's guard, one layer down. The vectors are private for the same
/// reason: holding a `&Block` across a push is `error[E0502]`.
///
/// Mutation: take the id from `len()` after the push rather than before.
/// Every id is off by one and this fails.
#[test]
fn an_id_still_names_its_block_after_more_are_pushed() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let void = unit.push_type(Ty::Void);
    let mut function = Function::new(at, void, []);

    let first = function.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    let second = function.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Goto(first),
    });

    assert_eq!(function.block(first).terminator, Terminator::Return);
    assert_eq!(function.block(second).terminator, Terminator::Goto(first));
}

/// One type has one id, however many times it is asked for.
///
/// That is what lets a `TyId` be compared with `==` instead of walked, and
/// what lets `Ty` derive `PartialEq` where the frontend's `ast::Type`
/// refuses to.
///
/// Mutation: have `push_type` append without looking in `interned`. The
/// two `int`s become two ids, the two `int *`s become two more, and this
/// fails.
#[test]
fn one_type_has_one_id() {
    let mut unit = TranslationUnit::new(a_target());

    let int = unit.push_type(Ty::INT);
    let also_int = unit.push_type(Ty::INT);
    let character = unit.push_type(Ty::CHAR);
    let pointer = unit.push_type(Ty::Pointer(int));
    let also_pointer = unit.push_type(Ty::Pointer(also_int));

    assert_eq!(int, also_int);
    assert_eq!(pointer, also_pointer);
    assert_ne!(int, character);
    assert_ne!(int, pointer);
    assert_eq!(unit.ty(pointer), Ty::Pointer(int));
}

/// An operation can say nobody wrote it, and still point somewhere.
///
/// `docs/roadmap.md` asks for exactly this: a destructor at the end of a
/// scope has "a location to blame and no source text". The two kinds carry
/// the same span here, because what differs is not where to point but what
/// a diagnostic may say about it.
///
/// Mutation: delete `Origin::Generated` and the arm of `span` that reads
/// it. This stops compiling. Mutation: have `Origin::Generated` mean the
/// same as `Written`, by making the two operations below compare equal.
/// The `assert_ne!` fails.
#[test]
fn an_operation_can_say_nobody_wrote_it() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);
    let mut function = Function::new(at, int, []);
    let temporary = function.push_local(int);

    let write = |origin| Operation {
        place: Place::local(temporary),
        value: Rvalue::Use(Operand::Constant(0)),
        origin,
    };
    let written = write(Origin::Written(at));
    let generated = write(Origin::Generated(at));

    assert_eq!(written.origin.span(), at);
    assert_eq!(generated.origin.span(), at);
    assert_ne!(written, generated);
}

/// Two functions with one name are two functions.
///
/// C says so already: two `static` functions in different translation
/// units share a name, and `docs/c-family.md` asks that identity in the IR
/// be an id rather than a string for that reason. The name here is one
/// span, which is the strongest version of the case: even the same text at
/// the same place does not merge them.
///
/// Mutation: have `push_function` hand back `FuncId(0)` rather than the
/// length before the push. This fails, on the ids and on the second
/// function's locals.
///
/// Mutation: have `push_local` take its id from `len() - 1`. The local it
/// hands back names the one before it and this fails.
#[test]
fn two_functions_with_one_name_have_two_ids() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);
    let character = unit.push_type(Ty::CHAR);

    let first = Function::new(at, int, []);
    let mut second = Function::new(at, int, []);
    let scratch = second.push_local(character);

    let first = unit.push_function(first);
    let second = unit.push_function(second);

    assert_ne!(first, second);
    assert_eq!(unit.function(first).name, unit.function(second).name);
    assert_eq!(unit.function(first).locals().len(), 1);
    assert_eq!(unit.function(second).locals().len(), 2);
    assert_eq!(unit.function(second).local(scratch), character);
}

/// `&a[i]`: the two shapes the lifetime analysis is written against.
///
/// `Rvalue::Address` is the only operation that turns a place into a
/// value, and `Projection::Index` is how a place reaches an element, so an
/// escape through an element goes through both at once. Nothing had built
/// either, and a variant nothing builds is one that can be deleted in
/// silence.
///
/// Mutation: delete `Rvalue::Address`, or `Projection::Index`, or
/// `UnOp::Neg`. Each stops this compiling. Mutation: have `Place::local`
/// hand back a place with a `Deref` on it. The `assert_ne!` fails.
#[test]
fn an_address_can_be_taken_of_an_element() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);
    let pointer = unit.push_type(Ty::Pointer(int));

    let mut function = Function::new(at, pointer, []);
    let array = function.push_local(int);
    let index = function.push_local(int);
    let p = function.push_local(pointer);

    let element = Place::new(
        array,
        vec![Projection::Index(Operand::Copy(Place::local(index)))],
    );
    let taken = Operation {
        place: Place::local(p),
        value: Rvalue::Address(element.clone()),
        origin: Origin::Written(at),
    };
    let negated = Operation {
        place: Place::local(index),
        value: Rvalue::Unary {
            op: UnOp::Neg,
            operand: Operand::Copy(Place::local(index)),
        },
        origin: Origin::Written(at),
    };

    assert_ne!(element, Place::local(array));
    assert_eq!(taken.value, Rvalue::Address(element));
    assert_ne!(taken.value, negated.value);
}

/// A loop, which is a graph a finished block cannot be pushed into.
///
/// The header names the body and the body names the header, so one of the
/// two ids exists before its block does. That is what `reserve_block` is
/// for, and without it the module could hold every straight-line function
/// and no `while` at all.
///
/// Mutation: delete `reserve_block` and `fill_block`. This stops compiling,
/// and no way of ordering the pushes brings it back. Mutation: have
/// `fill_block` push rather than write into the slot the id names. The back
/// edge lands on the wrong block and this fails.
#[test]
fn a_loop_is_built_by_reserving_the_block_it_jumps_back_to() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);
    let mut function = Function::new(at, int, []);
    let counter = function.push_local(int);

    let header = function.reserve_block();
    let exit = function.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    let body = function.push_block(Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(counter),
            value: Rvalue::Binary {
                op: BinOp::Sub,
                lhs: Operand::Copy(Place::local(counter)),
                rhs: Operand::Constant(1),
            },
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Goto(header),
    });
    function.fill_block(
        header,
        Block {
            elements: Vec::new(),
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(counter)),
                then: body,
                otherwise: exit,
                origin: Origin::Written(at),
            },
        },
    );

    let mut successors = Vec::new();
    function
        .block(header)
        .terminator
        .successors(&mut successors);
    assert_eq!(successors, [body, exit]);

    successors.clear();
    function.block(body).terminator.successors(&mut successors);
    assert_eq!(successors, [header]);
    assert_eq!(function.blocks().len(), 3);
}

/// A call says where it was written, and may write its result nowhere.
///
/// `free(p);` is both at once: `docs/safety-model.md` wants to say "p freed
/// here", and there is no place the result goes.
///
/// Mutation: delete `Terminator::Call`'s `origin`, or make `destination` a
/// `Place` again. Each stops this compiling, and the first takes the span
/// the memory analysis points at with it.
#[test]
fn a_call_says_where_it_is_and_may_write_nowhere() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let void = unit.push_type(Ty::Void);
    let int = unit.push_type(Ty::INT);
    let pointer = unit.push_type(Ty::Pointer(int));

    let free = unit.push_function(Function::declaration(at, void, [pointer]));
    let mut caller = Function::new(at, void, []);
    let p = caller.push_local(pointer);
    let after = caller.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    let call = caller.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Call {
            callee: free,
            arguments: vec![Operand::Copy(Place::local(p))],
            destination: None,
            then: Some(after),
            origin: Origin::Written(at),
        },
    });

    let Terminator::Call {
        destination,
        origin,
        callee,
        ..
    } = &caller.block(call).terminator
    else {
        panic!("a call");
    };
    assert_eq!(*destination, None);
    assert_eq!(origin.span(), at);
    assert_eq!(callee.index(), 0);
}

/// A function whose body is elsewhere is not a function with no blocks.
///
/// What an analysis may assume at a call turns on which of the two it is,
/// and the unsound reading of an empty list is that the callee does
/// nothing.
///
/// Mutation: have `Function::declaration` return what `Function::new`
/// returns. `is_defined` starts answering true and this fails.
#[test]
fn a_declaration_is_not_a_definition_with_no_blocks() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let void = unit.push_type(Ty::Void);
    let int = unit.push_type(Ty::INT);

    let declared = Function::declaration(at, void, [int]);
    let defined = Function::new(at, void, [int]);

    assert!(!declared.is_defined());
    assert!(defined.is_defined());
    assert_eq!(declared.locals().len(), defined.locals().len());
    assert_eq!(defined.blocks().len(), 0);
}

/// A place is what a dataflow lattice is keyed on.
///
/// `p` and `*p` carry separate states, so a table indexed by [`LocalId`] is
/// the wrong table and the key has to be the place itself.
///
/// Mutation: take `Eq` or `Hash` off `Place`. This stops compiling.
#[test]
fn a_place_is_a_key() {
    let mut states = HashMap::new();
    let p = Place::local(LocalId(1));
    let pointee = Place::new(LocalId(1), vec![Projection::Deref]);

    states.insert(p.clone(), "live");
    states.insert(pointee.clone(), "freed");

    assert_eq!(states.get(&p), Some(&"live"));
    assert_eq!(states.get(&pointee), Some(&"freed"));
}

/// A declaration becomes the definition it was standing in for.
///
/// Mutation: have `fill_function` write to the first function rather than
/// to the id it was given. The second keeps its declaration and this fails.
#[test]
fn a_declared_function_can_be_given_its_body() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);

    let first = unit.push_function(Function::declaration(at, int, []));
    let second = unit.push_function(Function::declaration(at, int, []));
    unit.fill_function(second, Function::new(at, int, []));

    assert!(!unit.function(first).is_defined());
    assert!(unit.function(second).is_defined());
}

/// A function is given a body once.
///
/// A second one would leave every call that was checked against the first
/// definition pointing at another, which is the defect `fill_block` refuses
/// for a block.
///
/// Mutation: drop the assertion in `fill_function`. Nothing panics and this
/// fails.
#[test]
#[should_panic(expected = "already defined")]
fn a_function_is_not_given_a_body_twice() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);

    let id = unit.push_function(Function::declaration(at, int, []));
    unit.fill_function(id, Function::new(at, int, []));
    unit.fill_function(id, Function::new(at, int, []));
}

/// A branch into an empty block that branches again on the same local is
/// threaded; into a block that is not empty, or that branches on another
/// local, it is left alone.
///
/// Mutation: drop the test that the second block has no elements in
/// `Function::thread_rebranches`, or the test that it branches on the same
/// local. The first branch is sent past a block it should not be, and this
/// fails.
#[test]
fn a_branch_is_threaded_only_through_an_empty_rebranch_on_its_own_local() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);

    for (empty, same) in [(true, true), (false, true), (true, false)] {
        let mut function = Function::new(at, int, []);
        let tested = function.push_local(int);
        let other = function.push_local(int);
        let ends = |function: &mut Function| {
            function.push_block(Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            })
        };
        let (taken, skipped) = (ends(&mut function), ends(&mut function));
        let elements = if empty {
            Vec::new()
        } else {
            vec![Element::Sequenced {
                origin: Origin::Generated(at),
            }]
        };
        let rebranch = function.push_block(Block {
            elements,
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(if same { tested } else { other })),
                then: taken,
                otherwise: skipped,
                origin: Origin::Written(at),
            },
        });
        let first = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(tested)),
                then: rebranch,
                otherwise: skipped,
                origin: Origin::Written(at),
            },
        });

        function.thread_rebranches();

        let mut successors = Vec::new();
        function.block(first).terminator.successors(&mut successors);
        let expected = if empty && same { taken } else { rebranch };
        assert_eq!(
            successors,
            [expected, skipped],
            "empty: {empty}, same: {same}"
        );
    }
}

/// A branch on a place read through a pointer is not threaded, on either
/// side, and a chain of two empty re-branches is followed to its end.
///
/// Mutation: drop either test that a branch's place has no projection in
/// `Function::thread_rebranches`, or return after its first pass. This
/// fails.
#[test]
fn a_branch_is_threaded_along_a_chain_and_never_through_a_place_read_through_a_pointer() {
    let (_sources, at) = spans();
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);
    let branch = |tested: Place, then: BlockId, otherwise: BlockId| Block {
        elements: Vec::new(),
        terminator: Terminator::Branch {
            condition: Operand::Copy(tested),
            then,
            otherwise,
            origin: Origin::Written(at),
        },
    };
    let read_through = |local: LocalId| Place::new(local, vec![Projection::Deref]);

    // A projection on the first branch, then on the second.
    for first_reads_through in [true, false] {
        let mut function = Function::new(at, int, []);
        let tested = function.push_local(int);
        let ends = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        let second_place = if first_reads_through {
            Place::local(tested)
        } else {
            read_through(tested)
        };
        let rebranch = function.push_block(branch(second_place, ends, ends));
        let first_place = if first_reads_through {
            read_through(tested)
        } else {
            Place::local(tested)
        };
        let first = function.push_block(branch(first_place, rebranch, ends));

        function.thread_rebranches();

        let mut successors = Vec::new();
        function.block(first).terminator.successors(&mut successors);
        assert_eq!(
            successors,
            [rebranch, ends],
            "first reads through: {first_reads_through}"
        );
    }

    // A chain: the first branch reaches the end only after two passes,
    // since the second re-branch is pushed after the first is looked at.
    let mut function = Function::new(at, int, []);
    let tested = function.push_local(int);
    let ends = function.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    let other = function.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    // Numbered first, so that one pass looks at it before `near` has moved.
    let first = function.reserve_block();
    let near = function.reserve_block();
    let far = function.push_block(branch(Place::local(tested), ends, other));
    function.fill_block(near, branch(Place::local(tested), far, other));
    function.fill_block(first, branch(Place::local(tested), near, other));

    function.thread_rebranches();

    let mut successors = Vec::new();
    function.block(first).terminator.successors(&mut successors);
    assert_eq!(successors, [ends, other]);
}

/// An index of an array reaches its element, and an array takes its
/// elements' bytes, C17 6.5.3.4 p7.
///
/// No C program asks the first: the lowering builds an `Index` only under
/// `Rvalue::Address`, which no reader asks the type of. Mutation: answer
/// `None` for an `Index` of an array; the first assertion fails. Mutation:
/// answer one element's size for an array; the second does.
#[test]
fn an_array_s_element_is_what_an_index_reaches_and_its_size_is_its_elements() {
    let mut unit = TranslationUnit::new(a_target());
    let int = unit.push_type(Ty::INT);
    let array = unit.push_type(Ty::Array {
        element: int,
        length: 4,
    });

    let (_sources, at) = spans();
    let mut function = Function::new(at, int, []);
    let a = function.push_local(array);

    assert_eq!(
        unit.place_ty(
            &function,
            &Place::new(a, vec![Projection::Index(Operand::Constant(0))])
        ),
        Some(int)
    );
    assert_eq!(unit.size_of(array), Some(16));
}

/// Every reader of a place answers every [`Shape`] with a `match`, so that a
/// new kind of step is `E0004` wherever a place is read (#1092). A `matches!`
/// or an `if let` over a place's shape or its steps answers only the arm it
/// names and lets a new one through in silence, which the compiler cannot see,
/// so the sources are read for one. See ADR-0113.
///
/// Mutation: write `matches!(place.shape(), Shape::Local)`, `if let
/// Shape::Local = place.shape()`, or a `_ =>` arm in a `match` on a shape, in
/// any reader in `safec-ir`, `safec-c` or
/// `safec-llvm`; this fails and names the file. **What it cannot see** is a
/// shape bound to a variable and then matched with `matches!` by that name,
/// which only review catches.
#[test]
fn every_reader_of_a_place_answers_every_shape() {
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("a source directory reads") {
            let path = entry.expect("an entry reads").path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate sits in `crates/`");
    let mut files = Vec::new();
    for name in ["safec-ir", "safec-c", "safec-llvm"] {
        sources(&crates.join(name).join("src"), &mut files);
    }
    assert!(
        files.len() > 20,
        "the walk found the sources: {}",
        files.len()
    );
    let mut evasions = Vec::new();
    // Not this file, which spells the evasions it looks for.
    let this = std::path::Path::new(file!())
        .file_name()
        .expect("a file name");
    for file in &files {
        if file.ends_with(std::path::Path::new("ir").join(this)) {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("a source file reads");
        let mut rest = text.as_str();
        while let Some(at) = rest.find("matches!(") {
            let tail = &rest[at..];
            let mut depth = 0;
            let mut end = tail.len();
            for (index, c) in tail.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = index;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let call = &tail[..end];
            // What it matches is a shape, or a pattern names a step. A guard
            // that matches a shape exhaustively inside it is fine.
            let first = call["matches!(".len()..].split(',').next().unwrap_or("");
            if first.contains(".shape()") || call.contains("Projection::") {
                evasions.push(format!("{}: {call})", file.display()));
            }
            rest = &tail[end..];
        }
        // And a `match` over a shape with a `_` arm, which answers a new
        // shape as quietly as a `matches!` does. Found by review.
        let mut rest = text.as_str();
        while let Some(at) = rest.find(".shape() {") {
            let tail = &rest[at + ".shape() ".len()..];
            let mut depth = 0;
            let mut end = tail.len();
            for (index, c) in tail.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = index;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let block = &tail[..end];
            let mut level = 0;
            for line in block.lines() {
                if level == 1 && line.trim_start().starts_with("_ =>") {
                    evasions.push(format!("{}: `_` arm on a shape", file.display()));
                }
                level += line.matches('{').count();
                level -= line.matches('}').count().min(level);
            }
            rest = &tail[end..];
        }
        // And an `if let`, `while let` or `let ... else` naming one shape or
        // step, and a comparison of the steps, outside a test module, where
        // a hand-built place is spelled out on purpose. Found by review.
        let test_module = file.file_name().is_some_and(|name| name == "tests.rs");
        for line in text.lines() {
            if line.contains("let Projection::")
                || line.contains("let Shape::")
                || line.contains("let crate::ir::Shape::")
                || line.contains(".shape() else")
                || (!test_module && line.contains(".steps() =="))
            {
                evasions.push(format!("{}: {}", file.display(), line.trim()));
            }
        }
    }
    assert!(
        evasions.is_empty(),
        "a place's shape or steps answered without a `match`:
{}",
        evasions.join(
            "
"
        )
    );
}

/// `*l` is read on the way to `**l` whatever steps follow, so the first of
/// two leading dereferences is found for a place that goes on to an `Index`
/// as well. Mutation: answer it from `Shape::Deref` alone, which is `Indexed`
/// for this place; it answers `None` and this fails.
#[test]
fn the_first_of_two_dereferences_is_read_whatever_follows() {
    let l = LocalId(1);
    let place = Place::new(
        l,
        vec![
            Projection::Deref,
            Projection::Deref,
            Projection::Index(Operand::Constant(0)),
        ],
    );
    assert_eq!(
        crate::reads::first_of_two_derefs(&place),
        Some(Place::local(l).deref())
    );
    assert_eq!(
        crate::reads::first_of_two_derefs(&Place::local(l).deref()),
        None
    );
}
