use super::*;
use safec_ir::source::SourceMap;

/// What C17 calls compatible types, and what it does not.
///
/// The pairs are written out rather than derived, because a table built
/// the way the code builds one compares the code with itself. Each row is
/// a declaration a reader can write down, and the two sides are separate
/// nodes in the arena, which is the whole point: a
/// derived `PartialEq` would call the two halves of every `true` row
/// different.
///
/// Mutation: have the `Pointer` arm answer `true` without comparing its
/// pointee. `int *` and `char *` become compatible and this fails.
/// Mutation: compare `Parameters` by length alone. `int (int)` and
/// `int (char)` become compatible and this fails. Mutation: have
/// `is_promoted_by_default` answer `false` for a `char`. `int (char)` and
/// `int ()` become compatible and this fails.
#[test]
fn two_types_are_compatible_when_c_says_they_are() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", "23");
    let mut ast = Ast::new();
    let two = ast.push_expr(Expr::Number {
        span: Span::new(file, 0, 1),
    });
    let three = ast.push_expr(Expr::Number {
        span: Span::new(file, 1, 2),
    });

    let int = ast.push_type(Type::Int);
    let also_int = ast.push_type(Type::Int);
    let character = ast.push_type(Type::Char);
    let pointer_to_int = ast.push_type(Type::Pointer(int));
    let also_pointer_to_int = ast.push_type(Type::Pointer(also_int));
    let pointer_to_char = ast.push_type(Type::Pointer(character));
    let pointer_to_pointer = ast.push_type(Type::Pointer(pointer_to_int));
    let two_ints = ast.push_type(Type::Array {
        element: int,
        length: Some(two),
        written: Some(Span::new(file, 0, 1)),
    });
    let three_ints = ast.push_type(Type::Array {
        element: also_int,
        length: Some(three),
        written: Some(Span::new(file, 1, 2)),
    });
    let two_chars = ast.push_type(Type::Array {
        element: character,
        length: Some(two),
        written: Some(Span::new(file, 0, 1)),
    });

    let unnamed = |ty| Declaration {
        name: None,
        ty,
        written: ty,
        span: Span::new(file, 0, 1),
        nullability: None,
    };
    let of = |ast: &mut Ast, returns, parameters| {
        ast.push_type(Type::Function {
            returns,
            parameters,
        })
    };
    let int_of_int = of(&mut ast, int, Parameters::Prototype(vec![unnamed(int)]));
    let int_of_also_int = of(
        &mut ast,
        also_int,
        Parameters::Prototype(vec![unnamed(also_int)]),
    );
    let int_of_char = of(
        &mut ast,
        int,
        Parameters::Prototype(vec![unnamed(character)]),
    );
    let char_of_int = of(
        &mut ast,
        character,
        Parameters::Prototype(vec![unnamed(int)]),
    );
    let int_of_two_ints = of(
        &mut ast,
        int,
        Parameters::Prototype(vec![unnamed(int), unnamed(int)]),
    );
    let int_of_void = of(&mut ast, int, Parameters::Prototype(Vec::new()));
    let int_of_nothing_said = of(&mut ast, int, Parameters::Unspecified);
    let also_int_of_nothing_said = of(&mut ast, also_int, Parameters::Unspecified);

    for (left, right, same, what) in [
        (int, also_int, true, "int against int"),
        (int, character, false, "int against char"),
        (
            pointer_to_int,
            also_pointer_to_int,
            true,
            "int * against int *",
        ),
        (
            pointer_to_int,
            pointer_to_char,
            false,
            "int * against char *",
        ),
        (
            pointer_to_int,
            pointer_to_pointer,
            false,
            "int * against int **",
        ),
        (pointer_to_int, int, false, "int * against int"),
        // 6.7.6.2 p6 says these are different types and this cannot say so:
        // the lengths are expressions nobody evaluates.
        (two_ints, three_ints, true, "int[2] against int[3]"),
        (two_ints, two_chars, false, "int[2] against char[2]"),
        (
            int_of_int,
            int_of_also_int,
            true,
            "int (int) against int (int)",
        ),
        (
            int_of_int,
            int_of_char,
            false,
            "int (int) against int (char)",
        ),
        (
            int_of_int,
            char_of_int,
            false,
            "int (int) against char (int)",
        ),
        (
            int_of_int,
            int_of_two_ints,
            false,
            "int (int) against int (int, int)",
        ),
        // 6.7.6.3 p15's own case: a prototype whose parameters are
        // unchanged by the default argument promotions is compatible with
        // an empty identifier list, and a `char` parameter is changed.
        (
            int_of_void,
            int_of_nothing_said,
            true,
            "int (void) against int ()",
        ),
        (
            int_of_int,
            int_of_nothing_said,
            true,
            "int (int) against int ()",
        ),
        (
            int_of_char,
            int_of_nothing_said,
            false,
            "int (char) against int ()",
        ),
        (
            int_of_nothing_said,
            also_int_of_nothing_said,
            true,
            "int () against int ()",
        ),
    ] {
        // No struct is compared here, so nothing is the same struct.
        let no_struct = |_, _| false;
        assert_eq!(ast.compatible(left, right, &no_struct), same, "{what}");
        assert_eq!(
            ast.compatible(right, left, &no_struct),
            same,
            "{what}, the other way"
        );
    }
}

/// Every shape of type, and how C declares one of it.
///
/// The expected strings are written out rather than derived from the types,
/// because a test that builds its expectation the way
/// the code does is comparing the code with itself. These were taken from
/// `clang -Xclang -ast-dump` on the same declarations, so they are what
/// another compiler prints and not what this one happens to.
///
/// Mutation: drop the parentheses from the pointer arm of `spell_type`, or
/// change where the space goes. This fails.
#[test]
fn every_type_is_spelled_the_way_c_declares_it() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", "10");
    let mut ast = Ast::new();
    let ten = ast.push_expr(Expr::Number {
        span: Span::new(file, 0, 2),
    });

    let int = ast.push_type(Type::Int);
    let void = ast.push_type(Type::Void);
    let character = ast.push_type(Type::Char);
    let pointer_to_int = ast.push_type(Type::Pointer(int));
    let pointer_to_pointer = ast.push_type(Type::Pointer(pointer_to_int));
    let pointer_to_char = ast.push_type(Type::Pointer(character));
    let pointer_to_void = ast.push_type(Type::Pointer(void));
    let array_of_int = ast.push_type(Type::Array {
        element: int,
        length: Some(ten),
        written: Some(Span::new(file, 0, 2)),
    });
    let incomplete = ast.push_type(Type::Array {
        element: int,
        length: None,
        written: None,
    });
    let array_of_pointer = ast.push_type(Type::Array {
        element: pointer_to_int,
        length: Some(ten),
        written: Some(Span::new(file, 0, 2)),
    });
    let pointer_to_array = ast.push_type(Type::Pointer(array_of_int));

    let unnamed = |ty| Declaration {
        name: None,
        ty,
        written: ty,
        span: Span::new(file, 0, 2),
        nullability: None,
    };
    let takes_int = |returns| Type::Function {
        returns,
        parameters: Parameters::Prototype(vec![unnamed(int)]),
    };
    let returns_int = ast.push_type(takes_int(int));
    let returns_pointer = ast.push_type(takes_int(pointer_to_int));
    let pointer_to_function = ast.push_type(Type::Pointer(returns_int));
    let takes_nothing = ast.push_type(Type::Function {
        returns: int,
        parameters: Parameters::Prototype(Vec::new()),
    });
    let unspecified = ast.push_type(Type::Function {
        returns: int,
        parameters: Parameters::Unspecified,
    });

    for (ty, spelling) in [
        (int, "int"),
        (character, "char"),
        (void, "void"),
        (pointer_to_int, "int *"),
        (pointer_to_pointer, "int **"),
        (pointer_to_void, "void *"),
        (pointer_to_char, "char *"),
        (array_of_int, "int[10]"),
        (incomplete, "int[]"),
        (array_of_pointer, "int *[10]"),
        (pointer_to_array, "int (*)[10]"),
        (returns_int, "int (int)"),
        (returns_pointer, "int *(int)"),
        (pointer_to_function, "int (*)(int)"),
        (takes_nothing, "int (void)"),
        (unspecified, "int ()"),
    ] {
        assert_eq!(spell_type(&sources, &ast, ty), spelling, "{ty:?}");
    }
}

/// A span in a file that exists, because `Span` cannot be built without a
/// `FileId` and a `FileId` cannot be built without a file. What these tests
/// are about is the indices, so the file is a formality.
fn span(start: u32) -> Span {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("test.c", "x".repeat(64));
    Span::new(file, start, start + 1)
}

/// A walk that hands its own worklist to `extend_children` reaches every
/// node.
///
/// This is the shortest correct walk of an expression tree, and it is
/// correct only because `extend_children` appends. Nothing in the signature
/// says which it does, and the corpus cannot tell: `dump_expr` clears its
/// buffer before every call, so it reads the same either way. This is the
/// only thing that does.
///
/// Mutation: put `out.clear()` at the top of `extend_children`. The walk
/// then loses everything it had queued each time it asks a node for its
/// children, and this fails on the count. It fails quietly in a caller,
/// which is the point: a safety analysis that visits a fifth of a function
/// and reports nothing is the failure `docs/safety-model.md` exists to
/// prevent, and it does not announce itself.
#[test]
fn the_simplest_walk_over_the_tree_reaches_every_node() {
    let s = span(0);
    let mut ast = Ast::default();

    // `a[b] + c`, built by hand rather than parsed: this is about the
    // arena, and `parser.rs` is not the thing under test.
    let a = ast.push_expr(Expr::Identifier { span: s });
    let b = ast.push_expr(Expr::Identifier { span: s });
    let subscript = ast.push_expr(Expr::Subscript {
        base: a,
        index: b,
        span: s,
    });
    let c = ast.push_expr(Expr::Identifier { span: s });
    let root = ast.push_expr(Expr::Binary {
        op: BinOp::Add,
        lhs: subscript,
        rhs: c,
        span: s,
    });

    let mut work = vec![root];
    let mut visited = 0;
    while let Some(id) = work.pop() {
        visited += 1;
        assert!(visited <= 5, "the walk did not terminate");
        ast.expr(id).extend_children(&mut work);
    }

    assert_eq!(visited, 5, "every node in the tree is reached exactly once");
}

/// Every node kind, and what it is called in the artifact.
///
/// Written out here rather than derived from the enums, because a test that
/// walks a table and checks it against itself proves nothing. That lesson
/// was learned on this crate's keyword table, where `return` spelled
/// `retrun` passed the whole suite, and the AST node kinds were the next
/// table in line.
///
/// Mutation: rename any arm in a `name` method. This fails.
#[test]
fn every_node_kind_is_named_the_way_the_artifact_spells_it() {
    let s = span(0);
    let e = ExprId(0);
    let t = StmtId(0);

    assert_eq!(Expr::Number { span: s }.name(), "Number");
    assert_eq!(Expr::Identifier { span: s }.name(), "Identifier");
    assert_eq!(
        Expr::Unary {
            op: UnOp::Not,
            operand: e,
            span: s
        }
        .name(),
        "Unary"
    );
    assert_eq!(
        Expr::Binary {
            op: BinOp::Add,
            lhs: e,
            rhs: e,
            span: s
        }
        .name(),
        "Binary"
    );
    assert_eq!(
        Expr::Assign {
            op: None,
            place: e,
            value: e,
            span: s
        }
        .name(),
        "Assign"
    );
    assert_eq!(
        Expr::Conditional {
            condition: e,
            then: e,
            otherwise: e,
            span: s
        }
        .name(),
        "Conditional"
    );
    assert_eq!(
        Expr::Call {
            callee: e,
            arguments: Vec::new(),
            span: s
        }
        .name(),
        "Call"
    );
    assert_eq!(
        Expr::Subscript {
            base: e,
            index: e,
            span: s
        }
        .name(),
        "Subscript"
    );
    assert_eq!(
        Expr::Comma {
            lhs: e,
            rhs: e,
            span: s
        }
        .name(),
        "Comma"
    );
    assert_eq!(Expr::Error { span: s }.name(), "Error");

    assert_eq!(
        Stmt::Compound {
            body: Vec::new(),
            span: s
        }
        .name(),
        "Compound"
    );
    assert_eq!(
        Stmt::Return {
            value: None,
            span: s
        }
        .name(),
        "Return"
    );
    assert_eq!(
        Stmt::Expression {
            value: None,
            span: s
        }
        .name(),
        "Expression"
    );
    assert_eq!(
        Stmt::If {
            condition: e,
            then: t,
            otherwise: None,
            span: s
        }
        .name(),
        "If"
    );
    assert_eq!(
        Stmt::While {
            condition: e,
            body: t,
            span: s
        }
        .name(),
        "While"
    );
    assert_eq!(
        Stmt::For {
            start: None,
            condition: None,
            step: None,
            body: t,
            span: s
        }
        .name(),
        "For"
    );
    assert_eq!(Stmt::Error { span: s }.name(), "Error");

    assert_eq!(
        Item::Function(Function {
            ty: TypeId(0),
            name: s,
            body: StmtId(0),
            span: s,
            attribute: None,
            return_nullability: None,
        })
        .name(),
        "Function"
    );
    assert_eq!(Item::Error { span: s }.name(), "Error");
}

/// Every operator, and how C spells it.
///
/// Written out for the reason the test above is: a table checked against
/// itself proves nothing, and this one is read straight into an artifact a
/// corpus case compares byte for byte.
///
/// Mutation: change any spelling. This fails.
#[test]
fn every_operator_is_spelled_the_way_c_spells_it() {
    for (op, spelling) in [
        (BinOp::Mul, "*"),
        (BinOp::Div, "/"),
        (BinOp::Rem, "%"),
        (BinOp::Add, "+"),
        (BinOp::Sub, "-"),
        (BinOp::Shl, "<<"),
        (BinOp::Shr, ">>"),
        (BinOp::Lt, "<"),
        (BinOp::Gt, ">"),
        (BinOp::Le, "<="),
        (BinOp::Ge, ">="),
        (BinOp::Eq, "=="),
        (BinOp::Ne, "!="),
        (BinOp::BitAnd, "&"),
        (BinOp::BitXor, "^"),
        (BinOp::BitOr, "|"),
        (BinOp::LogAnd, "&&"),
        (BinOp::LogOr, "||"),
    ] {
        assert_eq!(op.as_str(), spelling, "{op:?}");
    }

    for (op, spelling, fixity) in [
        (UnOp::Plus, "+", None),
        (UnOp::Minus, "-", None),
        (UnOp::Not, "!", None),
        (UnOp::BitNot, "~", None),
        (UnOp::Deref, "*", None),
        (UnOp::AddrOf, "&", None),
        (UnOp::PreInc, "++", Some("prefix")),
        (UnOp::PreDec, "--", Some("prefix")),
        (UnOp::PostInc, "++", Some("postfix")),
        (UnOp::PostDec, "--", Some("postfix")),
    ] {
        assert_eq!(op.as_str(), spelling, "{op:?}");
        assert_eq!(op.fixity(), fixity, "{op:?}");
    }
}

/// An id still names its node after more nodes are added.
///
/// This is the property `Box` cannot give and the whole reason the tree is
/// flat. See ADR-0008. A phase that walks the tree holds ids across the
/// pushes a later phase makes, and every one of them has to still mean what
/// it meant.
///
/// Mutation: take the id from `len()` after the push rather than before.
/// This fails.
#[test]
fn an_id_still_names_its_node_after_more_are_pushed() {
    let mut ast = Ast::new();

    let first = ast.push_expr(Expr::Number { span: span(0) });
    let second = ast.push_expr(Expr::Number { span: span(10) });
    ast.push_expr(Expr::Error { span: span(20) });

    assert_eq!(ast.expr(first), &Expr::Number { span: span(0) });
    assert_eq!(ast.expr(second), &Expr::Number { span: span(10) });
}

/// The three arenas are separate, so an index into one says nothing about
/// the others and cannot be used against them.
///
/// Mutation: give `push_stmt` the length of `exprs`. This fails.
#[test]
fn each_kind_of_node_is_numbered_on_its_own() {
    let mut ast = Ast::new();

    ast.push_expr(Expr::Number { span: span(0) });
    ast.push_expr(Expr::Number { span: span(1) });
    let stmt = ast.push_stmt(Stmt::Error { span: span(2) });

    assert_eq!(stmt, StmtId(0));
}
