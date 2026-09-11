//! What an artifact looks like, for every phase that writes one.
//!
//! `--emit safety-ir` is this module's own: [`dump_ir`] renders a
//! [`TranslationUnit`] the way a caller redirecting it would see. What an
//! artifact line says about a terminator or a projection is a fact about the
//! IR, so it lives beside the IR rather than in whoever happens to call it;
//! ADR-0011 is where that boundary is argued.
//!
//! The rest is shared with the artifacts that are not this crate's. `--emit
//! tokens` and `--emit ast` are written in `safec`'s driver and print the same
//! line shape, so [`dump_node`], [`dump_line`], [`DEEPEST_INDENT`] and
//! [`quoted`] are here for all three: one answer to what a level of indent
//! looks like, and one to what happens past the depth where it stops helping.
//!
//! [`shown`] is here for the same reason and one more. Every line of every
//! artifact begins with a file's name, and a name is content: a `.c` file
//! called `evil\u{1b}[31m.c` must not be able to colour somebody's terminal
//! from inside a dump. The renderer asks the same question of a message, so
//! the answer is one function and this is the crate both sides can reach.

use std::borrow::Cow;
use std::fmt::Write as _;

use crate::ir::{
    Element, LocalId, Operand, Place, Projection, Rvalue, Terminator, TranslationUnit, Ty, TyId,
};
use crate::source::{SourceMap, Span};

/// Text the compiler is echoing back, made safe to print.
///
/// A file name, an identifier or a string literal out of the source is content
/// rather than something this compiler wrote.
/// A terminal reads an escape sequence in it as an instruction: a colour, a
/// cursor move, or clearing the line the diagnostic above it is on. Colour is
/// something the renderer adds and not something content may smuggle in, so a
/// control character arriving from content is shown rather than obeyed, under
/// every colour mode rather than only under `--color never`.
///
/// Newline and tab are left alone. They are laid out rather than acted on, and
/// a message that runs to two lines is ordinary.
///
/// This is for text the compiler writes. The text of a file, which `ariadne`
/// echoes rather than this compiler, goes through `render::echoed` in `safec`
/// instead, for a reason that rules this function out there: it turns one
/// character into several, and a label's span is a byte offset into the very
/// text being echoed.
///
/// Public, and in this crate, because three artifacts and every diagnostic ask
/// one question. Every line of `--emit tokens`, `--emit ast` and `--emit
/// safety-ir` begins with a file's name, and a name is content the same way a
/// file's text is. Two of those artifacts are written in `safec` and reach
/// here rather than keeping a second answer.
pub fn shown(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_obeyed) {
        return Cow::Borrowed(text);
    }

    let mut safe = String::with_capacity(text.len());
    for ch in text.chars() {
        if is_obeyed(ch) {
            safe.extend(ch.escape_debug());
        } else {
            safe.push(ch);
        }
    }
    Cow::Owned(safe)
}

/// Whether a terminal would act on this rather than print it.
///
/// Public because `render::echoed` in `safec` is the other half of the pair
/// [`shown`] belongs to, and the two must agree about which characters are the
/// dangerous ones. One predicate rather than two that could drift.
pub fn is_obeyed(ch: char) -> bool {
    ch.is_control() && ch != '\n' && ch != '\t'
}

/// The source text a span covers, resolved against the file the span names.
///
/// Not against whichever file the driver's loop is on. One tree holds spans
/// from one file today and will hold several the moment `#include` lands, and a
/// span resolved against the wrong file prints another file's text at another
/// file's line, or panics when that file is shorter. `render.rs` looks the file
/// up per label for this reason, and ADR-0003 is where it is argued.
///
/// **Quoted as in quoted from, not as in quote marks: this hands back the
/// file's own bytes and escapes nothing.** Write it with `{:?}` and never with
/// `{}`. The text is content, and a `.c` file whose identifier held an escape
/// sequence would otherwise clear the terminal of whoever compiled it, which is
/// RK-002 in the review knowledge bank and has happened here once. Every caller
/// writes `{:?}` today; this says so where a new one is looking, rather than in
/// [`dump_node`], where the same rule was written when there was one file of
/// callers to keep to it.
pub fn quoted(sources: &SourceMap, span: Span) -> &str {
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
pub const DEEPEST_INDENT: usize = 32;

/// The part every line shares: indent, kind, and where it is.
///
/// Source text is written with `{:?}` by the callers that write any, for the
/// reason RK-002 records: a `.c` file's own bytes reaching a stream are
/// content, and one holding an escape sequence must not be able to clear the
/// terminal of whoever compiled it.
///
/// **The indent is written rather than passed to `write!` as a width.** Rust's
/// format width is a `u16` and `depth` is bounded by nothing: `driver::dump_expr`
/// says why, and the cost of not knowing it was that a tree 32768 levels deep
/// panicked inside `write!`, before `expect` could see a `Result`, for exit 101
/// with nothing on either stream. 32768 is where it starts: two spaces a level
/// is a width of 65536, and 65535 is the largest a `u16` holds. The smallest
/// `.c` file reaching it is 96 KB of `i[i][i]...`, 32764 subscripts, which
/// `clang` parses.
pub fn dump_node(sources: &SourceMap, kind: &str, span: Span, depth: usize, out: &mut String) {
    let file = sources.file(span.file());
    let at = file.line_col(span.start());

    dump_line(kind, depth, out);
    // A name is content: it comes from a command line today and from a file
    // once `#include` lands, and every line of every artifact begins with one.
    // RK-002 in the review knowledge bank is the entry, and `shown` is the
    // answer the renderer already gives to the same question.
    write!(
        out,
        " {}:{}:{}",
        shown(&file.name().to_string()),
        at.line,
        at.column
    )
    .expect("writing to a string cannot fail");
}

/// A line that names a kind and nothing about where it is.
///
/// For the parts of the Safety IR that have no span of their own: a local, a
/// block, and every terminator but a call. The indent lives here rather than in
/// each caller, so that the two line shapes cannot disagree about what a level
/// looks like or about what happens past [`DEEPEST_INDENT`].
pub fn dump_line(kind: &str, depth: usize, out: &mut String) {
    for _ in 0..depth.min(DEEPEST_INDENT) {
        out.push_str("  ");
    }
    if depth > DEEPEST_INDENT {
        write!(out, "+{} ", depth - DEEPEST_INDENT).expect("writing to a string cannot fail");
    }

    out.push_str(kind);
}

/// The IR, as a caller redirecting it would see.
///
/// The same line shape as `--emit ast`: a kind, where it is, and whatever that
/// line alone carries, two spaces of indent per level. What differs is that a
/// line carries a location only where the IR holds one. A span reaches this
/// printer in three places, a function's name, an operation's origin and a
/// call's, and a local, a block and every other terminator have none. Giving
/// one the span of the function it sits in would be a claim that it is written
/// there, and `--emit ast`'s whole discipline is that a line says where a thing
/// actually is.
///
/// **No ids.** A local is `_0`, a block is `bb0`, and a callee is named by its
/// name rather than by its [`crate::ir::FuncId`], which is an index into a table this
/// artifact does not show.
pub fn dump_ir(sources: &SourceMap, unit: &TranslationUnit, out: &mut String) {
    // First, because the rest means something else for a different machine:
    // what `int` is worth and whether `char` carries a sign are the target's to
    // say, and an artifact that hid which one it was built for would be two
    // different programs printed identically. See ADR-0013.
    dump_line("Target", 0, out);
    write!(out, " {:?}", unit.target().triple()).expect("writing to a string cannot fail");
    out.push('\n');

    for id in unit.functions() {
        let function = unit.function(id);
        dump_node(sources, "Function", function.name, 0, out);
        write!(out, " {:?}", quoted(sources, function.name))
            .expect("writing to a string cannot fail");
        // A declaration and a definition with no blocks would read alike, and
        // one of them is a function whose body this compiler never saw.
        if !function.is_defined() {
            out.push_str(" declared");
        }
        out.push('\n');

        for local in function.locals() {
            dump_line("Local", 1, out);
            write!(
                out,
                " {:?} {:?}",
                name_of(local),
                spell_ty(unit, function.local(local))
            )
            .expect("writing to a string cannot fail");
            if local == function.return_place() {
                out.push_str(" return");
            } else if function.parameters().any(|parameter| parameter == local) {
                out.push_str(" parameter");
            }
            out.push('\n');
        }

        if !function.is_defined() {
            continue;
        }

        // A block's position is its id: `blocks` hands them back in the order
        // their ids were handed out, and an id is where a block sits in the
        // function. That is what lets an edge, which is printed from an id, and
        // a block, which is printed from a position, name the same thing.
        let entry = function.entry();
        for (index, block) in function.blocks().enumerate() {
            dump_line("Block", 1, out);
            write!(out, " {:?}", block_name(index)).expect("writing to a string cannot fail");
            // Which block control enters is a fact about the function that the
            // artifact would otherwise only imply by printing it first.
            if index == entry.index() {
                out.push_str(" entry");
            }
            out.push('\n');

            for element in &block.elements {
                // Written out rather than `..`, so that a field added to a
                // storage marker is `error[E0027]` here and not something the
                // artifact silently stops showing. RK-018 is the entry, and
                // `Terminator::successors` is where the rule was written.
                match element {
                    Element::Assign(operation) => {
                        dump_node(sources, element.name(), operation.origin.span(), 2, out);
                        write!(out, " {}", operation.origin.name())
                            .expect("writing to a string cannot fail");
                        out.push('\n');

                        dump_place(sources, "Destination", &operation.place, 3, out);
                        dump_rvalue(sources, &operation.value, 3, out);
                    }
                    Element::StorageLive { local, origin }
                    | Element::StorageDead { local, origin } => {
                        // The local goes on the kind's own line, the way a
                        // block's name does, because a marker is one fact and
                        // descending to read it would cost a line to say what
                        // fits here.
                        dump_node(sources, element.name(), origin.span(), 2, out);
                        // The origin's kind, the same word an `Operation` line
                        // carries. Nobody writes a storage marker, so every one
                        // says `generated`; printing it is what makes a marker
                        // that claimed otherwise show up in a corpus case
                        // rather than only in the source.
                        write!(out, " {} {:?}", origin.name(), name_of(*local))
                            .expect("writing to a string cannot fail");
                        out.push('\n');
                    }
                }
            }

            dump_terminator(sources, unit, &block.terminator, 2, out);
        }
    }
}

/// A local's name in the artifact.
fn name_of(local: LocalId) -> String {
    format!("_{}", local.index())
}

/// A block's name in the artifact.
fn block_name(index: usize) -> String {
    format!("bb{index}")
}

/// A type, spelled the way `ast::spell_type` spells the same one.
///
/// Two spellings of one type is a second thing to keep in agreement, and a
/// reader moving between `--emit ast` and `--emit safety-ir` reads both.
fn spell_ty(unit: &TranslationUnit, ty: TyId) -> String {
    let mut pointers = 0usize;
    let mut current = ty;

    let base = loop {
        match unit.ty(current) {
            Ty::Int => break "int",
            Ty::Char => break "char",
            Ty::Void => break "void",
            Ty::Pointer(pointee) => {
                pointers += 1;
                current = pointee;
            }
        }
    };

    if pointers == 0 {
        base.to_owned()
    } else {
        format!("{base} {}", "*".repeat(pointers))
    }
}

/// A place: the local it starts at, and one line per step away from it.
///
/// A line per projection rather than a spelling like `(*_1)[_2]`, because the
/// artifact is read by kind: somebody looking for every dereference in a
/// program greps `Deref`.
fn dump_place(sources: &SourceMap, kind: &str, place: &Place, depth: usize, out: &mut String) {
    dump_line(kind, depth, out);
    write!(out, " {:?}", name_of(place.local)).expect("writing to a string cannot fail");
    out.push('\n');

    for (step, projection) in place.projection.iter().enumerate() {
        dump_line(projection.name(), depth + 1 + step, out);
        out.push('\n');

        if let Projection::Index(index) = projection {
            dump_operand(sources, index, depth + 2 + step, out);
        }
    }
}

/// An operand: what it is, and the place it reads where it reads one.
fn dump_operand(sources: &SourceMap, operand: &Operand, depth: usize, out: &mut String) {
    match operand {
        Operand::Copy(place) => dump_place(sources, operand.name(), place, depth, out),
        Operand::Constant(value) => {
            dump_line(operand.name(), depth, out);
            write!(out, " {value}").expect("writing to a string cannot fail");
            out.push('\n');
        }
    }
}

/// What an operation computes, and what it reads to compute it.
fn dump_rvalue(sources: &SourceMap, value: &Rvalue, depth: usize, out: &mut String) {
    dump_line(value.name(), depth, out);

    match value {
        Rvalue::Use(operand) => {
            out.push('\n');
            dump_operand(sources, operand, depth + 1, out);
        }
        Rvalue::Unary { op, operand } => {
            write!(out, " {:?}", op.name()).expect("writing to a string cannot fail");
            out.push('\n');
            dump_operand(sources, operand, depth + 1, out);
        }
        Rvalue::Binary { op, lhs, rhs } => {
            write!(out, " {:?}", op.name()).expect("writing to a string cannot fail");
            out.push('\n');
            dump_operand(sources, lhs, depth + 1, out);
            dump_operand(sources, rhs, depth + 1, out);
        }
        Rvalue::Address(place) => {
            out.push('\n');
            dump_place(sources, "Place", place, depth + 1, out);
        }
    }
}

/// How a block ends, and where control goes from it.
///
/// The blocks it can reach are on the kind's own line, because they are the
/// edges of the graph and a reader following one should not have to descend to
/// find where it goes.
fn dump_terminator(
    sources: &SourceMap,
    unit: &TranslationUnit,
    terminator: &Terminator,
    depth: usize,
    out: &mut String,
) {
    match terminator {
        Terminator::Call { origin, .. } => {
            dump_node(sources, terminator.name(), origin.span(), depth, out);
        }
        Terminator::Goto(_)
        | Terminator::Branch { .. }
        | Terminator::Return
        | Terminator::Abnormal { .. } => dump_line(terminator.name(), depth, out),
    }

    match terminator {
        Terminator::Goto(to) => {
            write!(out, " {:?}", block_name(to.index())).expect("writing to a string cannot fail");
            out.push('\n');
        }
        Terminator::Branch {
            condition,
            then,
            otherwise,
        } => {
            write!(
                out,
                " {:?} {:?}",
                block_name(then.index()),
                block_name(otherwise.index())
            )
            .expect("writing to a string cannot fail");
            out.push('\n');
            dump_operand(sources, condition, depth + 1, out);
        }
        Terminator::Call {
            callee,
            arguments,
            destination,
            then,
            origin: _,
        } => {
            write!(
                out,
                " {:?} {:?}",
                quoted(sources, unit.function(*callee).name),
                block_name(then.index())
            )
            .expect("writing to a string cannot fail");
            out.push('\n');

            if let Some(destination) = destination {
                dump_place(sources, "Destination", destination, depth + 1, out);
            }
            // Whatever else is under a call is what it passes, in order.
            for argument in arguments {
                dump_operand(sources, argument, depth + 1, out);
            }
        }
        Terminator::Return => out.push('\n'),
        Terminator::Abnormal { to } => {
            write!(out, " {:?}", block_name(to.index())).expect("writing to a string cannot fail");
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::Target;

    /// A target, for a test that is not about the machine.
    ///
    /// Named rather than defaulted, because `TranslationUnit::new` takes one on
    /// purpose: ADR-0013 says a unit nobody said the target of is one whose
    /// `int` has no width. A test that *is* about the machine names its own.
    fn a_target() -> Target {
        Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple")
    }
    use crate::ir::{Block, Function, Operation, Origin};

    /// Newline and tab survive, and every other control character does not.
    ///
    /// The two exceptions are the whole difference between laying text out and
    /// obeying it: a diagnostic that runs to two lines is ordinary, and a
    /// `shown` that escaped its own newlines would print one line saying
    /// `first\nsecond`. Everything else in the class is an instruction to a
    /// terminal.
    ///
    /// The newline half was already held, by
    /// `render::a_newline_in_a_message_is_left_alone` and by twenty-five
    /// corpus cases whose `.stderr` runs to more than one line. **The tab half
    /// was held by nothing**: dropping `&& ch != '\t'` alone passed the whole
    /// workspace, because no message and no artifact in the suite contains a
    /// tab. That is the mutation this test exists for.
    ///
    /// It is worth asserting here rather than reading, because the pair this
    /// predicate serves now sits either side of a crate boundary, `shown` here
    /// and `render::echoed` in `safec`, and the two can no longer be changed
    /// in one edit.
    #[test]
    fn only_newline_and_tab_are_laid_out_rather_than_obeyed() {
        assert_eq!(shown("first\nsecond\tthird"), "first\nsecond\tthird");

        // One from each of the ranges C0, C1 and the delete character, so that
        // an exception added to any of them is an exception this catches.
        for ch in ['\u{1b}', '\r', '\u{0}', '\u{7f}', '\u{85}', '\u{9b}'] {
            assert!(is_obeyed(ch), "{ch:?} reaches a terminal unescaped");
            let text = format!("a{ch}b");
            assert!(
                !shown(&text).contains(ch),
                "{ch:?} survived `shown`: {:?}",
                shown(&text)
            );
        }
    }

    /// A file's name is content, and every artifact line begins with one.
    ///
    /// A name is not something this compiler wrote: it comes from a command
    /// line or, once `#include` lands, from a file. RK-002 in the review
    /// knowledge bank is the entry, and the case it records is a `.c` file that
    /// cleared the terminal of whoever compiled it. A name can do the same, and
    /// a name that reorders the line it is on is the shape somebody would use
    /// to make an artifact say something it does not.
    ///
    /// Mutation: write the name with `{}` rather than through `shown`. The
    /// escape reaches the artifact and this fails.
    #[test]
    fn a_file_name_is_escaped_wherever_an_artifact_prints_one() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("evil\u{1b}[31m.c", "int f(void) { return 0; }\n");
        let at = Span::new(file, 4, 5);

        let mut unit = TranslationUnit::new(a_target());
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(at, int, []);
        function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        unit.push_function(function);

        let mut ir = String::new();
        dump_ir(&sources, &unit, &mut ir);
        assert!(!ir.contains('\u{1b}'), "{ir:?}");
        assert!(
            ir.contains("\\u{1b}"),
            "the escape is shown rather than obeyed: {ir:?}"
        );

        // The tree and the tokens print the same name through the same
        // function, so they answer the same question here.
        let mut tree = String::new();
        dump_node(&sources, "Node", at, 0, &mut tree);
        assert!(!tree.contains('\u{1b}'), "{tree:?}");
    }

    /// An operation nobody wrote says so, and one somebody wrote says that.
    ///
    /// The IR can tell them apart and the tree cannot, which is the whole
    /// reason `Origin` exists; `docs/roadmap.md` asks for "a location to blame
    /// and no source text". Built by hand because nothing in the lowering
    /// produces a generated *operation*: what ends a scope is generated and is
    /// a storage marker rather than an operation, so this shape still has no
    /// producer. A C++ destructor is the one that will.
    ///
    /// Mutation: print `written` whatever the origin. This fails on the second
    /// line, and with it goes the only thing the artifact says that `--emit
    /// ast` could not.
    #[test]
    fn an_operation_that_nobody_wrote_says_so() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int f(void) { int x; }\n");
        let at = Span::new(file, 14, 20);

        let mut unit = TranslationUnit::new(a_target());
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(at, int, []);
        let local = function.push_local(int);
        function.push_block(Block {
            elements: vec![
                Element::Assign(Operation {
                    place: Place::local(local),
                    value: Rvalue::Use(Operand::Constant(1)),
                    origin: Origin::Written(at),
                }),
                Element::Assign(Operation {
                    place: Place::local(local),
                    value: Rvalue::Use(Operand::Constant(0)),
                    origin: Origin::Generated(at),
                }),
            ],
            terminator: Terminator::Return,
        });
        unit.push_function(function);

        let mut out = String::new();
        dump_ir(&sources, &unit, &mut out);

        let origins: Vec<&str> = out
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("Operation "))
            .filter_map(|line| line.split_whitespace().nth(1))
            .collect();
        assert_eq!(origins, ["written", "generated"], "{out}");
    }

    /// The shapes the printer has to answer for and no C program makes.
    ///
    /// An edge no statement produced is one, which ADR-0010 put in the IR
    /// before anything built one. A call that writes its result nowhere is the
    /// other, and `free(p);` will be the first of those the day a program can
    /// declare `free`.
    ///
    /// Mutation: drop the `Abnormal` arm from `dump_terminator`. It stops
    /// compiling, because the match over a terminator is written out. Mutation:
    /// print a `Destination` line whether or not there is one. This fails on
    /// the call's children.
    #[test]
    fn an_abnormal_edge_and_a_call_that_writes_nowhere_are_printed() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "void f(void) { g(); }\n");
        let at = Span::new(file, 15, 18);

        let mut unit = TranslationUnit::new(a_target());
        let void = unit.push_type(Ty::Void);
        let callee = unit.push_function(Function::declaration(at, void, []));

        let mut function = Function::new(at, void, []);
        let handler = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        let after = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Abnormal { to: handler },
        });
        function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Call {
                callee,
                arguments: Vec::new(),
                destination: None,
                then: after,
                origin: Origin::Written(at),
            },
        });
        unit.push_function(function);

        let mut out = String::new();
        dump_ir(&sources, &unit, &mut out);
        let lines: Vec<&str> = out.lines().map(str::trim_start).collect();

        assert!(
            lines.contains(&"Abnormal \"bb0\""),
            "an abnormal edge names where it goes: {out}"
        );
        assert!(lines.iter().any(|line| line.starts_with("Call ")), "{out}");
        assert!(
            !lines.iter().any(|line| line.starts_with("Destination")),
            "a call with nowhere to write says nothing about a destination: {out}"
        );
    }

    /// A declaration is a function with no body, and the artifact says which.
    ///
    /// A definition always has a block, so a `Function` line with no `Block`
    /// under it is either a declaration or an artifact that lost one.
    ///
    /// Mutation: print `declared` for every function, or for none. This fails.
    #[test]
    fn a_declaration_is_marked_and_a_definition_is_not() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int g(void);\nint f(void) { return 0; }\n");
        let there = Span::new(file, 4, 5);
        let here = Span::new(file, 17, 18);

        let mut unit = TranslationUnit::new(a_target());
        let int = unit.push_type(Ty::Int);
        unit.push_function(Function::declaration(there, int, []));

        let mut defined = Function::new(here, int, []);
        defined.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        unit.push_function(defined);

        let mut out = String::new();
        dump_ir(&sources, &unit, &mut out);

        let functions: Vec<&str> = out
            .lines()
            .filter(|line| line.starts_with("Function "))
            .collect();
        assert_eq!(functions.len(), 2, "{out}");
        assert!(functions[0].ends_with("\"g\" declared"), "{out}");
        assert!(functions[1].ends_with("\"f\""), "{out}");
    }

    /// A place is its local and one line per step away from it.
    ///
    /// `p` and `*p` are two places and an analysis reading this artifact has to
    /// see which one an operation touched, so a projection is a line rather
    /// than punctuation inside one.
    ///
    /// Mutation: print a place's projections on its own line, or drop them.
    /// This fails.
    #[test]
    fn a_projection_is_a_line_of_its_own() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int f(int *p) { return p[0]; }\n");
        let at = Span::new(file, 23, 27);

        let mut unit = TranslationUnit::new(a_target());
        let int = unit.push_type(Ty::Int);
        let pointer = unit.push_type(Ty::Pointer(int));

        let mut function = Function::new(at, int, [pointer]);
        let p = function.parameters().next().expect("one parameter");
        function.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(function.return_place()),
                value: Rvalue::Use(Operand::Copy(Place {
                    local: p,
                    projection: vec![Projection::Index(Operand::Constant(0))],
                })),
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        unit.push_function(function);

        let mut out = String::new();
        dump_ir(&sources, &unit, &mut out);
        let lines: Vec<&str> = out.lines().map(str::trim_start).collect();

        let index = lines
            .iter()
            .position(|line| *line == "Index")
            .unwrap_or_else(|| panic!("{out}"));
        assert_eq!(lines[index - 1], "Copy \"_1\"", "{out}");
        assert_eq!(lines[index + 1], "Constant 0", "{out}");
    }
}
