//! What the driver writes for `--emit tokens`, `--emit ast` and `--emit hatches`:
//! a token a line, the tree a node a line, and every hatch with what the checks
//! concluded inside it.
//!
//! `--emit safety-ir` is not here: what an IR line says is a fact about the IR,
//! and it is written by `safec_ir::print`.

use std::fmt::Write as _;

use crate::ast::{
    Ast, Declaration, Expr, ExprId, ForStart, InitDeclarator, Item, Parameters, Stmt, StmtId, Type,
    TypeId, spell_type,
};
use crate::diagnostics::Diagnostic;
use crate::token::Token;
use safec_ir::analysis::Conclusion;
use safec_ir::ir::TranslationUnit;
use safec_ir::print::{dump_node, quoted, shown};
use safec_ir::source::{SourceFile, SourceMap, Span};

use super::Hatched;

/// Every hatch in `unit`, and under each what the checks concluded inside it.
///
/// One line per hatch, at its name, and one line per conclusion, at its caret,
/// in the order a reader meets them in the file. Each conclusion says its
/// code, what was concluded, and its message. A hatch nothing was concluded
/// in is listed all the same, which is what makes this the count of them: at
/// `--safety off` no check runs and every hatch is listed with nothing under
/// it.
///
/// Every line begins with [`dump_node`], so a file's name is escaped the way it
/// is in every other artifact, and the function's name is the file's text and
/// is written with `{:?}`, because source text is content.
pub(super) fn dump_hatches(
    sources: &SourceMap,
    unit: &TranslationUnit,
    hatched: &[Hatched],
    out: &mut String,
) {
    for id in unit.functions() {
        let function = unit.function(id);
        if !function.hatch() {
            continue;
        }

        dump_node(sources, "Hatch", function.name, 0, out);
        writeln!(out, " {:?}", quoted(sources, function.name))
            .expect("writing to a string cannot fail");

        let mut inside: Vec<&Hatched> = hatched
            .iter()
            .filter(|concluded| concluded.function == id)
            .collect();
        inside.sort_by_key(|concluded| {
            let at = caret(&concluded.diagnostic);
            (at.file().index(), at.start())
        });

        for concluded in inside {
            dump_node(sources, "Conclusion", caret(&concluded.diagnostic), 1, out);
            let code = concluded
                .diagnostic
                .code()
                .map_or(String::new(), |code| code.as_str().to_owned());
            // In `docs/safety-model.md`'s three words, so that the one a
            // reader does not see here yet reads alike when it arrives.
            let said = match concluded.conclusion {
                Conclusion::Safe => "safe",
                Conclusion::Unsafe => "unsafe",
                Conclusion::Unknown => "unknown",
            };
            writeln!(
                out,
                " {:?} {:?} {:?}",
                code,
                said,
                concluded.diagnostic.message()
            )
            .expect("writing to a string cannot fail");
        }
    }
}

/// Where a check's diagnostic puts its caret.
///
/// Every one `memory_finding` and `nullability_finding` build has a primary
/// label, because a conclusion is about a place in the program.
fn caret(diagnostic: &Diagnostic) -> Span {
    diagnostic
        .primary_label()
        .expect("a check's diagnostic points at what it concluded about")
        .span()
}

/// The tree, as a caller redirecting it would see.
///
/// One node per line, two spaces of indent per level: the kind, where it is,
/// and whatever that node alone carries. Past
/// [`safec_ir::print::DEEPEST_INDENT`] levels the indent stops growing and the
/// line says how many it is short by, which is where the depth of a tree
/// nothing bounds stops being a number of spaces.
///
/// **No node identity.** `clang -Xclang -ast-dump` prints one and it is the
/// node's address, so two runs of the same command on the same file disagree.
/// A corpus case compares byte for byte, and an artifact nobody can pin is an
/// interface nobody can hold this compiler to.
pub(super) fn dump_ast(sources: &SourceMap, ast: &Ast, out: &mut String) {
    for item in ast.items() {
        dump_item(sources, ast, item, 0, out);
    }
}

fn dump_item(sources: &SourceMap, ast: &Ast, item: &Item, depth: usize, out: &mut String) {
    dump_node(sources, item.name(), item.span(), depth, out);
    match item {
        Item::Function(function) => {
            write!(
                out,
                " {:?} {:?}",
                quoted(sources, function.name),
                spell_type(sources, ast, function.ty)
            )
            .expect("writing to a string cannot fail");
            // Where a declaration of it prints its own, and for the reason
            // `dump_declaration` gives.
            if let Some(written) = function.return_nullability {
                write!(out, " {}", written.specifier.spelling())
                    .expect("writing to a string cannot fail");
            }
            // The name and the string as written, because the tree records what
            // was read and `sema::resolve` is what says whether it is a hatch.
            // Both are the file's text, written with `{:?}` because source
            // text is content.
            if let Some(attribute) = function.attribute {
                write!(
                    out,
                    " __attribute__ {:?} {:?}",
                    quoted(sources, attribute.name),
                    quoted(sources, attribute.argument)
                )
                .expect("writing to a string cannot fail");
            }
            out.push('\n');
            dump_parameters(sources, ast, function.ty, depth + 1, out);
            dump_stmt(sources, ast, ast.stmt(function.body), depth + 1, out);
        }
        Item::Declaration { declarators, span } => {
            dump_declarators(sources, ast, declarators, item.name(), *span, depth, out);
        }
        Item::Error { .. } => out.push('\n'),
    }
}

/// Every declarator of one declaration, a line each.
///
/// `dump_node` has already written the line the first one goes on, so each one
/// after it opens a line of its own at the same depth and with the same span.
/// That is what keeps `int x;` printing exactly what it printed before there
/// was a list at all, and it is honest about `int a, b;`: both declarators do
/// begin at the specifiers, which is what [`Declaration::span`] says and why
/// the two lines carry the same position.
///
/// [`Declaration::span`]: crate::ast::Declaration::span
fn dump_declarators(
    sources: &SourceMap,
    ast: &Ast,
    declarators: &[InitDeclarator],
    name: &'static str,
    span: Span,
    depth: usize,
    out: &mut String,
) {
    if declarators.is_empty() {
        // `dump_node` has written a prefix and nothing below would end the
        // line. Both variants say in their doc comments that a list is never
        // empty, and #34 is the change that would make one: this is the line
        // it has to find.
        out.push('\n');
        return;
    }

    for (at, declarator) in declarators.iter().enumerate() {
        if at > 0 {
            dump_node(sources, name, span, depth, out);
        }

        dump_declaration(sources, ast, &declarator.declaration, out);
        dump_parameters(sources, ast, declarator.declaration.ty, depth + 1, out);
        if let Some(init) = declarator.init {
            dump_expr(sources, ast, init, depth + 1, out);
        }
    }
}

/// The tail of a line that declares something: the name, the type, and the
/// nullability specifier where one was written.
///
/// A name is the file's own bytes and is quoted because source text is
/// content. A
/// type is this compiler's spelling of what the declarator derived, quoted
/// beside it so that the two read alike; the only file text inside one is the
/// length of an array, which [`spell_type`] answers for.
///
/// The specifier is a word on the line rather than part of the type string,
/// because it is not part of the type: [`Declaration::nullability`] says why.
fn dump_declaration(sources: &SourceMap, ast: &Ast, declaration: &Declaration, out: &mut String) {
    if let Some(name) = declaration.name {
        write!(out, " {:?}", quoted(sources, name)).expect("writing to a string cannot fail");
    }
    write!(out, " {:?}", spell_type(sources, ast, declaration.written))
        .expect("writing to a string cannot fail");
    if let Some(written) = declaration.nullability {
        write!(out, " {}", written.specifier.spelling()).expect("writing to a string cannot fail");
    }
    out.push('\n');
}

/// The parameters a declarator named, where the type is directly a function.
///
/// Only directly. A parameter of `int (*g)(int x)` is inside a pointer, and
/// what it is called is not something anything can refer to, so the type string
/// is where it stays. What a definition writes is reachable, and #27 resolves
/// it, so it gets a line of its own.
fn dump_parameters(sources: &SourceMap, ast: &Ast, ty: TypeId, depth: usize, out: &mut String) {
    let Type::Function {
        parameters: Parameters::Prototype(parameters),
        ..
    } = ast.ty(ty)
    else {
        return;
    };

    for parameter in parameters {
        dump_node(sources, "Parameter", parameter.span, depth, out);
        dump_declaration(sources, ast, parameter, out);
    }
}

/// One statement and everything under it.
///
/// A recursion, unlike `dump_expr`, and it can be one because every place a
/// statement nests inside another is a recursion in the parser too, and
/// `MAX_NESTING` bounds those. `dump_expr` needs its own stack because an
/// expression tree does not have that property: two of its rules fold with a
/// loop.
///
/// Which optional parts were written goes on the node's own line rather than
/// into a child, because a child would be a line for something the source does
/// not contain, and every line in this artifact carries a position. `clang`
/// says `has_else` for the same reason. Without it `for (i;;) ;` and
/// `for (;;i) ;` would be the same two lines.
fn dump_stmt(sources: &SourceMap, ast: &Ast, stmt: &Stmt, depth: usize, out: &mut String) {
    dump_node(sources, stmt.name(), stmt.span(), depth, out);

    let child =
        |id: StmtId, out: &mut String| dump_stmt(sources, ast, ast.stmt(id), depth + 1, out);

    match stmt {
        Stmt::Compound { body, .. } => {
            out.push('\n');
            for &id in body {
                child(id, out);
            }
        }
        Stmt::Return { value, .. } => {
            out.push('\n');
            if let Some(id) = value {
                dump_expr(sources, ast, *id, depth + 1, out);
            }
        }
        Stmt::Declaration { declarators, span } => {
            dump_declarators(sources, ast, declarators, stmt.name(), *span, depth, out);
        }
        Stmt::Expression { value, .. } => {
            out.push('\n');
            if let Some(id) = value {
                dump_expr(sources, ast, *id, depth + 1, out);
            }
        }
        Stmt::If {
            condition,
            then,
            otherwise,
            ..
        } => {
            // Redundant here, where two children mean no `else` and three mean
            // one, and not redundant on a `for`. One rule for both is worth
            // more than the line it saves.
            if otherwise.is_some() {
                out.push_str(" else");
            }
            out.push('\n');
            dump_expr(sources, ast, *condition, depth + 1, out);
            child(*then, out);
            if let Some(otherwise) = otherwise {
                child(*otherwise, out);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            out.push('\n');
            dump_expr(sources, ast, *condition, depth + 1, out);
            child(*body, out);
        }
        Stmt::For {
            start,
            condition,
            step,
            body,
            ..
        } => {
            // `init` for an expression start, as before, and `declaration`
            // for the other form, which is printed as the statement it is.
            let (initialiser, declaration) = match *start {
                Some(ForStart::Expression(initialiser)) => (Some(initialiser), None),
                Some(ForStart::Declaration(declaration)) => (None, Some(declaration)),
                None => (None, None),
            };
            if declaration.is_some() {
                out.push_str(" declaration");
            }
            for (clause, name) in [
                (initialiser, "init"),
                (*condition, "condition"),
                (*step, "step"),
            ] {
                if clause.is_some() {
                    write!(out, " {name}").expect("writing to a string cannot fail");
                }
            }
            out.push('\n');
            if let Some(declaration) = declaration {
                child(declaration, out);
            }
            for clause in [initialiser, *condition, *step].into_iter().flatten() {
                dump_expr(sources, ast, clause, depth + 1, out);
            }
            child(*body, out);
        }
        Stmt::Error { .. } => out.push('\n'),
    }
}

/// One expression and everything under it.
///
/// **An explicit stack and not recursion.** The tree can be deeper than the
/// parser ever went: `MAX_NESTING` bounds the parser's own recursion, and a
/// left-associative chain (`a + a + ...`) and a run of postfix operators
/// (`a++++`) are folded by a loop, so each adds a level to the tree without the
/// parser calling itself once. A thousand of either is an ordinary generated
/// line, and walking it recursively ended the process at around a thousand with
/// no diagnostic and an exit code nothing here chose. Every later walk of this
/// tree owes itself the same answer, and owes it here rather than borrowing
/// this one: [`Expr::extend_children`] is the half that can be shared, and the stack
/// is the half that cannot.
///
/// A node writes at most one quoted thing after its position: either the file's
/// own text, or the operator this compiler spells. The two are not the same
/// kind of thing. `Number` and `Identifier` echo the source, which is content
/// and why they are quoted; an operator comes from `BinOp::as_str` and is
/// this compiler's own word, so `a  +  b` still prints `"+"`.
fn dump_expr(sources: &SourceMap, ast: &Ast, root: ExprId, depth: usize, out: &mut String) {
    // Children are pushed in reverse, so that they come back off in the order
    // they were written. What that makes is a pre-order walk, the same one the
    // recursive version made.
    let mut pending = vec![(root, depth)];
    // Cleared per node, because `extend_children` appends: this wants the
    // children of one node, not of every node so far. Reused rather than built
    // afresh so that a whole walk allocates once.
    let mut children = Vec::new();

    while let Some((id, depth)) = pending.pop() {
        let expr = ast.expr(id);
        dump_node(sources, expr.name(), expr.span(), depth, out);

        // What this node says about itself, and nothing about what is under it.
        match expr {
            Expr::Number { span } | Expr::Identifier { span } => {
                write!(out, " {:?}", quoted(sources, *span))
                    .expect("writing to a string cannot fail");
            }
            Expr::Unary { op, .. } => {
                // `++` and `--` are the only operators C writes on either side,
                // so they are the only ones that need saying which side this
                // was.
                if let Some(fixity) = op.fixity() {
                    write!(out, " {fixity}").expect("writing to a string cannot fail");
                }
                write!(out, " {:?}", op.as_str()).expect("writing to a string cannot fail");
            }
            Expr::Binary { op, .. } => {
                write!(out, " {:?}", op.as_str()).expect("writing to a string cannot fail");
            }
            Expr::Assign { op, .. } => {
                // `+=` is `+` and `=`, built rather than tabulated: eleven more
                // spellings in a second table is a second table to disagree
                // with the first, and two spellings of one decision have
                // already left a case here answered by neither.
                let spelling = match op {
                    Some(op) => format!("{}=", op.as_str()),
                    None => "=".to_owned(),
                };
                write!(out, " {spelling:?}").expect("writing to a string cannot fail");
            }
            Expr::Comma { .. } => {
                write!(out, " {:?}", ",").expect("writing to a string cannot fail");
            }
            Expr::Conditional { .. }
            | Expr::Call { .. }
            | Expr::Subscript { .. }
            | Expr::Error { .. } => {}
        }
        out.push('\n');

        children.clear();
        expr.extend_children(&mut children);
        for &child in children.iter().rev() {
            pending.push((child, depth + 1));
        }
    }
}

/// One line per token: where it starts, what it is, and the text it covers.
///
/// The text is quoted rather than written plainly. It comes out of the file, so
/// it is content, and this goes to a terminal: quoting escapes a control
/// character rather than obeying it, for the same reason the renderer does not
/// echo one, and it makes a token legible whose text is a space or a newline.
///
/// A position rather than a span, because the quoted text says how far the
/// token reaches and a dump is read down its left edge. Not because the next
/// token begins where this one ends: trivia sits between them more often than
/// not.
pub(super) fn dump_tokens(file: &SourceFile, tokens: &[Token], out: &mut String) {
    for token in tokens {
        let at = file.line_col(token.span.start());
        write!(
            out,
            "{}:{}:{} {}",
            shown(&file.name().to_string()),
            at.line,
            at.column,
            token.kind.name()
        )
        .expect("writing to a string cannot fail");

        if !token.is_eof() {
            write!(out, " {:?}", &file.contents()[token.span.range()])
                .expect("writing to a string cannot fail");
        }
        out.push('\n');
    }
}
