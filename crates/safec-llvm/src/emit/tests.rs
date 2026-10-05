use super::*;

use safec_ir::ir::{Block, Element, Operation, Origin};

// Every unit here is built by hand, and that is the point rather than a
// convenience: this crate has no frontend to compile C with, so a test
// that emits is a test that the IR can be read on its own. ADR-0011 is the
// boundary these are on the far side of. The programs that go through the
// lexer, the parser and the lowering first are in `safec`'s corpus, which
// is the only side that can compile one.

/// A target by name, because `Target` has no other way to be one.
fn target(triple: &str) -> Target {
    Target::from_triple(triple).expect("a known triple")
}

/// A source map holding one name, and the span that covers it.
fn named(name: &str) -> (SourceMap, Span) {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", name);
    let end = u32::try_from(name.len()).expect("a short name");
    (sources, Span::new(file, 0, end))
}

/// The whole module, and whatever could not be written.
fn module(sources: &SourceMap, unit: &TranslationUnit) -> (String, Vec<Refusal>) {
    let mut out = header(unit.target());
    let refusals = functions(sources, unit, &mut out);
    (out, refusals)
}

/// The whole module, byte for byte, from a unit nothing compiled.
///
/// One assertion over the whole text rather than a `contains` per line,
/// because the artifact is an interface: a reader redirects it into
/// `clang` and diffs two runs of it, so what it looks like is the claim.
///
/// Mutation: drop the `store` that puts a parameter into its local. The
/// text stops matching, and `clang` stops accepting the module, which is
/// two tests for two claims.
#[test]
fn a_unit_built_by_hand_becomes_a_module() {
    let (sources, at) = named("twice");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
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
    unit.push_function(twice);

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(refusals, Vec::new());
    assert_eq!(
        out,
        concat!(
            "target triple = \"x86_64-pc-windows-msvc\"\n",
            "\n",
            "define i32 @twice(i32 %arg0) {\n",
            "entry:\n",
            "  %_0 = alloca i32\n",
            "  %_1 = alloca i32\n",
            "  store i32 %arg0, ptr %_1\n",
            "  br label %bb0\n",
            "\n",
            "bb0:\n",
            "  %t0 = load i32, ptr %_1\n",
            "  %t1 = load i32, ptr %_1\n",
            "  %t2 = add nsw i32 %t0, %t1\n",
            "  store i32 %t2, ptr %_0\n",
            "  %t3 = load i32, ptr %_0\n",
            "  ret i32 %t3\n",
            "}\n",
        )
    );
}

/// Each type is spelled the way the target says, and a `void` local gets no
/// storage: `alloca void` is not a thing, and nothing reads one.
///
/// Mutation: spell `Ty::Char` as the target's `int`. The `i8` lines become
/// `i32` and this fails. Mutation: give every local an `alloca`. The `void`
/// return place gets one and this fails.
#[test]
fn each_type_is_spelled_as_the_target_says() {
    let (sources, at) = named("f");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    let char = unit.push_type(Ty::Char);
    let pointer = unit.push_type(Ty::Pointer(int));
    let void = unit.push_type(Ty::Void);
    let mut f = Function::new(at, void, [int, char, pointer]);
    f.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    unit.push_function(f);

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(refusals, Vec::new());
    assert!(
        out.contains("define void @f(i32 %arg0, i8 %arg1, ptr %arg2) {\n"),
        "{out}"
    );
    assert!(out.contains("  %_1 = alloca i32\n"), "{out}");
    assert!(out.contains("  %_2 = alloca i8\n"), "{out}");
    assert!(out.contains("  %_3 = alloca ptr\n"), "{out}");
    assert!(!out.contains("alloca void"), "{out}");
    assert!(out.contains("  ret void\n"), "{out}");
}

/// Widening a narrower type carries the sign only where that type has one,
/// which is the target's to say. `char` is the only type here that differs
/// between the measured machines, and it is reachable from ordinary C:
/// `char c; int i; i = c;`.
///
/// Mutation: always `sext`. The unsigned half of this fails. Mutation:
/// always `zext`. The signed half does.
#[test]
fn a_narrower_type_widens_the_way_its_own_target_says() {
    for (triple, instruction) in [
        ("x86_64-pc-windows-msvc", "sext"),
        ("aarch64-unknown-linux-gnu", "zext"),
    ] {
        let (sources, at) = named("f");
        let mut unit = TranslationUnit::new(target(triple));
        let int = unit.push_type(Ty::Int);
        let char = unit.push_type(Ty::Char);
        let mut f = Function::new(at, int, [char]);
        let c = f.parameters().next().expect("one parameter");
        f.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(f.return_place()),
                value: Rvalue::Use(Operand::Copy(Place::local(c))),
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        unit.push_function(f);

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(refusals, Vec::new());
        assert!(
            out.contains(&format!("= {instruction} i8 ")),
            "{triple}: {out}"
        );
    }
}

/// A division, a shift right and an addition mean different things on a
/// signed type and on an unsigned one, and which a `char` is follows the
/// target. C17 6.2.5 p9 is why the addition is in this list: an unsigned
/// computation cannot overflow, so `nsw` there would make a defined
/// program poison.
///
/// Nothing the frontend builds reaches this: C17 6.3.1.1 p2 promotes a
/// `char` operand, so an operation never happens at `char` in a unit this
/// compiler lowered. A hand-built one can, which is the case a Clang
/// adapter is, and is why this crate is the side of ADR-0011's arrow that
/// can be tested without a frontend.
///
/// Mutation: use `sdiv`, `ashr` and `add nsw` whatever the type. The
/// unsigned half fails. Mutation: use `udiv`, `lshr` and a plain `add`
/// whatever the type. The signed half does.
#[test]
fn an_operation_on_an_unsigned_type_is_unsigned() {
    for (triple, division, shift, addition) in [
        ("x86_64-pc-windows-msvc", "sdiv", "ashr", "add nsw"),
        ("aarch64-unknown-linux-gnu", "udiv", "lshr", "add"),
    ] {
        let (sources, at) = named("f");
        let mut unit = TranslationUnit::new(target(triple));
        let char = unit.push_type(Ty::Char);
        let mut f = Function::new(at, char, [char, char]);
        let mut parameters = f.parameters();
        let a = parameters.next().expect("two parameters");
        let b = parameters.next().expect("two parameters");
        let operation = |op| {
            Element::Assign(Operation {
                place: Place::local(f.return_place()),
                value: Rvalue::Binary {
                    op,
                    lhs: Operand::Copy(Place::local(a)),
                    rhs: Operand::Copy(Place::local(b)),
                },
                origin: Origin::Written(at),
            })
        };
        f.push_block(Block {
            elements: vec![
                operation(BinOp::Div),
                operation(BinOp::Shr),
                operation(BinOp::Add),
            ],
            terminator: Terminator::Return,
        });
        unit.push_function(f);

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(refusals, Vec::new());
        assert!(
            out.contains(&format!("= {division} i8 ")),
            "{triple}: {out}"
        );
        assert!(out.contains(&format!("= {shift} i8 ")), "{triple}: {out}");
        assert!(
            out.contains(&format!("= {addition} i8 ")),
            "{triple}: {out}"
        );
    }
}

/// A name LLVM would not take unquoted is quoted, and every byte that is
/// not plainly printable inside the quotes becomes `\xx`.
///
/// No C identifier needs this, which is the point: the IR is meant to be
/// reachable from a Clang adapter, and a C++ name is not a C identifier.
///
/// Mutation: write every name bare. LLVM stops parsing the module and this
/// fails on the text. Mutation: escape with `print::shown`. The escape
/// arrives as `\u{1b}`, which LLVM reads as `\u` and four more characters.
#[test]
fn a_name_llvm_would_not_take_is_quoted() {
    let (sources, at) = named("a\u{1b}b");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    unit.push_function(Function::declaration(at, int, []));

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(refusals, Vec::new());
    assert!(out.contains("declare i32 @\"a\\1Bb\"()\n"), "{out}");
}

/// A declaration is a `declare` and has no body, because the definition is
/// not in this unit and saying otherwise would claim a call does something
/// this compiler has not seen.
///
/// Mutation: emit a `define` with an empty body for a declaration. The
/// text stops matching and LLVM refuses a function with no blocks.
#[test]
fn a_declaration_has_no_body() {
    let (sources, at) = named("other");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    unit.push_function(Function::declaration(at, int, [int]));

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(refusals, Vec::new());
    assert!(out.ends_with("declare i32 @other(i32)\n"), "{out}");
    assert!(!out.contains("define"), "{out}");
}

/// An edge no statement produced is refused, and the function it was in
/// becomes a declaration rather than half a definition.
///
/// ADR-0010 put the variant in the IR so that every walk has to answer for
/// it before anything builds one. This is this backend's answer.
///
/// Mutation: write a `br` to the abnormal edge's target. Nothing is
/// refused and this fails on the refusal. Mutation: keep the body that was
/// written before the refusal. The module holds a definition and this
/// fails on `declare`.
#[test]
fn an_edge_no_statement_produced_is_refused() {
    let (sources, at) = named("f");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    let mut f = Function::new(at, int, []);
    let block = f.reserve_block();
    f.fill_block(
        block,
        Block {
            elements: Vec::new(),
            terminator: Terminator::Abnormal { to: block },
        },
    );
    unit.push_function(f);

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(
        refusals,
        vec![Refusal {
            why: "an edge no statement produced, which has no LLVM spelling".to_owned(),
            at: None,
        }]
    );
    assert!(out.ends_with("declare i32 @f()\n"), "{out}");
}

/// Arithmetic on a pointer is refused, in the interpreter's own words,
/// and the refusal says where.
///
/// `p + 1` counts elements and `ir::Ty` holds no width to count them with,
/// which ADR-0013 decided. Two consumers of one IR that disagreed about
/// which programs it can express would make the IR mean two things.
///
/// Mutation: emit a `getelementptr`. Nothing is refused and this fails.
#[test]
fn arithmetic_on_a_pointer_is_refused() {
    let (sources, at) = named("f");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut f = Function::new(at, pointer, [pointer]);
    let p = f.parameters().next().expect("one parameter");
    f.push_block(Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(f.return_place()),
            value: Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(Place::local(p)),
                rhs: Operand::Constant(1),
            },
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Return,
    });
    unit.push_function(f);

    let (_, refusals) = module(&sources, &unit);

    assert_eq!(
        refusals,
        vec![Refusal {
            why: "arithmetic on a pointer, which counts elements and so needs a width".to_owned(),
            at: Some(at),
        }]
    );
}

/// Whether two pointers name one object is defined and needs no width, so
/// it is written rather than refused. The interpreter answers the same
/// question the same way.
///
/// Mutation: refuse every operator on a pointer. This fails on the
/// refusal. Mutation: store the `i1` rather than widening it. The module
/// stops matching, and LLVM refuses a `store i1` into an `i32`.
#[test]
fn two_pointers_can_be_compared() {
    let (sources, at) = named("f");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut f = Function::new(at, int, [pointer, pointer]);
    let mut parameters = f.parameters();
    let p = parameters.next().expect("two parameters");
    let q = parameters.next().expect("two parameters");
    f.push_block(Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(f.return_place()),
            value: Rvalue::Binary {
                op: BinOp::Eq,
                lhs: Operand::Copy(Place::local(p)),
                rhs: Operand::Copy(Place::local(q)),
            },
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Return,
    });
    unit.push_function(f);

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(refusals, Vec::new());
    assert!(out.contains("= icmp eq ptr %t0, %t1\n"), "{out}");
    assert!(out.contains("= zext i1 %t2 to i32\n"), "{out}");
}

/// A call whose result nothing wanted writes nowhere, and a call to a
/// function that returns nothing has no result to write.
///
/// Both shapes reach the same place: the IR allows `destination: None`,
/// and the lowering instead builds a local of the callee's return type,
/// which is `void` when the callee returns nothing.
///
/// Mutation: give the `call void` a result name. LLVM refuses a value of
/// type `void`, and this fails on the text first.
#[test]
fn a_call_that_returns_nothing_stores_nothing() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", "v f");
    let (v, at) = (Span::new(file, 0, 1), Span::new(file, 2, 3));

    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    let void = unit.push_type(Ty::Void);
    let callee = unit.push_function(Function::declaration(v, void, []));

    let mut f = Function::new(at, int, []);
    // Reserved before the block it goes to, because a block's id is where
    // it sits and `Function::entry` is the first id handed out. Pushing the
    // return block first would make *it* the entry and the call would never
    // be reached.
    let first = f.reserve_block();
    let after = f.push_block(Block {
        elements: Vec::new(),
        terminator: Terminator::Return,
    });
    f.fill_block(
        first,
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
    assert_eq!(first, f.entry());
    unit.push_function(f);

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(refusals, Vec::new());
    assert!(out.contains("  call void @v()\n"), "{out}");
    assert!(!out.contains("= call"), "{out}");
}

/// A parameter that holds nothing cannot be written at all, so the function
/// it belongs to leaves neither a definition nor a declaration.
///
/// `void` is a result type and nothing else: LLVM answers `void type only
/// allowed for function results` to `declare i32 @g(void)` just as it does
/// to a definition. This is the one refusal that leaves nothing behind,
/// and it is why a caller of such a function is refused in turn.
///
/// Mutation: spell the parameter and carry on. The module holds
/// `declare i32 @g(void)`, `clang` refuses to parse it, and this fails on
/// both the refusal and the text. Mutation: leave a `declare` behind after
/// refusing. This fails on the text.
#[test]
fn a_parameter_that_holds_nothing_leaves_no_function() {
    let (sources, at) = named("g");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    let void = unit.push_type(Ty::Void);
    unit.push_function(Function::declaration(at, int, [void]));

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(
        refusals,
        vec![Refusal {
            why: "@g, which has a parameter that holds nothing".to_owned(),
            at: Some(at),
        }]
    );
    assert!(!out.contains("void"), "{out}");
    assert!(!out.contains("@g"), "{out}");
}

/// Taking the address of an indexed place is refused, and so is taking the
/// address of a dereference of something that is not a pointer.
///
/// Both reach `address` without going through `place_ty` first, which is
/// what makes them the two shapes that arm answers for. Nothing the
/// frontend builds is either: `p[i]` lowers as `*(p + i)`, which
/// `docs/frontend.md` records, and the type checker is what stops the
/// other one.
///
/// Mutation: emit a `getelementptr` for an index. The first half stops
/// being refused. Mutation: treat a `Deref` of a non-pointer as a `load`
/// of `ptr`. The second half does.
#[test]
fn a_place_this_cannot_reach_is_refused() {
    for (projection, why) in [
        (
            Projection::Index(Operand::Constant(0)),
            "an indexed place, which counts elements and so needs a width",
        ),
        (
            Projection::Deref,
            "a dereference of something that is not a pointer",
        ),
    ] {
        let (sources, at) = named("f");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        let pointer = unit.push_type(Ty::Pointer(int));
        let mut f = Function::new(at, pointer, [int]);
        let n = f.parameters().next().expect("one parameter");
        f.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(f.return_place()),
                value: Rvalue::Address(Place {
                    local: n,
                    projection: vec![projection],
                }),
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        unit.push_function(f);

        let (_, refusals) = module(&sources, &unit);

        assert_eq!(
            refusals,
            vec![Refusal {
                why: why.to_owned(),
                at: Some(at),
            }]
        );
    }
}

/// A conversion with no instruction behind it is refused rather than
/// written as something else.
///
/// An integer becoming a pointer is the shape: C gives it no meaning
/// without a cast, and LLVM's `inttoptr` would be this backend deciding
/// one. Nothing the frontend builds reaches it, because the type checker
/// is in the way.
///
/// Mutation: answer the value unchanged when neither side is an integer.
/// The module holds `store ptr %t0` where `%t0` is an `i32`, nothing is
/// refused, and this fails.
#[test]
fn a_conversion_with_no_instruction_is_refused() {
    let (sources, at) = named("f");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    let pointer = unit.push_type(Ty::Pointer(int));
    let mut f = Function::new(at, pointer, [int]);
    let n = f.parameters().next().expect("one parameter");
    f.push_block(Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(f.return_place()),
            value: Rvalue::Use(Operand::Copy(Place::local(n))),
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Return,
    });
    unit.push_function(f);

    let (_, refusals) = module(&sources, &unit);

    assert_eq!(
        refusals,
        vec![Refusal {
            why: "a conversion from `i32` to `ptr`".to_owned(),
            at: Some(at),
        }]
    );
}

/// A name is quoted for every reason LLVM has, not only for a byte that
/// needs escaping.
///
/// `@0` is an unnamed value rather than a name, so a name beginning with a
/// digit needs the quoted form even though every character in it is one
/// LLVM takes. `-`, `$` and `.` do not: they are in the unquoted set and a
/// name of them is written bare.
///
/// Mutation: drop the leading-digit test from `plain`. The first row
/// fails. Mutation: take `$`, `.` and `-` out of `is_name`. The last three
/// do.
#[test]
fn a_name_is_quoted_for_every_reason_llvm_has() {
    for (name, quoted) in [
        ("0f", true),
        ("f0", false),
        ("a$b", false),
        ("a.b", false),
        ("a-b", false),
        ("a b", true),
    ] {
        assert_eq!(plain(name), !quoted, "{name}");
    }
    assert_eq!(escaped("a b"), "a b");
    assert_eq!(escaped("a\"b"), "a\\22b");
}

/// The header names the target and is written once, which is what makes an
/// artifact with several inputs in it one module rather than a parse error
/// at the second `target triple`.
///
/// Mutation: have `functions` write the header too. An artifact built from
/// two inputs holds two of them, and LLVM stops parsing at the second.
#[test]
fn the_header_names_the_target_once() {
    let (sources, at) = named("f");
    let mut unit = TranslationUnit::new(target("wasm32-unknown-unknown"));
    let int = unit.push_type(Ty::Int);
    unit.push_function(Function::declaration(at, int, []));

    let mut out = header(unit.target());
    functions(&sources, &unit, &mut out);
    functions(&sources, &unit, &mut out);

    assert_eq!(out.matches("target triple").count(), 1, "{out}");
    assert!(
        out.starts_with("target triple = \"wasm32-unknown-unknown\"\n"),
        "{out}"
    );
}

/// A branch on a constant is written as a test of that constant at the
/// target's `int`, which is what C gives an integer constant.
///
/// **No C program reaches this any more**: the lowering decides a statement's
/// constant controlling expression and ends the block with a `Goto` (#338),
/// and every other branch is on a temporary. IR built by hand, or by another
/// frontend, still branches on a constant, so this is built by hand.
///
/// Mutation: have `condition` answer `0` for a constant. The test reads
/// `icmp ne i32 0, 0` and this fails.
#[test]
fn a_branch_on_a_constant_tests_the_constant() {
    let (sources, at) = named("pick");
    let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
    let int = unit.push_type(Ty::Int);
    let mut pick = Function::new(at, int, []);
    let entry = pick.reserve_block();
    let taken = pick.reserve_block();
    let skipped = pick.reserve_block();
    let result = pick.return_place();
    let returns = |value| Block {
        elements: vec![Element::Assign(Operation {
            place: Place::local(result),
            value: Rvalue::Use(Operand::Constant(value)),
            origin: Origin::Written(at),
        })],
        terminator: Terminator::Return,
    };
    pick.fill_block(
        entry,
        Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then: taken,
                otherwise: skipped,
                origin: Origin::Written(at),
            },
        },
    );
    pick.fill_block(taken, returns(1));
    pick.fill_block(skipped, returns(0));
    unit.push_function(pick);

    let (out, refusals) = module(&sources, &unit);

    assert_eq!(refusals, Vec::new());
    assert_eq!(
        out,
        concat!(
            "target triple = \"x86_64-pc-windows-msvc\"
",
            "
",
            "define i32 @pick() {
",
            "entry:
",
            "  %_0 = alloca i32
",
            "  br label %bb0
",
            "
",
            "bb0:
",
            "  %t0 = icmp ne i32 1, 0
",
            "  br i1 %t0, label %bb1, label %bb2
",
            "
",
            "bb1:
",
            "  store i32 1, ptr %_0
",
            "  %t1 = load i32, ptr %_0
",
            "  ret i32 %t1
",
            "
",
            "bb2:
",
            "  store i32 0, ptr %_0
",
            "  %t2 = load i32, ptr %_0
",
            "  ret i32 %t2
",
            "}
",
        )
    );
}
