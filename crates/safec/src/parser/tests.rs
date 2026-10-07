use super::*;
use crate::lexer::lex;
use safec_ir::source::SourceMap;

struct Parsed {
    ast: Ast,
    diagnostics: DiagnosticSink,
    sources: SourceMap,
}

/// Scan and parse, the way the driver does.
fn parsed(text: &str) -> Parsed {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("test.c", text);
    let mut diagnostics = DiagnosticSink::new();
    let tokens = lex(file, sources.file(file), &mut diagnostics);
    let ast = parse(file, &tokens, &mut diagnostics);

    Parsed {
        ast,
        diagnostics,
        sources,
    }
}

/// The one declarator of a source that declares exactly one name.
///
/// The tests below are about what a declarator derives, and a declaration
/// carries a list of them, so this is where the list is unwrapped rather
/// than in each of them.
fn only_declared(parsed: &Parsed) -> &Declaration {
    let [Item::Declaration { declarators, .. }] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    let [declarator] = &declarators[..] else {
        panic!("{declarators:?}");
    };

    &declarator.declaration
}

/// The smallest whole translation unit, and the shape it makes.
///
/// Mutation: have `item` push the body before the function, or `compound`
/// return the wrong id. This fails, because the ids stop naming what the
/// tree says they name.
#[test]
fn the_smallest_translation_unit_is_a_function_a_compound_and_a_return() {
    let parsed = parsed("int main(void) { return 0; }\n");

    assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

    let [Item::Function(function)] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };

    let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
        panic!("{:?}", parsed.ast.stmt(function.body));
    };
    let [only] = body[..] else {
        panic!("{body:?}");
    };

    let Stmt::Return {
        value: Some(value), ..
    } = parsed.ast.stmt(only)
    else {
        panic!("{:?}", parsed.ast.stmt(only));
    };
    assert!(matches!(parsed.ast.expr(*value), Expr::Number { .. }));
}

/// A `return` with nothing to return still parses.
///
/// Not valid in a function returning `int`, and saying so is the semantic
/// analysis's job rather than this one's: the grammar C17 6.8.6.4 gives
/// allows the expression to be absent.
#[test]
fn a_return_without_a_value_parses() {
    let parsed = parsed("int main(void) { return; }\n");

    assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

    let [Item::Function(function)] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
        panic!()
    };
    assert!(matches!(
        parsed.ast.stmt(body[0]),
        Stmt::Return { value: None, .. }
    ));
}

/// An `else` belongs to the nearest `if` it is allowed to belong to.
///
/// C17 6.8.4.1 p3: "An else is associated with the lexically nearest
/// preceding if that is allowed by the syntax." So `if (a) if (b) x; else
/// y;` runs `y;` when `a` holds and `b` does not, and runs nothing at all
/// when `a` does not.
///
/// Mutation, and it is not a typo: a recursive descent already binds
/// inward, so breaking this takes adding something. Give `if_statement` a
/// flag saying an enclosing `if` wants the `else`, pass it to the inner
/// `statement`, and have the inner `if` leave the `else` alone. This test
/// then finds the `else` on the outer one and fails.
#[test]
fn a_dangling_else_binds_to_the_nearest_if() {
    let parsed = parsed("int main(void) { if (a) if (b) x; else y; }\n");

    assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

    let [Item::Function(function)] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
        panic!()
    };

    // The outer `if` has no `else`, and the inner one has it.
    let Stmt::If {
        then,
        otherwise: None,
        ..
    } = parsed.ast.stmt(body[0])
    else {
        panic!("{:?}", parsed.ast.stmt(body[0]));
    };
    assert!(matches!(
        parsed.ast.stmt(*then),
        Stmt::If {
            otherwise: Some(_),
            ..
        }
    ));
}

/// A null statement is an expression statement with no expression.
///
/// C17 6.8.3 p1 gives one production, `expression_opt ;`, and p3 describes
/// the null statement as "consisting of just a semicolon" without giving it
/// one of its own. `Stmt::Expression`'s own comment says what `clang` does
/// with the same question and why this answers differently.
///
/// Mutation: give the null statement a variant of its own, or read `;` as
/// an expression statement whose value is an `Expr::Error`. Either way the
/// first of these two stops matching `value: None`.
#[test]
fn a_null_statement_is_an_expression_statement_with_nothing_in_it() {
    let parsed = parsed("int main(void) { ; i; }\n");

    assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

    let [Item::Function(function)] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
        panic!()
    };

    assert!(matches!(
        parsed.ast.stmt(body[0]),
        Stmt::Expression { value: None, .. }
    ));
    assert!(matches!(
        parsed.ast.stmt(body[1]),
        Stmt::Expression { value: Some(_), .. }
    ));
}

/// One syntax error per file, however many the file has.
///
/// Reporting the second would mean believing a position the parser has no
/// reason to believe in, because nothing has recovered yet. The sibling
/// issue that adds recovery is what makes a second report worth reading.
///
/// The input matters, and its first version did not. `)` begins no
/// expression, and the brace after it is then not where the compound
/// expects one, so two places reach for the right to report. A file that
/// fails at its very first token passes this whichever way `report`
/// behaves, because the loop in `run` has ended before anything else can
/// speak: it guards the loop and calls it the report.
///
/// It was `{ 0; }` until statements arrived, which is the shape to keep in
/// mind when picking a replacement: an input chosen because the parser
/// rejects it stops testing anything the day the parser accepts it.
///
/// Two mutations, and each fails this on its own. Take the `failed` check
/// out of `report` and the closing brace speaks as well as the statement.
/// Drop `!self.failed` from the loop in `run` and the parser meets the same
/// token until the budget runs out.
#[test]
fn only_the_first_syntax_error_is_reported() {
    let parsed = parsed("int main(void) { ) }\n");

    assert_eq!(
        parsed.diagnostics.diagnostics().len(),
        1,
        "{:?}",
        parsed.diagnostics.diagnostics()
    );
}

/// The parser stops rather than reading past the end.
///
/// `lex` always ends its stream with `Eof`, so an empty slice does not
/// arrive through the driver. It arrives here, and the answer is an empty
/// tree rather than a panic.
#[test]
fn no_tokens_at_all_parse_to_nothing() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("empty.c", "");
    let mut diagnostics = DiagnosticSink::new();

    let ast = parse(file, &[], &mut diagnostics);

    assert!(ast.items().is_empty());
    assert_eq!(diagnostics.diagnostics().len(), 0);
}

/// A file that is only a lexical error still leaves a tree, and the parser
/// adds one report of its own rather than none or many.
#[test]
fn a_file_the_lexer_could_not_read_still_parses_to_a_tree() {
    let parsed = parsed("@\n");

    let [Item::Error { .. }] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    assert_eq!(parsed.diagnostics.diagnostics().len(), 2);
}

/// A directive never reaches the grammar.
///
/// The lexer keeps the line whole and reports it, and `TokenKind::Directive`
/// says that is so a parser is not made to answer for a line that is not C.
/// Keeping the line whole only turns an avalanche into one error, and one
/// is still one too many: this parser stops at the first thing it cannot
/// read, so a `#include` at the top of a real file left the function under
/// it unparsed.
///
/// Mutation: drop the filter in `parse`. The tree becomes a single error
/// node and the parser adds a report of its own, and this fails on both.
#[test]
fn a_directive_is_not_something_the_grammar_reads() {
    let parsed = parsed(
        "#include <stdio.h>
int main(void) { return 0; }
",
    );

    // The lexer's own report stands. What must not be there is a second.
    assert_eq!(
        parsed.diagnostics.diagnostics().len(),
        1,
        "{:?}",
        parsed.diagnostics.diagnostics()
    );
    let [Item::Function(_)] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
}

/// Nesting is bounded, and the bound is reported rather than met.
///
/// One shape per recursion that can reach the bound, because a bracket is
/// only one of them: nested blocks; nested parentheses; a chain of a
/// right-associative operator, which re-enters the climb once per operator;
/// a chain of a prefix operator, which re-enters `unary`; a chain of `else
/// if`, a nest of `while` and a nest of `for`, each of which recurses into
/// its substatement; and the two declarator shapes, one bounded by `apply`
/// and one by `deeper`. Only three of the nine write a bracket, which is
/// why the list is longer than it looks like it should be.
///
/// The list is held to that: a recursion added without a row here is a
/// level nothing says is taken. Adding one is the whole of what `deeper`
/// asks of a new rule.
///
/// Mutation: remove the `MAX_NESTING` check from `deeper`. This fails,
/// because nothing is reported for the input past the limit. Taking the
/// `deeper` call out of `compound`, `infix_from`, `unary`, `if_statement`,
/// `while_statement` or `for_statement` fails it for one shape each.
///
/// That is not the failure the bound exists to prevent, and no test here
/// can be. Without a bound the recursion runs to the end of the native
/// stack: 4000 blocks killed this compiler outright on this host, with no
/// diagnostic and an exit code the driver never chose, and a test that
/// reproduced it would take the suite down rather than fail. So each input
/// below stops one level past the limit, and what they hold is that the
/// limit is enforced at all.
#[test]
fn nesting_is_bounded_and_the_bound_is_reported() {
    // A function's body is a level like any other, so `n` braces is `n`
    // deep. The two expression shapes are two levels deeper for the same
    // count: the body is one, and reading the statement's expression at all
    // is the second, before any of the nesting being counted here.
    let blocks = |levels: usize| {
        format!(
            "int main(void) {}{}\n",
            "{".repeat(levels),
            "}".repeat(levels)
        )
    };
    let parens = |levels: usize| {
        format!(
            "int main(void) {{ return {}0{}; }}\n",
            "(".repeat(levels),
            ")".repeat(levels)
        )
    };
    let assignments =
        |levels: usize| format!("int main(void) {{ return {}0; }}\n", "a = ".repeat(levels));

    let prefixes =
        |levels: usize| format!("int main(void) {{ return {}0; }}\n", "!".repeat(levels));

    // The three statements that hold a statement, nested through their
    // bodies. One level each, on top of the function body's, and none of
    // them writes a bracket per level: `else if` in particular is `else`
    // followed by an `if` statement rather than a construct of its own, so
    // a chain of them is a chain of nested `if`s.
    let else_ifs = |levels: usize| {
        format!(
            "int main(void) {{ {}; }}\n",
            "if (a) ; else ".repeat(levels)
        )
    };
    let whiles = |levels: usize| format!("int main(void) {{ {}; }}\n", "while (a) ".repeat(levels));
    let fors = |levels: usize| format!("int main(void) {{ {}; }}\n", "for (;;) ".repeat(levels));

    // A declarator at file scope, so nothing above it has taken a level.
    // The first of these two is bounded by `apply`, which counts the
    // derivations a declarator folds; the second by `deeper`, which counts
    // the parenthesised declarators it recurses through.
    let derivations = |levels: usize| format!("int {}p;\n", "*".repeat(levels));
    let parenthesised =
        |levels: usize| format!("int {}*p{};\n", "(".repeat(levels), ")".repeat(levels));

    for (shape, at_the_limit) in [
        (&blocks as &dyn Fn(usize) -> String, MAX_NESTING),
        (&parens, MAX_NESTING - 2),
        (&assignments, MAX_NESTING - 2),
        (&prefixes, MAX_NESTING - 2),
        (&else_ifs, MAX_NESTING - 1),
        (&whiles, MAX_NESTING - 1),
        (&fors, MAX_NESTING - 1),
        (&derivations, MAX_NESTING),
        (&parenthesised, MAX_NESTING),
    ] {
        let inside = parsed(&shape(at_the_limit));
        assert_eq!(
            inside.diagnostics.diagnostics().len(),
            0,
            "{:?}",
            inside.diagnostics.diagnostics()
        );

        let beyond = parsed(&shape(at_the_limit + 1));
        let reported = beyond.diagnostics.diagnostics();
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert_eq!(reported[0].message(), "nesting is too deep");

        // Inside the thing that is too deep, never after it. `deeper`
        // reports where it stands, which is the token that opened the
        // level; `apply` only knows the count once the whole declarator has
        // been read, so it has to be handed where that declarator began.
        // Reporting where the parser is standing by then puts the caret on
        // the `;`, which is a byte with nothing wrong with it.
        let [label] = reported[0].labels() else {
            panic!("{:?}", reported[0].labels());
        };
        let source = beyond.sources.file(label.span().file());
        assert!(
            (label.span().end() as usize) < source.contents().trim_end().len(),
            "the caret is on the last token of {:?}",
            source.contents()
        );
    }
}

/// A code is the stable handle, so each syntax code is pinned by its string
/// rather than by the constant that holds it. `docs/diagnostics.md` says
/// which range they come from and why a code outlives the wording beside
/// it. `lexer.rs` holds the same test over the lexical five.
///
/// Mutation: change `EXPECTED`, `TOO_DEEP` or `BRACED_INITIALIZER`. Each
/// fails here. Before this test existed, `TOO_DEEP` could be renumbered
/// with the whole suite still green: no corpus case nests deep enough to
/// reach it, so the six that carry `SC0201` were guarding one of the
/// parser's codes and nothing was guarding the other.
///
/// `BRACED_INITIALIZER` is the one case here that a corpus case also
/// holds, because `a_braced_initializer_is_refused` compares the whole
/// rendered diagnostic. It is listed anyway rather than left to that one,
/// so that the row somebody adds for the parser's fourth code has three
/// above it to copy rather than two.
#[test]
fn each_syntax_diagnostic_keeps_the_code_it_was_assigned() {
    let too_deep = format!(
        "int main(void) {}{}\n",
        "{".repeat(MAX_NESTING + 1),
        "}".repeat(MAX_NESTING + 1)
    );

    for (source, code) in [
        ("int x\n", "SC0201"),
        (too_deep.as_str(), "SC0202"),
        ("int x = {1};\n", "SC0203"),
    ] {
        let parsed = parsed(source);
        let reported = parsed
            .diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect::<Vec<_>>();

        assert_eq!(reported, [code], "{source:?}");
    }
}

/// A nested block is one node in the arena, not two.
///
/// `compound` pushes the node and hands back its id, and `statement` passes
/// that id on. A `statement` that returned the node instead would have its
/// caller push a second copy, so one pair of braces would have two ids,
/// both naming the same children. ADR-0008 exists so that an id names one
/// node, and two ids for one construct is that undone: a side table keyed
/// by id would have two slots for those braces and no way to say which the
/// analysis meant.
///
/// The arena is read through `Debug` because the duplicate is unreachable
/// from the root, so no walk of the tree can see it. That is the whole
/// difficulty: it is invisible in the artifact and costs a later phase.
///
/// Mutation: have `statement` return `self.ast.stmt(id).clone()` and its
/// caller push it. This fails with three.
#[test]
fn a_nested_block_is_one_node_in_the_arena() {
    let parsed = parsed(
        "int main(void) { { return 0; } }
",
    );

    let arena = format!("{:?}", parsed.ast);
    assert_eq!(arena.matches("Compound").count(), 2, "{arena}");
}

/// A node's span reaches the end of what it covers.
///
/// Nothing else in the repository can see this: the artifact prints a start
/// position only, and every other test matches on shape. What the extent is
/// for is a later diagnostic underlining a whole statement, which is the
/// point at which being wrong would be expensive and quiet.
///
/// Mutation: end each span at `start.end()` rather than at
/// `self.previous().span.end()`. This fails.
#[test]
fn a_node_reaches_the_end_of_what_it_covers() {
    let text = "int main(void) { return 0; }
";
    let parsed = parsed(text);

    let [Item::Function(function)] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    assert_eq!(&text[function.span.range()], "int main(void) { return 0; }");

    let Stmt::Compound { body, span } = parsed.ast.stmt(function.body) else {
        panic!("{:?}", parsed.ast.stmt(function.body));
    };
    assert_eq!(&text[span.range()], "{ return 0; }");
    assert_eq!(&text[parsed.ast.stmt(body[0]).span().range()], "return 0;");
}

/// A statement that holds a statement reaches the end of the one it holds.
///
/// The same claim as above, made separately because these four end
/// somewhere the one above cannot reach: not at a `;` this rule read for
/// itself, but wherever the substatement happened to stop. An `if` with an
/// `else` ends at the end of the `else`, and a `while` whose body is a
/// block ends at the block's brace.
///
/// Mutation: end any of these four spans at `start` rather than at
/// `parser.previous().span.end()`. This fails, and nothing else in the
/// repository does: the artifact prints a start position only.
#[test]
fn a_statement_that_holds_a_statement_reaches_the_end_of_the_one_it_holds() {
    let text = "int main(void) { if (a) b; else c; while (a) { b; } for (;;) b; d; }\n";
    let parsed = parsed(text);

    assert_eq!(parsed.diagnostics.diagnostics().len(), 0);

    let [Item::Function(function)] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
        panic!("{:?}", parsed.ast.stmt(function.body));
    };

    let covers = |id| &text[parsed.ast.stmt(id).span().range()];
    assert_eq!(covers(body[0]), "if (a) b; else c;");
    assert_eq!(covers(body[1]), "while (a) { b; }");
    assert_eq!(covers(body[2]), "for (;;) b;");
    assert_eq!(covers(body[3]), "d;");
}

/// So does an expression built out of other expressions.
///
/// Held separately from the statement above because nothing else can see
/// it: the artifact prints a start position only, and every other test
/// matches on shape. What the extent is for is a later diagnostic
/// underlining a whole subexpression, which is the point at which being
/// wrong is expensive and quiet.
///
/// Mutation: have `joined` end at `from` rather than at `to`, or drop the
/// closing token from any of the three spans below. This fails.
#[test]
fn a_composite_expression_reaches_the_end_of_its_last_operand() {
    let text = "int main(void) { return f(a, b) + c[d]; }\n";
    let parsed = parsed(text);

    let sum = parsed.ast.expr(returned(&parsed));
    assert_eq!(&text[sum.span().range()], "f(a, b) + c[d]");

    let Expr::Binary { lhs, rhs, .. } = sum else {
        panic!("{sum:?}");
    };
    assert_eq!(&text[parsed.ast.expr(*lhs).span().range()], "f(a, b)");
    assert_eq!(&text[parsed.ast.expr(*rhs).span().range()], "c[d]");
}

/// Every prefix operator reaches the variant that spells it.
///
/// The spellings themselves are checked in `ast.rs`; what is checked here
/// is the other half, which punctuator the parser routes to which variant.
/// Without it `!x` could be read as `~x`, or `*p` as `&p`, and both dumps
/// and both suites would agree with themselves.
///
/// Mutation: swap any two arms of the match in `unary`. This fails.
#[test]
fn every_prefix_operator_reaches_the_variant_that_spells_it() {
    for (text, expected) in [
        ("+a", UnOp::Plus),
        ("-a", UnOp::Minus),
        ("!a", UnOp::Not),
        ("~a", UnOp::BitNot),
        ("*a", UnOp::Deref),
        ("&a", UnOp::AddrOf),
        ("++a", UnOp::PreInc),
        ("--a", UnOp::PreDec),
    ] {
        let parsed = parsed(&format!("int main(void) {{ return {text}; }}\n"));
        let Expr::Unary { op, .. } = parsed.ast.expr(returned(&parsed)) else {
            panic!("{text:?} gave {:?}", parsed.ast.expr(returned(&parsed)));
        };
        assert_eq!(*op, expected, "{text:?}");
    }

    for (text, expected) in [("a++", UnOp::PostInc), ("a--", UnOp::PostDec)] {
        let parsed = parsed(&format!("int main(void) {{ return {text}; }}\n"));
        let Expr::Unary { op, .. } = parsed.ast.expr(returned(&parsed)) else {
            panic!("{text:?} gave {:?}", parsed.ast.expr(returned(&parsed)));
        };
        assert_eq!(*op, expected, "{text:?}");
    }
}

/// Parentheses change what a declarator derives, which is the whole reason
/// the tree holds a type and not a declarator.
///
/// C17 6.7.6 p6: "a declarator in parentheses is identical to the
/// unparenthesized declarator, but the binding of complicated declarators
/// may be altered by parentheses." `int *f(int)` is a function returning a
/// pointer; `int (*f)(int)` is a pointer to a function. The two differ only
/// in where the parentheses are.
///
/// Mutation: stop treating `(` as opening a parenthesised declarator in
/// `core`, or apply the suffixes before the pointers in `declarator`. Both
/// make the two derive the same thing, and this fails.
#[test]
fn parentheses_change_what_a_declarator_derives() {
    let function_returning_pointer = parsed("int *f(int);\n");
    let declaration = only_declared(&function_returning_pointer);
    let Type::Function { returns, .. } = function_returning_pointer.ast.ty(declaration.ty) else {
        panic!("{:?}", function_returning_pointer.ast.ty(declaration.ty));
    };
    assert!(matches!(
        function_returning_pointer.ast.ty(*returns),
        Type::Pointer(_)
    ));

    let pointer_to_function = parsed("int (*f)(int);\n");
    let declaration = only_declared(&pointer_to_function);
    let Type::Pointer(pointee) = pointer_to_function.ast.ty(declaration.ty) else {
        panic!("{:?}", pointer_to_function.ast.ty(declaration.ty));
    };
    assert!(matches!(
        pointer_to_function.ast.ty(*pointee),
        Type::Function { .. }
    ));
}

/// A declarator's span runs from the specifiers through its own
/// initializer, and stops before the `;`.
///
/// Both halves of what `Declaration::span` claims, and nothing else holds
/// either: `--emit ast` prints where a span starts and never where it ends,
/// so the only thing that would notice one stopping short is a diagnostic
/// that underlines a declaration, and there is not one yet.
///
/// Mutation: give each declarator the span of the specifiers alone. The
/// starts still agree, the ends collapse onto them, and no corpus case
/// moves because none of them can see an end.
#[test]
fn a_declarator_spans_the_specifiers_through_its_own_initializer() {
    let parsed = parsed("int a = 1, b;\n");
    let [Item::Declaration { declarators, .. }] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    let [first, second] = &declarators[..] else {
        panic!("{declarators:?}");
    };

    // Neither carries the `;`, and both carry the specifiers they share.
    assert_eq!(parsed.sources.snippet(first.declaration.span), "int a = 1");
    assert_eq!(
        parsed.sources.snippet(second.declaration.span),
        "int a = 1, b"
    );
}

/// A declarator that nests deeper than this parser goes is reported at
/// itself, not at the specifiers it shares with the others.
///
/// The count is only known once the whole declarator has been read, so the
/// caret has to be put back deliberately; putting it on the declaration
/// would point at an `int` that is fine, in front of however many
/// declarators are also fine.
///
/// Mutation: pass the declaration's start rather than the declarator's in
/// `init_declarator_list`. The caret moves to byte 0 and nothing else in
/// the suite notices, because no case nests a later declarator this deep.
#[test]
fn a_later_declarator_that_nests_too_deep_is_reported_where_it_was_written() {
    let source = format!("int a, {}b;\n", "*".repeat(MAX_NESTING + 1));
    let parsed = parsed(&source);

    let [reported] = parsed.diagnostics.diagnostics() else {
        panic!("{:?}", parsed.diagnostics.diagnostics());
    };
    let Some(label) = reported.primary_label() else {
        panic!("{reported:?}");
    };

    // `int a, ` is seven bytes, so this is the first `*` of the second
    // declarator rather than the `int` in front of both.
    assert_eq!(label.span().start(), 7);
}

/// The suffix written last is the first to wrap the base.
///
/// C17 6.7.6.2 p3 reads `D [ ... ]` by handing "array of T" to `D`, so
/// `int a[3][5]` is three arrays of five and not five of three. Nothing
/// else in the artifact can tell the two apart, because both spell
/// `int[3][5]`.
///
/// Mutation: drop the `reverse` in `declarator`. This fails on the length.
#[test]
fn the_last_suffix_written_wraps_the_base_first() {
    let parsed = parsed("int a[3][5];\n");

    let declaration = only_declared(&parsed);
    let Type::Array {
        element,
        length: Some(outer),
    } = parsed.ast.ty(declaration.ty)
    else {
        panic!("{:?}", parsed.ast.ty(declaration.ty));
    };
    let Type::Array {
        length: Some(inner),
        ..
    } = parsed.ast.ty(*element)
    else {
        panic!("{:?}", parsed.ast.ty(*element));
    };

    let text = |id| {
        let span = parsed.ast.expr(id).span();
        parsed.sources.file(span.file()).contents()[span.range()].to_owned()
    };
    assert_eq!(text(*outer), "3");
    assert_eq!(text(*inner), "5");
}

/// A call with no arguments is a call, not a parse error.
///
/// C17 6.5.2's `argument-expression-list` is optional, and `f()` is the
/// ordinary way to write a call to a function of no parameters.
///
/// Mutation: read an argument unconditionally in `call` rather than only
/// when the next token is not `)`. This fails.
#[test]
fn a_call_can_have_no_arguments() {
    let parsed = parsed("int main(void) { return f(); }\n");

    assert_eq!(
        parsed.diagnostics.diagnostics().len(),
        0,
        "{:?}",
        parsed.diagnostics.diagnostics()
    );

    let Expr::Call { arguments, .. } = parsed.ast.expr(returned(&parsed)) else {
        panic!("{:?}", parsed.ast.expr(returned(&parsed)));
    };
    assert!(arguments.is_empty(), "{arguments:?}");
}

/// The expression a lone `return` in a lone function reads.
fn returned(parsed: &Parsed) -> ExprId {
    let [Item::Function(function)] = parsed.ast.items() else {
        panic!("{:?}", parsed.ast.items());
    };
    let Stmt::Compound { body, .. } = parsed.ast.stmt(function.body) else {
        panic!("{:?}", parsed.ast.stmt(function.body));
    };
    let Stmt::Return {
        value: Some(value), ..
    } = parsed.ast.stmt(body[0])
    else {
        panic!("{:?}", parsed.ast.stmt(body[0]));
    };

    *value
}

/// The table C17 6.5 gives, written out rather than walked.
///
/// It is written out because a test that walks a table is comparing the
/// table with itself, and this
/// repository has already shipped one, on the C keyword list, where
/// `return` spelled `retrun` passed the entire suite.
///
/// Mutation: swap two rows, or change one operator's power, associativity
/// or node kind. This fails.
#[test]
fn the_precedence_table_is_the_one_c17_gives() {
    use Assoc::{Left, Right};
    use InfixKind::{Assign, Binary, Comma, Conditional};

    for (punct, power, assoc, kind) in [
        (Punct::Comma, 1, Left, Comma),
        (Punct::Equal, 2, Right, Assign(None)),
        (Punct::StarEqual, 2, Right, Assign(Some(BinOp::Mul))),
        (Punct::SlashEqual, 2, Right, Assign(Some(BinOp::Div))),
        (Punct::PercentEqual, 2, Right, Assign(Some(BinOp::Rem))),
        (Punct::PlusEqual, 2, Right, Assign(Some(BinOp::Add))),
        (Punct::MinusEqual, 2, Right, Assign(Some(BinOp::Sub))),
        (Punct::LessLessEqual, 2, Right, Assign(Some(BinOp::Shl))),
        (
            Punct::GreaterGreaterEqual,
            2,
            Right,
            Assign(Some(BinOp::Shr)),
        ),
        (Punct::AmpersandEqual, 2, Right, Assign(Some(BinOp::BitAnd))),
        (Punct::CaretEqual, 2, Right, Assign(Some(BinOp::BitXor))),
        (Punct::PipeEqual, 2, Right, Assign(Some(BinOp::BitOr))),
        (Punct::Question, 3, Right, Conditional),
        (Punct::PipePipe, 4, Left, Binary(BinOp::LogOr)),
        (Punct::AmpersandAmpersand, 5, Left, Binary(BinOp::LogAnd)),
        (Punct::Pipe, 6, Left, Binary(BinOp::BitOr)),
        (Punct::Caret, 7, Left, Binary(BinOp::BitXor)),
        (Punct::Ampersand, 8, Left, Binary(BinOp::BitAnd)),
        (Punct::EqualEqual, 9, Left, Binary(BinOp::Eq)),
        (Punct::BangEqual, 9, Left, Binary(BinOp::Ne)),
        (Punct::Less, 10, Left, Binary(BinOp::Lt)),
        (Punct::Greater, 10, Left, Binary(BinOp::Gt)),
        (Punct::LessEqual, 10, Left, Binary(BinOp::Le)),
        (Punct::GreaterEqual, 10, Left, Binary(BinOp::Ge)),
        (Punct::LessLess, 11, Left, Binary(BinOp::Shl)),
        (Punct::GreaterGreater, 11, Left, Binary(BinOp::Shr)),
        (Punct::Plus, 12, Left, Binary(BinOp::Add)),
        (Punct::Minus, 12, Left, Binary(BinOp::Sub)),
        (Punct::Star, 13, Left, Binary(BinOp::Mul)),
        (Punct::Slash, 13, Left, Binary(BinOp::Div)),
        (Punct::Percent, 13, Left, Binary(BinOp::Rem)),
    ] {
        assert_eq!(
            infix(punct),
            Some(Infix { power, assoc, kind }),
            "{punct:?}"
        );
    }

    // Punctuators that look like they belong and do not. `:` and the
    // increments are read where the expression they belong to is read, and
    // answering here would take them out from under it.
    for punct in [
        Punct::Colon,
        Punct::PlusPlus,
        Punct::MinusMinus,
        Punct::Dot,
        Punct::Arrow,
        Punct::Bang,
        Punct::Tilde,
        Punct::LeftParen,
        Punct::LeftBracket,
        Punct::Semicolon,
        Punct::RightBrace,
    ] {
        assert_eq!(infix(punct), None, "{punct:?}");
    }
}

/// Two operators of the same power group to the left.
///
/// `a - b - c` is `(a - b) - c` and not `a - (b - c)`, which is a different
/// number.
///
/// Mutation: in `climb`, give `Assoc::Left` the same next power as
/// `Assoc::Right`. This fails.
#[test]
fn left_associative_operators_group_to_the_left() {
    let parsed = parsed("int main(void) { return a - b - c; }\n");

    let Expr::Binary { lhs, rhs, .. } = parsed.ast.expr(returned(&parsed)) else {
        panic!("{:?}", parsed.ast.expr(returned(&parsed)));
    };

    assert!(matches!(parsed.ast.expr(*lhs), Expr::Binary { .. }));
    assert!(matches!(parsed.ast.expr(*rhs), Expr::Identifier { .. }));
}

/// Assignment groups to the right. C17 6.5.16.
///
/// `a = b = c` is `a = (b = c)`. The other grouping assigns to the result
/// of an assignment, which is not even a place.
///
/// Mutation: give assignment `Assoc::Left` in the table. This fails.
#[test]
fn assignment_groups_to_the_right() {
    let parsed = parsed("int main(void) { return a = b = c; }\n");

    let Expr::Assign { place, value, .. } = parsed.ast.expr(returned(&parsed)) else {
        panic!("{:?}", parsed.ast.expr(returned(&parsed)));
    };

    assert!(matches!(parsed.ast.expr(*place), Expr::Identifier { .. }));
    assert!(matches!(parsed.ast.expr(*value), Expr::Assign { .. }));
}

/// A comma between arguments separates them; one inside parentheses is the
/// operator.
///
/// C17 6.5.2 makes an argument an assignment-expression, which is the whole
/// of what tells the two apart. Nothing in the tree has to.
///
/// Mutation: read an argument with `expression` rather than `assignment`.
/// `f(a, b)` becomes one argument, and this fails.
#[test]
fn a_comma_in_a_call_separates_arguments() {
    let separated = parsed("int main(void) { return f(a, b); }\n");
    let Expr::Call { arguments, .. } = separated.ast.expr(returned(&separated)) else {
        panic!("{:?}", separated.ast.expr(returned(&separated)));
    };
    assert_eq!(arguments.len(), 2, "{arguments:?}");

    let grouped = parsed("int main(void) { return f((a, b)); }\n");
    let Expr::Call { arguments, .. } = grouped.ast.expr(returned(&grouped)) else {
        panic!("{:?}", grouped.ast.expr(returned(&grouped)));
    };
    assert_eq!(arguments.len(), 1, "{arguments:?}");
    assert!(matches!(grouped.ast.expr(arguments[0]), Expr::Comma { .. }));
}

/// C17 6.5.15 puts a whole expression between `?` and `:`, the comma
/// operator included, because the `:` is what ends it.
///
/// Only the third operand is read at the conditional's own power.
///
/// Mutation: read the middle with `assignment` rather than `expression`.
/// The comma then ends it, ``expected `:``` is reported, and this fails.
#[test]
fn the_middle_of_a_conditional_is_a_full_expression() {
    let parsed = parsed("int main(void) { return a ? b, c : d; }\n");

    assert_eq!(
        parsed.diagnostics.diagnostics().len(),
        0,
        "{:?}",
        parsed.diagnostics.diagnostics()
    );

    let Expr::Conditional { then, .. } = parsed.ast.expr(returned(&parsed)) else {
        panic!("{:?}", parsed.ast.expr(returned(&parsed)));
    };
    assert!(matches!(parsed.ast.expr(*then), Expr::Comma { .. }));
}

/// Each thing the parser insists on says which one was missing.
///
/// Mutation: give every `expect` the same `what`. This fails, because the
/// messages stop telling the four apart.
#[test]
fn what_was_expected_is_named_in_the_message() {
    for (text, expected) in [
        ("int (void) { }\n", "expected a name"),
        ("int (*)(void) { }\n", "expected a name"),
        ("int main void) { }\n", "expected `;`"),
        ("int main(void { }\n", "expected `)`"),
        ("int main(void) return 0; }\n", "expected `;`"),
        ("int (*p;\n", "expected `)`"),
        ("int a[10;\n", "expected `]`"),
        ("int f(int;\n", "expected `)`"),
        ("int f(int a, );\n", "expected a declaration"),
        ("int main(void) { return 0 }\n", "expected `;`"),
        ("int main(void) { return; \n", "expected `}`"),
        ("int main(void) { return + ; }\n", "expected an expression"),
        ("int main(void) { 0 }\n", "expected `;`"),
        ("int main(void) { if x) ; }\n", "expected `(`"),
        ("int main(void) { if (x ; }\n", "expected `)`"),
        ("int main(void) { while x) ; }\n", "expected `(`"),
        ("int main(void) { while (x ; }\n", "expected `)`"),
        ("int main(void) { for ;;) ; }\n", "expected `(`"),
        ("int main(void) { for (x x; ) ; }\n", "expected `;`"),
        ("int main(void) { for (;; x ; }\n", "expected `)`"),
        ("int main(void) { return (a; }\n", "expected `)`"),
        ("int main(void) { return f(a; }\n", "expected `)`"),
        ("int main(void) { return a[i; }\n", "expected `]`"),
        ("int main(void) { return a ? b c; }\n", "expected `:`"),
    ] {
        let parsed = parsed(text);
        let reported = parsed.diagnostics.diagnostics();

        assert_eq!(reported.len(), 1, "{text:?} gave {reported:?}");
        assert_eq!(reported[0].message(), expected, "{text:?}");
    }
}

/// A parameter declared as an array or a function is a pointer to C, and
/// keeps what was written for whoever prints it.
///
/// C17 6.7.6.3 p7 adjusts an array of `T` to a pointer to `T`, and p8 a
/// function to a pointer to it. So the function type is the one
/// `int g(int *, int (*)(void))` declares, by p15.
///
/// Mutation: leave a parameter's `ty` as written. The adjusted spellings
/// and the compatibility fail. Mutation: compare `written` in
/// `Ast::compatible_parameters`. The compatibility fails.
#[test]
fn a_parameter_declared_as_an_array_or_a_function_is_a_pointer() {
    let parsed = parsed("int g(int a[4], int h(void));\nint g(int *a, int (*h)(void));\n");
    let [first, second] = parsed.ast.items() else {
        panic!("two declarations: {:?}", parsed.ast.items());
    };
    let ty = |item: &Item| {
        let Item::Declaration { declarators, .. } = item else {
            panic!("a declaration: {item:?}");
        };
        declarators[0].declaration.ty
    };
    let (first, second) = (ty(first), ty(second));

    let Type::Function {
        parameters: Parameters::Prototype(parameters),
        ..
    } = parsed.ast.ty(first)
    else {
        panic!("a prototype");
    };
    let spelled = |id| crate::ast::spell_type(&parsed.sources, &parsed.ast, id);
    let adjusted: Vec<String> = parameters.iter().map(|p| spelled(p.ty)).collect();
    let written: Vec<String> = parameters.iter().map(|p| spelled(p.written)).collect();
    assert_eq!(adjusted, ["int *", "int (*)(void)"]);
    assert_eq!(written, ["int[4]", "int (void)"]);

    assert!(parsed.ast.compatible(first, second));
    assert!(!parsed.diagnostics.has_errors());
}
