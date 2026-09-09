//! The driver: everything between a parsed command line and an exit code.
//!
//! The phases each know how to do one thing. What is decided here is the order
//! they happen in: which files are read, when diagnostics are rendered, and
//! what the process reports. Keeping that in one place is what lets a phase be
//! written without an opinion about the ones around it.
//!
//! [`compile`] does the work and hands back what it found, so that a caller can
//! read the diagnostics rather than scrape them out of a stream. [`run_compiler`] is the
//! half that reports and decides the outcome.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::ast::{Ast, Declaration, Expr, ExprId, Item, Parameters, Stmt, StmtId, Type, TypeId};
use crate::diagnostics::render::Renderer;
use crate::diagnostics::{Diagnostic, DiagnosticSink, Policy};
use crate::lexer::lex;
use crate::options::{EmitKind, Options};
use crate::parser::parse;
use crate::source::{SourceFile, SourceMap, Span};
use crate::token::Token;

/// Everything one run of the compiler produced.
///
/// Not `Compilation`, which both reference compilers already use and for the
/// opposite thing: clang builds one before running anything, as the list of
/// jobs it is about to perform, and rustc uses it for `{Stop, Continue}`. This
/// is the far end of a run, and the other name is worth leaving free for the
/// job list that `--emit object` and `--emit executable` will eventually want.
#[derive(Debug)]
pub struct Compiled {
    /// Every file that was read.
    pub sources: SourceMap,
    /// Everything the compiler had to say about them.
    pub diagnostics: DiagnosticSink,
    /// What `--emit` asked for, if the pipeline reaches that far.
    ///
    /// `None` means the run was asked for something it cannot produce yet, and
    /// the diagnostics say so. It does not mean nothing was written: a run that
    /// reported a lexical error still emits the tokens, because they are still
    /// what was asked for and they are still worth reading.
    pub artifact: Option<String>,
}

/// What a run amounted to.
///
/// Separate from [`ExitCode`], which is the process's answer rather than the
/// compiler's: it can be built but never read back, so a caller that is not
/// `main` can do nothing with one. This says what happened, and `main` decides
/// what to exit with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing reported an error.
    Succeeded,
    /// Something did, and the diagnostics say what.
    Failed,
}

impl From<Outcome> for ExitCode {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Succeeded => ExitCode::SUCCESS,
            Outcome::Failed => ExitCode::FAILURE,
        }
    }
}

/// Run the compiler over `options`, rendering nothing.
///
/// Split from [`run_compiler`] so that a test can inspect what was reported
/// instead of reading it back out of a stream.
///
/// Phase 0 has one phase, so nothing here is gated on what an earlier one
/// found. That changes with the lexer: the moment `compile` does per-input work
/// beyond loading it, the gate for that work goes on the input and not on the
/// run. [`DiagnosticSink::has_errors`] answers whether anything in the whole run
/// failed, so gating the lexer on it would let a typo in `a.c` decide that `b.c`
/// is never looked at, and a user with two broken files would fix them one run
/// at a time. The run-level answer is for what genuinely spans the run: the
/// outcome, and later, linking.
pub fn compile(options: &Options) -> Compiled {
    // The one place a sink is built from the options, so that no part of the
    // compiler can invent its own policy. See ADR-0001.
    let mut diagnostics = DiagnosticSink::with_policy(Policy::from(options));
    let mut sources = SourceMap::new();

    if options.inputs.is_empty() {
        // Not reachable through the command line, which requires an input, but
        // `Options` is built directly by everything that is not the parser.
        // Such a run would otherwise be told only that the pipeline is missing,
        // which is true and beside the point.
        diagnostics.report(Diagnostic::error("no input files"));
    }

    let mut seen = HashSet::new();
    let mut loaded = Vec::new();
    for input in &options.inputs {
        // The same path twice is one translation unit, not two: reading it
        // twice would report everything in it twice. `cc` answers differently,
        // compiling the file twice so that the link fails on a duplicate
        // symbol, which is a divergence taken on purpose: saying everything a
        // user has to read twice is the worse of the two answers for something
        // whose output is diagnostics.
        //
        // `Path` compares by component rather than by spelling, so `dir/./a.c`
        // is caught along with an identical spelling. `a.c` and `./a.c` are
        // still two, and so are `main.c` and `MAIN.C` on a filesystem that says
        // otherwise. Deciding when two paths name one file belongs to the
        // source map, and `#include` is what will make it worth deciding.
        if !seen.insert(input.as_path()) {
            continue;
        }

        // Every input is attempted. Reporting the first bad path and stopping
        // would make a user with three of them run the compiler three times.
        match sources.load(input) {
            Ok(file) => loaded.push(file),
            Err(error) => diagnostics.report(load_failure(input, &error)),
        }
    }

    // The gate is the input, not the run. `DiagnosticSink::has_errors` answers
    // for everything reported so far, so consulting it here would let a typo in
    // `a.c` decide that `b.c` is never looked at, and a user with two broken
    // files would fix them one run at a time. What genuinely spans the run is
    // the outcome, and later, linking.
    // What the pipeline can produce, decided once, for every kind there is.
    //
    // A `match` rather than a comparison on `EmitKind`'s pipeline order. A
    // comparison answers "beyond the last stage that exists" and never "before
    // the first one", and a pipeline grows at both ends: C runs the
    // preprocessor before the lexer, so `--emit preprocessed` belongs above
    // `Tokens` in that enum, and a `>` gate let such a run exit successfully
    // having produced nothing and said nothing.
    //
    // Exhaustive, so a kind added anywhere is a compile error until somebody
    // says which side of the line it falls on.
    let mut artifact = match options.emit {
        EmitKind::Tokens => Some(Emitted::Tokens(String::new())),
        EmitKind::Ast => Some(Emitted::Ast(String::new())),
        EmitKind::SafetyIr | EmitKind::LlvmIr | EmitKind::Object | EmitKind::Executable => None,
    };
    for &file in &loaded {
        // `file_owned` rather than `file`: the scan holds its text for longer
        // than a statement, and adds to the map while doing so once `#include`
        // lands. See ADR-0005.
        let source = sources.file_owned(file);
        let tokens = lex(file, &source, &mut diagnostics);

        // Every input appends to one artifact, and every line names its file,
        // so `--emit tokens a.c b.c` reads as one dump rather than needing two
        // destinations.
        // The parser runs whatever the lexer found in this file. Gating it on
        // that is the second half of per-input gating and has an issue of its
        // own; the half that exists is the loop above, which attempts every
        // input rather than stopping at the first bad one.
        match &mut artifact {
            Some(Emitted::Tokens(out)) => dump_tokens(&source, &tokens, out),
            Some(Emitted::Ast(out)) => {
                let ast = parse(file, &tokens, &mut diagnostics);
                dump_ast(&sources, &ast, out);
            }
            None => {}
        }
    }

    // Accepted by the parser and acted on by nothing. Saying so is not the
    // same as implementing it, and it is the half that cannot wait: a run that
    // was told where to put its output, put it somewhere else, and exited
    // successfully has lied about the one thing the exit code is for. What `-o`
    // should mean for several inputs, one artifact and no linker is a decision
    // for when there is a backend to make it with.
    if options.output.is_some() {
        diagnostics.report(
            Diagnostic::error("`-o` is not supported yet")
                .with_note("the artifact is written to standard output")
                .with_note("redirect it instead, until an output path is honoured"),
        );
    }

    // The other half of the same decision, and it reads from the same answer
    // rather than asking again. Said whenever it applies, including on a run
    // that also failed to read a file, because a run that compiled nothing has
    // to say so: leaving it out lets a user with one bad path among several
    // believe the rest were built. Exiting successfully without producing what
    // was asked for is the one thing a compiler must never do.
    if artifact.is_none() {
        diagnostics.report(
            Diagnostic::error("the compilation pipeline is not implemented yet")
                .with_note("safec currently scans its inputs and parses them into a tree")
                .with_note("`--emit tokens` and `--emit ast` are what it can produce today"),
        );
    }

    Compiled {
        sources,
        diagnostics,
        artifact: artifact.map(Emitted::into_text),
    }
}

/// The artifact being built, and which one it is.
///
/// The gate that decides whether to build anything runs once, before the
/// inputs, and the dump runs once per input, so the two are in different
/// places. Naming the artifact keeps them one decision rather than two matches
/// on `EmitKind` that have to agree. RK-003 in the review knowledge bank
/// records what the two-copy version of this cost: a gate written as two
/// comparisons left `--emit preprocessed` answered by neither, and the compiler
/// exited zero having written nothing to either stream.
enum Emitted {
    /// What `--emit tokens` asked for.
    Tokens(String),
    /// What `--emit ast` asked for.
    Ast(String),
}

impl Emitted {
    fn into_text(self) -> String {
        match self {
            Self::Tokens(text) | Self::Ast(text) => text,
        }
    }
}

/// The tree, as a caller redirecting it would see.
///
/// One node per line, two spaces of indent per level: the kind, where it is,
/// and whatever that node alone carries. Past [`DEEPEST_INDENT`] levels the
/// indent stops growing and the line says how many it is short by, which is
/// where the depth of a tree nothing bounds stops being a number of spaces.
///
/// **No node identity.** `clang -Xclang -ast-dump` prints one and it is the
/// node's address, so two runs of the same command on the same file disagree.
/// A corpus case compares byte for byte, and an artifact nobody can pin is an
/// interface nobody can hold this compiler to.
fn dump_ast(sources: &SourceMap, ast: &Ast, out: &mut String) {
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
            out.push('\n');
            dump_parameters(sources, ast, function.ty, depth + 1, out);
            dump_stmt(sources, ast, ast.stmt(function.body), depth + 1, out);
        }
        Item::Declaration(declaration) => {
            dump_declaration(sources, ast, declaration, out);
            dump_parameters(sources, ast, declaration.ty, depth + 1, out);
        }
        Item::Error { .. } => out.push('\n'),
    }
}

/// The tail of a line that declares something: the name, then the type.
///
/// A name is the file's own bytes and is quoted for the reason RK-002 gives. A
/// type is this compiler's spelling of what the declarator derived, quoted
/// beside it so that the two read alike; the only file text inside one is the
/// length of an array, which [`spell_type`] answers for.
fn dump_declaration(sources: &SourceMap, ast: &Ast, declaration: &Declaration, out: &mut String) {
    if let Some(name) = declaration.name {
        write!(out, " {:?}", quoted(sources, name)).expect("writing to a string cannot fail");
    }
    write!(out, " {:?}", spell_type(sources, ast, declaration.ty))
        .expect("writing to a string cannot fail");
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
        Stmt::Declaration(declaration) => {
            dump_declaration(sources, ast, declaration, out);
            dump_parameters(sources, ast, declaration.ty, depth + 1, out);
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
            initialiser,
            condition,
            step,
            body,
            ..
        } => {
            for (clause, name) in [
                (initialiser, "init"),
                (condition, "condition"),
                (step, "step"),
            ] {
                if clause.is_some() {
                    write!(out, " {name}").expect("writing to a string cannot fail");
                }
            }
            out.push('\n');
            for clause in [initialiser, condition, step].into_iter().flatten() {
                dump_expr(sources, ast, *clause, depth + 1, out);
            }
            child(*body, out);
        }
        Stmt::Error { .. } => out.push('\n'),
    }
}

/// A type, in the declarator notation C itself writes.
///
/// `int *`, `int[10]`, `int (*)(int)`: the same spelling `clang` prints, so the
/// two dumps can be diffed rather than read against one another. It diverges in
/// one place, and deliberately: `clang` shows a parameter after the adjustments
/// C17 6.7.6.3 p7 and p8 make, so it spells `int f(int [10])` as `int (int *)`,
/// where this spells it `int (int[10])`. Those adjustments are semantic rules,
/// and this stage records what was written.
///
/// The walk down the spine is a loop and not a recursion, so a type of ten
/// thousand pointers costs no stack. Only a parameter list recurses, and how
/// deeply one can nest is bounded by the parser. `Parser::apply` bounds the
/// spine too, which is what every other walk of a type will rely on; the loop
/// here means this one does not have to.
fn spell_type(sources: &SourceMap, ast: &Ast, id: TypeId) -> String {
    let mut id = id;
    let mut inner = String::new();

    loop {
        let base = match ast.ty(id) {
            Type::Int => "int",
            Type::Char => "char",
            Type::Void => "void",
            Type::Pointer(pointee) => {
                inner = if binds_tighter_than_a_pointer(ast.ty(*pointee)) {
                    format!("(*{inner})")
                } else {
                    format!("*{inner}")
                };
                id = *pointee;
                continue;
            }
            Type::Array { element, length } => {
                // The length is the source's own bytes and is not evaluated, so
                // `int a[1 + 2]` spells `int[1 + 2]` where `clang`, which does
                // evaluate it, spells `int[3]`. What 6.7.6.2 p1 asks of it is a
                // constraint, and constraints are checked later.
                let length = match length {
                    Some(length) => quoted(sources, ast.expr(*length).span()),
                    None => "",
                };
                inner = format!("{inner}[{length}]");
                id = *element;
                continue;
            }
            Type::Function {
                returns,
                parameters,
            } => {
                inner = format!("{inner}({})", spell_parameters(sources, ast, parameters));
                id = *returns;
                continue;
            }
        };

        // A space between the base and what follows, unless there is nothing
        // to follow or it begins with `[`. That is the spacing `clang` prints:
        // `int *`, `int (*)(int)`, and `int[10]` with no space at all.
        return if inner.is_empty() || inner.starts_with('[') {
            format!("{base}{inner}")
        } else {
            format!("{base} {inner}")
        };
    }
}

/// Whether a derivation binds its operand more tightly than a pointer does.
///
/// An array and a function do, so a pointer to either is written with the `*`
/// in parentheses to say that the pointer is the outer one. That rule alone is
/// what makes `int (*)(int)` and `int *(int)` two different strings, and C17
/// 6.7.6 p6 is where the binding it reflects is stated.
///
/// An exhaustive `match` and not a `matches!`, for the reason the emit gate
/// above gives: a `matches!` answers `false` for a variant nobody has thought
/// about, and the answer here is the one thing that tells two types apart in
/// the artifact. `[*]`, which `parser.rs` already lists as a shape it does not
/// read yet, is a variant this will have to answer for.
fn binds_tighter_than_a_pointer(ty: &Type) -> bool {
    match ty {
        Type::Array { .. } | Type::Function { .. } => true,
        Type::Int | Type::Char | Type::Void | Type::Pointer(_) => false,
    }
}

/// What goes between a function type's parentheses.
///
/// `(void)` for a prototype that declared no parameters and `()` for an empty
/// identifier list, because C17 6.7.6.3 gives the two spellings two meanings.
fn spell_parameters(sources: &SourceMap, ast: &Ast, parameters: &Parameters) -> String {
    let Parameters::Prototype(parameters) = parameters else {
        return String::new();
    };

    if parameters.is_empty() {
        return "void".to_owned();
    }

    parameters
        .iter()
        .map(|parameter| spell_type(sources, ast, parameter.ty))
        .collect::<Vec<_>>()
        .join(", ")
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
/// kind of thing. `Number` and `Identifier` echo the source, which is why
/// RK-002 asks for the quoting; an operator comes from `BinOp::as_str` and is
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
                // with the first, which is what RK-003 records the cost of.
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

/// The source text a span covers, resolved against the file the span names.
///
/// Not against whichever file the driver's loop is on. One tree holds spans
/// from one file today and will hold several the moment `#include` lands, and a
/// span resolved against the wrong file prints another file's text at another
/// file's line, or panics when that file is shorter. `render.rs` looks the file
/// up per label for this reason, and ADR-0003 is where it is argued.
fn quoted(sources: &SourceMap, span: Span) -> &str {
    &sources.file(span.file()).contents()[span.range()]
}

/// How deep the artifact indents before it starts counting instead.
///
/// Past this a line carries `+N`, the levels the indent no longer shows, so a
/// reader keeps the depth where the shape has run out. Two things make that
/// the right answer rather than a compromise. A dump is read down its left
/// edge, and nothing is read down a left edge a hundred levels out. And the
/// indent is what made this artifact quadratic: `n` nodes each indented by `n`
/// is `n` squared bytes, so 20 KB of the generated C that
/// `a_long_flat_expression_does_not_end_the_process` describes printed 51 MB.
///
/// A cap on a display rather than a measurement. 32 levels of two spaces each
/// is 64 columns, already more indentation than a dump is read at, and that is
/// what picks the number; nothing about the language or the tree does. The two
/// spaces are in `dump_node` below, so the two move together.
const DEEPEST_INDENT: usize = 32;

/// The part every line shares: indent, kind, and where it is.
///
/// Source text is written with `{:?}` by the callers that write any, for the
/// reason RK-002 records: a `.c` file's own bytes reaching a stream are
/// content, and one holding an escape sequence must not be able to clear the
/// terminal of whoever compiled it.
///
/// **The indent is written rather than passed to `write!` as a width.** Rust's
/// format width is a `u16` and `depth` is bounded by nothing: [`dump_expr`]
/// says why, and the cost of not knowing it was that a tree 32768 levels deep
/// panicked inside `write!`, before `expect` could see a `Result`, for exit 101
/// with nothing on either stream. 32768 is where it starts: two spaces a level
/// is a width of 65536, and 65535 is the largest a `u16` holds. The smallest
/// `.c` file reaching it is 96 KB of `i[i][i]...`, 32764 subscripts, which
/// `clang` parses.
fn dump_node(sources: &SourceMap, kind: &str, span: Span, depth: usize, out: &mut String) {
    let file = sources.file(span.file());
    let at = file.line_col(span.start());

    for _ in 0..depth.min(DEEPEST_INDENT) {
        out.push_str("  ");
    }
    if depth > DEEPEST_INDENT {
        write!(out, "+{} ", depth - DEEPEST_INDENT).expect("writing to a string cannot fail");
    }

    write!(out, "{} {}:{}:{}", kind, file.name(), at.line, at.column)
        .expect("writing to a string cannot fail");
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
fn dump_tokens(file: &SourceFile, tokens: &[Token], out: &mut String) {
    for token in tokens {
        let at = file.line_col(token.span.start());
        write!(
            out,
            "{}:{}:{} {}",
            file.name(),
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

/// Why a file could not be turned into source.
///
/// [`SourceMap::load`] reads and decodes in one step, so one error covers both,
/// and the two are worth telling apart. Saying a file cannot be read when it
/// reads perfectly and only the decoding failed points the user at paths and
/// permissions, and a C file with a Latin-1 or Shift-JIS byte in a comment is
/// ordinary in the code this compiler exists to accept.
fn load_failure(path: &Path, error: &io::Error) -> Diagnostic {
    if error.kind() == io::ErrorKind::InvalidData {
        Diagnostic::error(format!("`{}` is not valid UTF-8", path.display()))
            .with_note("safec reads source files as UTF-8")
            .with_note(error.to_string())
    } else {
        Diagnostic::error(format!("cannot read `{}`", path.display())).with_note(error.to_string())
    }
}

/// Run the compiler and write what it found to `report`.
///
/// Named the way rustc names its entry point rather than `run`, which the
/// Safety IR interpreter will want: once a program can be executed in process,
/// "run" means two different things and only one of them returns the outcome of
/// a compilation.
///
/// The outcome follows `cc`: a run that reported an error failed. Two belongs
/// to the argument parser, which uses it for an invocation it could not
/// understand, so nothing here returns it.
///
/// `report` is a parameter rather than `io::stderr()` so that a caller can read
/// what a run said without spawning a process. Colour is still resolved against
/// stderr, because that is where the binary sends this; a caller writing
/// somewhere else should ask for `Never` or `Always` rather than `Auto`.
///
/// `artifact` is the other stream, and it is separate because the two are
/// different kinds of thing. Diagnostics are what the compiler says; an
/// artifact is what it was asked to make. A caller reading one must not be
/// handed the other, which is why the binary sends them to stderr and stdout.
///
/// The report goes first. If the artifact is being piped into something that
/// stops reading, the write fails, and a user who loses the diagnostics as well
/// learns nothing about why.
///
/// # Errors
///
/// If the diagnostics could not be written. There is nothing left to report
/// that on, so the caller has only the outcome to say it with.
pub fn run_compiler(
    options: &Options,
    report: &mut impl io::Write,
    artifact: &mut impl io::Write,
) -> io::Result<Outcome> {
    let compiled = compile(options);

    Renderer::new(options.color).render_all(&compiled.sources, &compiled.diagnostics, report)?;
    if let Some(emitted) = &compiled.artifact {
        artifact.write_all(emitted.as_bytes())?;
    }

    Ok(if compiled.diagnostics.has_errors() {
        Outcome::Failed
    } else {
        Outcome::Succeeded
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use clap::ValueEnum as _;

    use super::*;
    use crate::options::{ColorMode, EmitKind};
    use crate::safety::SafetyLevel;

    fn options(inputs: Vec<PathBuf>) -> Options {
        Options {
            inputs,
            output: None,
            safety: SafetyLevel::Memory,
            emit: EmitKind::Executable,
            deny_unknown: false,
            color: ColorMode::Never,
        }
    }

    /// A file on disk that removes itself. Named per test, because the suite
    /// runs in parallel and the temporary directory is shared.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, contents: impl AsRef<[u8]>) -> Self {
            let path = std::env::temp_dir().join(name);
            fs::write(&path, contents).expect("the temporary directory is writable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn missing_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    /// A stream that is already gone, which is what `safec ... | head -1`
    /// leaves behind.
    struct Closed;

    impl io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Both of a run's streams: what it reported, and what it made.
    fn run(options: &Options) -> (String, String, Outcome) {
        let mut report = Vec::new();
        let mut artifact = Vec::new();
        let outcome = run_compiler(options, &mut report, &mut artifact)
            .expect("writing to a vector cannot fail");
        (
            String::from_utf8(report).expect("the renderer writes text"),
            String::from_utf8(artifact).expect("the emitter writes text"),
            outcome,
        )
    }

    fn rendered(options: &Options) -> (String, Outcome) {
        let (report, _, outcome) = run(options);
        (report, outcome)
    }

    /// The wiring ADR-0001 is about. Without it `--deny-unknown` parses,
    /// documents itself, and does nothing.
    #[test]
    fn the_sink_is_built_from_the_resolved_options() {
        for deny_unknown in [false, true] {
            let mut options = options(Vec::new());
            options.deny_unknown = deny_unknown;

            assert_eq!(
                compile(&options).diagnostics.policy().deny_unknown(),
                deny_unknown,
            );
        }
    }

    /// The strictest level is defined as leaving nothing `Unknown`, so its
    /// implication has to reach the sink even when the flag was not given as
    /// well. The level itself does not: a sink holds a [`Policy`] and never
    /// learns which checks ran.
    #[test]
    fn the_strictest_safety_level_denies_unknown_in_the_sink() {
        let mut options = options(Vec::new());
        options.safety = SafetyLevel::Strict;

        assert!(compile(&options).diagnostics.policy().deny_unknown());
    }

    #[test]
    fn every_input_is_read_in_the_order_it_was_given() {
        let first = TempFile::new("safec_driver_order_a.c", "int a;\n");
        let second = TempFile::new("safec_driver_order_b.c", "int b;\n");

        let compiled = compile(&options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]));

        assert_eq!(compiled.sources.len(), 2);
        let contents: Vec<_> = compiled
            .sources
            .files()
            .map(|(_, file)| file.contents().to_owned())
            .collect();
        assert_eq!(contents, ["int a;\n", "int b;\n"]);
    }

    /// One path named twice is one translation unit. Reading it twice would
    /// report everything in it twice and double the error count a user reads.
    #[test]
    fn the_same_input_twice_is_read_once() {
        let file = TempFile::new("safec_driver_duplicate.c", "int x;\n");
        let path = file.path().to_path_buf();

        let compiled = compile(&options(vec![path.clone(), path]));

        assert_eq!(compiled.sources.len(), 1);
    }

    /// The rule is component equality, which is what `Path` compares, and not
    /// the spelling. A redundant `.` in the middle of a path names the same
    /// file and is caught; a leading `./` is a different first component and is
    /// not. Without both halves the comment above is a claim about behaviour
    /// that nothing holds to, and a later `canonicalize` would arrive looking
    /// like a fix rather than like a change.
    #[test]
    fn two_spellings_are_one_input_only_when_their_components_match() {
        let file = TempFile::new(
            "safec_driver_spelling.c",
            "int x;
",
        );
        let directory = file.path().parent().expect("the file has a parent");
        let name = file.path().file_name().expect("the file has a name");

        let matching = compile(&options(vec![
            directory.join(name),
            directory.join(".").join(name),
        ]));

        assert_eq!(matching.sources.len(), 1);

        let differing = compile(&options(vec![
            PathBuf::from("safec_driver_spelling_relative.c"),
            PathBuf::from("./safec_driver_spelling_relative.c"),
        ]));
        let attempted = differing
            .diagnostics
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.message().contains("cannot read"))
            .count();

        assert_eq!(attempted, 2, "{:?}", differing.diagnostics.diagnostics());
    }

    /// A path that is not there is an ordinary diagnostic. Reaching for
    /// `unwrap` here would answer a typo with a backtrace.
    #[test]
    fn a_missing_input_is_reported_rather_than_panicking() {
        let path = missing_path("safec_driver_no_such_file.c");
        let compiled = compile(&options(vec![path.clone()]));

        let reported = &compiled.diagnostics.diagnostics()[0];
        assert!(
            reported.message().contains(&path.display().to_string()),
            "{:?}",
            reported.message()
        );
        // The message says which file; the note is the only thing that says
        // why, so it carries the actionable half.
        assert!(
            !reported.notes().is_empty(),
            "the reason was dropped: {reported:?}"
        );
        assert!(compiled.diagnostics.has_errors());
        assert!(
            compiled.sources.is_empty(),
            "a failed load must not add a file"
        );
    }

    /// A C file with a Latin-1 comment reads perfectly; only the decoding
    /// fails. Calling that unreadable points the user at permissions.
    ///
    /// The message is asserted rather than the whole diagnostic, because the
    /// message is this compiler's and one of the notes is not: `read_to_string`
    /// contributes `stream did not contain valid UTF-8`. That is `std`'s
    /// wording, which is what keeps the diagnostic out of the corpus, where a
    /// case pins text that is entirely ours.
    ///
    /// Unlike `cannot read`, it does not vary by host: it is a constant in
    /// `std` rather than a message from the operating system, so the reason
    /// `docs/architecture.md` records for that one does not apply here. What
    /// this note follows is the Rust version, not the platform.
    #[test]
    fn a_file_that_is_not_utf8_is_not_reported_as_unreadable() {
        let file = TempFile::new(
            "safec_driver_latin1.c",
            b"/* caf\xE9 */\nint main(void) { return 0; }\n",
        );

        let compiled = compile(&options(vec![file.path().to_path_buf()]));
        let reported = &compiled.diagnostics.diagnostics()[0];

        assert!(
            reported.message().contains("is not valid UTF-8"),
            "{:?}",
            reported.message()
        );
        assert!(!reported.message().contains("cannot read"), "{reported:?}");
        assert!(
            reported.notes().iter().any(|note| note.contains("UTF-8")),
            "{reported:?}"
        );
    }

    /// Stopping at the first bad path would make a user with three of them run
    /// the compiler three times.
    #[test]
    fn every_unreadable_input_is_reported_not_just_the_first() {
        let compiled = compile(&options(vec![
            missing_path("safec_driver_missing_a.c"),
            missing_path("safec_driver_missing_b.c"),
        ]));

        // Two bad paths, plus the statement that nothing was compiled.
        assert_eq!(compiled.diagnostics.error_count(), 3);
    }

    /// Reachable only from an `Options` the parser did not build, which is what
    /// the Clang adapter will do. Saying only that the pipeline is missing
    /// would be true and beside the point.
    #[test]
    fn a_run_with_no_inputs_says_so() {
        let compiled = compile(&options(Vec::new()));

        assert!(
            compiled.diagnostics.diagnostics()[0]
                .message()
                .contains("no input files"),
            "{:?}",
            compiled.diagnostics.diagnostics()[0].message()
        );
    }

    /// The pipeline does not exist, so no artifact was produced, so the run
    /// failed. Reporting success here would be a positive claim that the
    /// compilation happened.
    #[test]
    fn a_compilation_that_produces_no_artifact_reports_an_error() {
        let file = TempFile::new("safec_driver_nothing_yet.c", "int main(void) { return 0; }");
        let compiled = compile(&options(vec![file.path().to_path_buf()]));

        assert!(compiled.diagnostics.has_errors());
        assert!(
            compiled
                .diagnostics
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message().contains("not implemented")),
        );
    }

    /// A run that failed to read one of its inputs has still compiled nothing,
    /// and has to say so. Leaving it out lets a user with one bad path among
    /// several believe the files that did load were checked.
    #[test]
    fn a_failed_load_is_still_told_that_nothing_was_compiled() {
        let good = TempFile::new("safec_driver_good.c", "int x;\n");
        let compiled = compile(&options(vec![
            good.path().to_path_buf(),
            missing_path("safec_driver_absent.c"),
        ]));

        let messages: Vec<_> = compiled
            .diagnostics
            .diagnostics()
            .iter()
            .map(Diagnostic::message)
            .collect();

        assert!(
            messages
                .iter()
                .any(|message| message.contains("cannot read")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("not implemented")),
            "{messages:?}"
        );
    }

    /// `run_compiler` writes where it is told rather than to a stream a test
    /// cannot read. Without this the render-and-decide half is unguarded.
    #[test]
    fn run_compiler_writes_its_report_to_the_writer_it_was_given() {
        let (report, _) = rendered(&options(vec![missing_path("safec_driver_run_report.c")]));

        assert!(report.contains("cannot read"), "{report}");
        assert!(report.contains("safec_driver_run_report.c"), "{report}");
    }

    /// The outcome is what the process reports, so it has to follow what was
    /// reported rather than being decided some other way.
    #[test]
    fn a_run_that_reported_an_error_failed() {
        let file = TempFile::new("safec_driver_outcome.c", "int x;\n");
        let (_, outcome) = rendered(&options(vec![file.path().to_path_buf()]));

        assert_eq!(outcome, Outcome::Failed);
    }

    /// Zero for success and one for failure is what `cc` does and what every
    /// build system reads.
    #[test]
    fn an_outcome_maps_to_the_conventional_exit_code() {
        assert_eq!(ExitCode::from(Outcome::Succeeded), ExitCode::SUCCESS);
        assert_eq!(ExitCode::from(Outcome::Failed), ExitCode::FAILURE);
    }

    /// A run that could not write its report has to say so rather than hand
    /// back an outcome. The outcome is what the process reports, and reporting
    /// one for a run whose diagnostics nobody saw claims the user was told.
    #[test]
    fn a_report_that_could_not_be_written_is_an_error() {
        let options = options(vec![missing_path("safec_driver_closed.c")]);

        let error = run_compiler(&options, &mut Closed, &mut Vec::new())
            .expect_err("a failed write must not come back as an outcome");

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    /// The first artifact the compiler can produce. Each line names where a
    /// token starts, what it is, and the text it covers, which is read out of
    /// the source rather than carried by the token. See ADR-0006.
    #[test]
    fn asking_for_tokens_produces_them() {
        let file = TempFile::new("safec_driver_emit_tokens.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        let lines: Vec<_> = artifact
            .lines()
            .map(|line| line.split_once(' ').unwrap().1)
            .collect();
        assert_eq!(
            lines,
            ["keyword \"int\"", "identifier \"x\"", "punct \";\"", "eof",]
        );
    }

    /// The whole line, not a piece of it. `--emit tokens` is an artifact people
    /// redirect and diff, and `options.rs` calls `--emit` a stable interface
    /// rather than a debug convenience, so the format is the interface: the
    /// position, its order, the separators, the quoting and the trailing
    /// newline. Asserting a substring leaves every one of those free to change.
    #[test]
    fn the_token_dump_has_one_exact_line_per_token() {
        let file = TempFile::new("safec_driver_dump_exact.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        let name = file.path().display();
        assert_eq!(
            artifact,
            format!(
                "{name}:1:1 keyword \"int\"\n\
                 {name}:1:5 identifier \"x\"\n\
                 {name}:1:6 punct \";\"\n\
                 {name}:2:1 eof\n"
            )
        );
    }

    /// A column counts characters, not bytes, so a caret in an editor lands
    /// where the dump says. A tab counts as one, which is what `LineCol` means
    /// and what the renderer's gutter agrees with.
    #[test]
    fn the_token_dump_counts_columns_in_characters() {
        let file = TempFile::new("safec_driver_dump_columns.c", "\tint caf\u{e9};\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        let columns: Vec<_> = artifact
            .lines()
            .map(|line| line.rsplit(':').next().unwrap().split(' ').next().unwrap())
            .collect();
        // `int` at 2, `caf` at 6, the non-ASCII character at 9, `;` at 10.
        // The character is two bytes and advances the column by one, which is
        // the whole point.
        assert_eq!(columns, ["2", "6", "9", "10", "1"], "{artifact}");
    }

    /// The text is quoted rather than written plainly, which is what keeps a
    /// control character out of a source file from reaching the terminal
    /// through stdout. The renderer answers the same question for diagnostics
    /// and has its own tests; this is the other place a file's text is echoed.
    ///
    /// It also keeps a line parseable: a `"` inside a string literal has to be
    /// escaped or the line's own quoting is ambiguous.
    #[test]
    fn the_token_dump_escapes_the_text_it_quotes() {
        let file = TempFile::new(
            "safec_driver_dump_escape.c",
            "s = \"a\u{1b}[2Jb\"; c = '\\\"';\n",
        );
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        assert!(!artifact.contains('\u{1b}'), "{artifact:?}");
        assert!(
            artifact.contains(r#"string "\"a\u{1b}[2Jb\"""#),
            "{artifact:?}"
        );
    }

    /// Every kind a scan can produce has a word, and the words are what a
    /// reader greps for. An exhaustive `match` makes a missing arm a compile
    /// error; nothing makes a wrong word one.
    #[test]
    fn every_token_kind_is_named_in_the_dump() {
        let file = TempFile::new(
            "safec_driver_dump_kinds.c",
            "#define X 1\nint x = 1 + 'c' + @; char *s = \"t\";\n",
        );
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        let named: Vec<_> = artifact
            .lines()
            .filter_map(|line| line.split(' ').nth(1))
            .collect();
        for kind in [
            "keyword",
            "identifier",
            "number",
            "string",
            "character",
            "punct",
            "directive",
            "unknown",
            "eof",
        ] {
            assert!(named.contains(&kind), "no {kind} in {artifact}");
        }
    }

    /// The first run that can succeed. Until `--emit` reached something the
    /// pipeline produces, every run reported that it had built nothing, so
    /// `Outcome::Succeeded` was unreachable and the branch that returns it was
    /// guarded by nothing.
    #[test]
    fn asking_for_tokens_is_a_run_that_can_succeed() {
        let file = TempFile::new("safec_driver_emit_ok.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (report, artifact, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Succeeded);
        assert_eq!(report, "", "a clean run says nothing");
        assert!(!artifact.is_empty());
    }

    /// Accepted and then ignored is the shape of the failure this driver is
    /// most careful about elsewhere: a run that was told where to put its
    /// output, put it somewhere else, and exited zero. Reported until it is
    /// honoured.
    #[test]
    fn an_output_path_that_is_not_honoured_is_reported() {
        let file = TempFile::new(
            "safec_driver_output_path.c",
            "int x;
",
        );
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;
        options.output = Some(PathBuf::from("out.tok"));

        let (report, artifact, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(report.contains("`-o` is not supported yet"), "{report}");
        // Still emitted: the tokens are what was asked for, and stdout is
        // somewhere the caller can reach.
        assert!(artifact.contains("keyword"), "{artifact}");
    }

    /// The rule, over every kind there is: a run either produces what was
    /// asked for or says it cannot. Driven by `EmitKind::value_variants` rather
    /// than by a list, so a kind added anywhere in the pipeline is covered
    /// without anyone remembering this test exists.
    #[test]
    fn every_emit_kind_is_either_produced_or_reported() {
        let file = TempFile::new(
            "safec_driver_emit_every.c",
            "int x;
",
        );

        for &emit in EmitKind::value_variants() {
            let mut options = options(vec![file.path().to_path_buf()]);
            options.emit = emit;

            let compiled = compile(&options);

            assert!(
                compiled.artifact.is_some() || compiled.diagnostics.has_errors(),
                "{emit:?} produced nothing and said nothing",
            );
        }
    }

    /// Everything past the parser. A run that cannot produce what was asked for
    /// has to say so rather than exit successfully having made nothing.
    #[test]
    fn asking_for_an_artifact_the_pipeline_cannot_reach_is_an_error() {
        let file = TempFile::new("safec_driver_emit_beyond.c", "int x;\n");

        for emit in [
            // `Ast` was here until a parser existed to reach it. What is left
            // is everything past the parser, and the list shrinks again each
            // time a phase lands.
            EmitKind::SafetyIr,
            EmitKind::LlvmIr,
            EmitKind::Object,
            EmitKind::Executable,
        ] {
            let mut options = options(vec![file.path().to_path_buf()]);
            options.emit = emit;

            let compiled = compile(&options);

            assert!(compiled.artifact.is_none(), "{emit:?}");
            assert!(compiled.diagnostics.has_errors(), "{emit:?}");
            assert!(
                compiled
                    .diagnostics
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.message().contains("not implemented")),
                "{emit:?}"
            );
        }
    }

    /// A reader of one must not be handed the other. The binary sends them to
    /// stdout and stderr, so `safec --emit tokens a.c > a.tok` captures the
    /// tokens and leaves the diagnostics on the terminal.
    #[test]
    fn the_artifact_and_the_report_go_to_different_streams() {
        let file = TempFile::new("safec_driver_emit_streams.c", "int x = @;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (report, artifact, _) = run(&options);

        assert!(report.contains("unexpected character"), "{report}");
        assert!(!report.contains("keyword"), "{report}");
        assert!(artifact.contains("keyword"), "{artifact}");
        assert!(!artifact.contains("unexpected character"), "{artifact}");
    }

    /// The scan keeps going, so it still has tokens to hand over, and they are
    /// still what was asked for. The run fails on the diagnostics rather than
    /// by withholding the artifact.
    #[test]
    fn a_lexical_error_does_not_withhold_the_tokens() {
        let file = TempFile::new("safec_driver_emit_broken.c", "int x = @;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, outcome) = run(&options);

        assert_eq!(outcome, Outcome::Failed);
        assert!(artifact.contains("unknown \"@\""), "{artifact}");
    }

    /// One dump for the run, not one per input. Every line names its file, so
    /// the concatenation is unambiguous and there is one thing to redirect.
    #[test]
    fn every_input_appends_to_one_artifact() {
        let first = TempFile::new("safec_driver_emit_a.c", "int a;\n");
        let second = TempFile::new("safec_driver_emit_b.c", "int b;\n");
        let mut options = options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]);
        options.emit = EmitKind::Tokens;

        let (_, artifact, _) = run(&options);

        assert!(artifact.contains("safec_driver_emit_a.c"), "{artifact}");
        assert!(artifact.contains("safec_driver_emit_b.c"), "{artifact}");
        assert_eq!(artifact.lines().count(), 8, "{artifact}");
    }

    /// The same for the tree, which reaches the artifact by a different path:
    /// the tokens dump writes what the loop already has, and this one parses
    /// first, so the two arms cannot vouch for each other.
    ///
    /// Mutation: clear the artifact before dumping each input. This fails,
    /// because the second file's tree is then all there is.
    ///
    /// It also pins that each node is placed against the file its own span
    /// names rather than the file the loop is on, which is the property the
    /// second name below would lose.
    #[test]
    fn every_input_appends_to_one_tree_dump() {
        let first = TempFile::new(
            "safec_driver_ast_a.c",
            "int a(void) { return 0; }
",
        );
        let second = TempFile::new(
            "safec_driver_ast_b.c",
            "int b(void) { return 1; }
",
        );
        let mut options = options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]);
        options.emit = EmitKind::Ast;

        let (_, artifact, _) = run(&options);

        assert!(artifact.contains("safec_driver_ast_a.c"), "{artifact}");
        assert!(artifact.contains("safec_driver_ast_b.c"), "{artifact}");
        assert!(artifact.contains("\"a\""), "{artifact}");
        assert!(artifact.contains("\"b\""), "{artifact}");
        assert_eq!(artifact.lines().count(), 8, "{artifact}");
    }

    /// Every shape of type, and how C declares one of it.
    ///
    /// The expected strings are written out rather than derived from the types,
    /// for the reason RK-001 gives: a test that builds its expectation the way
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
        });
        let incomplete = ast.push_type(Type::Array {
            element: int,
            length: None,
        });
        let array_of_pointer = ast.push_type(Type::Array {
            element: pointer_to_int,
            length: Some(ten),
        });
        let pointer_to_array = ast.push_type(Type::Pointer(array_of_int));

        let unnamed = |ty| Declaration {
            name: None,
            ty,
            span: Span::new(file, 0, 2),
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

    /// Each node is placed against the file its own span names.
    ///
    /// One tree holds spans from one file today, and will hold several the
    /// moment `#include` lands. Resolving them all against whichever file the
    /// driver's loop happens to be on prints one file's text at another file's
    /// line, and panics outright when the other file is shorter. The renderer
    /// looks a label's file up per label for the same reason; ADR-0003 argues
    /// it. The tree here is built by hand because the parser cannot yet produce
    /// one that spans two files.
    ///
    /// Mutation: have `dump_node` and `quoted` take the loop's `SourceFile`
    /// again. This fails.
    #[test]
    fn a_node_is_placed_against_the_file_its_span_names() {
        use crate::ast::Function;

        let mut sources = SourceMap::new();
        let first = sources.add_virtual(
            "first.c",
            "int outer(void) { return 0; }
",
        );
        let second = sources.add_virtual(
            "second.c",
            "


      inner
",
        );

        let mut ast = Ast::new();
        let body = ast.push_stmt(Stmt::Compound {
            body: Vec::new(),
            span: Span::new(second, 9, 14),
        });
        let ty = ast.push_type(Type::Int);
        ast.push_item(Item::Function(Function {
            ty,
            name: Span::new(first, 4, 9),
            body,
            span: Span::new(first, 0, 29),
        }));

        let mut out = String::new();
        dump_ast(&sources, &ast, &mut out);

        assert_eq!(
            out,
            "Function <first.c>:1:1 \"outer\" \"int\"
  Compound <second.c>:4:7
"
        );
    }

    /// The artifact is what was asked for, so failing to write it is a failed
    /// run even though every diagnostic was delivered.
    #[test]
    fn an_artifact_that_could_not_be_written_is_an_error() {
        let file = TempFile::new("safec_driver_emit_closed.c", "int x;\n");
        let mut options = options(vec![file.path().to_path_buf()]);
        options.emit = EmitKind::Tokens;

        let error = run_compiler(&options, &mut Vec::new(), &mut Closed)
            .expect_err("a failed write must not come back as an outcome");

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    /// Colour is asked for through the options and has to reach the output.
    /// Every diagnostic the driver produces today is unanchored, so this covers
    /// the path that renders without pointing at source.
    #[test]
    fn the_colour_mode_reaches_the_output() {
        let mut options = options(vec![missing_path("safec_driver_colour.c")]);

        options.color = ColorMode::Never;
        let (plain, _) = rendered(&options);
        options.color = ColorMode::Always;
        let (coloured, _) = rendered(&options);

        assert!(!plain.contains('\u{1b}'), "{plain:?}");
        assert!(coloured.contains('\u{1b}'), "{coloured:?}");
    }

    /// Every file that loaded is scanned, whatever happened to the others.
    /// Gating on the run would make a user fix one file per run.
    #[test]
    fn a_bad_input_does_not_stop_the_others_being_scanned() {
        let scanned = TempFile::new("safec_driver_still_scanned.c", "int x = @;\n");
        let compiled = compile(&options(vec![
            missing_path("safec_driver_absent_first.c"),
            scanned.path().to_path_buf(),
        ]));

        let messages: Vec<_> = compiled
            .diagnostics
            .diagnostics()
            .iter()
            .map(Diagnostic::message)
            .collect();

        assert!(
            messages.iter().any(|m| m.contains("cannot read")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("unexpected character")),
            "{messages:?}"
        );
    }

    #[test]
    fn a_lexical_error_in_one_input_does_not_hide_one_in_another() {
        let first = TempFile::new("safec_driver_lex_a.c", "int a = @;\n");
        let second = TempFile::new("safec_driver_lex_b.c", "int b = `;\n");

        let compiled = compile(&options(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]));

        let unexpected = compiled
            .diagnostics
            .diagnostics()
            .iter()
            .filter(|d| d.message().contains("unexpected character"))
            .count();

        assert_eq!(unexpected, 2);
    }

    /// Until now every diagnostic the compiler produced was unanchored, so
    /// `render_all` never reached the source map at all and the driver's tests
    /// passed against an empty one. The scan is what raises the first labelled
    /// diagnostic; what is under test here is the driver's wiring, that the map
    /// it loaded is the map the renderer is given.
    #[test]
    fn a_labelled_diagnostic_reaches_the_source_map_the_driver_loaded() {
        let file = TempFile::new("safec_driver_quotes_source.c", "int x = @;\n");
        let mut report = Vec::new();

        run_compiler(
            &options(vec![file.path().to_path_buf()]),
            &mut report,
            &mut Vec::new(),
        )
        .expect("writing to a vector cannot fail");
        let report = String::from_utf8(report).expect("the renderer writes text");

        assert!(report.contains("int x = @;"), "{report}");
        assert!(report.contains("not part of any token"), "{report}");
    }
}
