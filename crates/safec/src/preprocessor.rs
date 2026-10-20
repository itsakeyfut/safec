//! Directives, read between the lexer and the parser.
//!
//! What it reads is `#include "file"`, whose file's tokens, themselves
//! preprocessed, take the directive's place; object-like `#define` and
//! `#undef`, whose macros' names are replaced by their replacement lists,
//! rescanned (C17 6.10.3); and `#ifdef`, `#ifndef`, `#else` and `#endif`,
//! which choose the groups that are compiled (6.10.1). Every other directive
//! is refused as `SC0104`, `#include <file>` among them, which needs a search
//! path and a system header this compiler cannot read yet anyway, and so are
//! function-like macros, `#`, `##`, `#if` and `#elif`.
//!
//! Each inclusion and each expansion is a file of its own in the source map,
//! so a diagnostic in a header names the header, one in an expansion is shown
//! at its `#define` and its use, and two of either never share a span. See
//! ADR-0054. The translation unit is preprocessed whole before it is
//! parsed, for the reason that record gives.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::driver::{Access, reason};
use crate::lexer::{UNSUPPORTED_SPLICE, lex};
use crate::token::{Punct, Token, TokenKind};
use safec_ir::source::{Cut, Edge, FileId, FileName, SourceMap, Span};

// `SC01xx` with the lexer's, because the topic is the same: what a line of the
// source is before the grammar reads it. `SC0104` was the lexer's until the
// preprocessor took the line over, and kept its number when it moved.
const UNSUPPORTED_DIRECTIVE: Code = Code::new("SC0104");
/// A file an `#include` names that cannot be read.
const UNREADABLE_INCLUSION: Code = Code::new("SC0109");
/// An `#include` of a file that is already being read.
const RECURSIVE_INCLUSION: Code = Code::new("SC0110");
/// An `#include` nested deeper than [`MAX_DEPTH`].
const TOO_DEEP: Code = Code::new("SC0111");
/// An `#include` past [`MAX_INCLUSIONS`].
const TOO_MANY: Code = Code::new("SC0112");
/// A macro expanded past [`MAX_EXPANSIONS`].
const TOO_MANY_EXPANSIONS: Code = Code::new("SC0113");
/// A macro defined a second time with a different replacement list.
const REDEFINED: Code = Code::new("SC0114");
/// A conditional written wrong: one not closed in its file, an `#else`,
/// `#elif` or `#endif` with none open, a second `#else`, or an `#ifdef` with
/// no name. One code, because the fix is the same each time: a directive to
/// add, move or remove.
const UNBALANCED: Code = Code::new("SC0115");
/// Tokens after a conditional directive's name, or after `#else` or `#endif`.
const EXTRA_TOKENS: Code = Code::new("SC0116");

/// How many files may be being read at once, the input among them.
///
/// Not a bound on the native stack, which [`read_includes`] does not recurse on. It
/// bounds a chain of different files, each including the next, which nothing
/// else ends. C17 5.2.4.1 asks for 15 levels, so this leaves a program written
/// for any compiler room, and a refusal past it says where.
const MAX_DEPTH: usize = 200;

/// How many inclusions one input may make in all.
///
/// [`MAX_DEPTH`] bounds a chain, not a tree: a header that includes the next
/// one twice, fifteen deep, is 65,536 inclusions, each a file of its own
/// (ADR-0054), and every two levels more took four times as long, measured,
/// with nothing printed until it ended. An include guard stops that in a real
/// header, since a second reading skips the `#include`s inside it, and this
/// bounds a header that has none. Found by review.
const MAX_INCLUSIONS: usize = 4096;

/// How many expansions one use of a macro, written outside any expansion,
/// may make with everything it expands to.
///
/// The rule that a macro is not expanded inside its own expansion ends every
/// chain, so this bounds a tree: a macro whose body uses the next one twice,
/// sixteen deep, is 131,071 expansions, each a file of its own. Past it the
/// next is refused, once. Counted per use and not per input, because a file
/// that uses a macro on each of 65,537 lines asks nothing unusual of C, and
/// counting them all refused it. Found by review.
const MAX_EXPANSIONS: usize = 65536;

/// How many expansions one input may make in all.
///
/// [`MAX_EXPANSIONS`] bounds one use's tree, and a file can hold many uses:
/// with the count starting again at each, forty uses of a tree just under the
/// bound took over 600 MB, each expansion a file of its own, and printed
/// nothing before it was stopped. Found by review. This sits above the most a
/// file that uses a macro once on each of its lines needs at the bound, and
/// past it the next expansion is refused, once.
const MAX_EXPANSIONS_IN_ALL: usize = 2 * MAX_EXPANSIONS;

/// The files being read, the macros defined, and how many inclusions and
/// expansions there have been so far.
struct Reading {
    /// Outermost first, each by its canonical path, so a cycle is found
    /// whatever spelling reached it.
    open: Vec<(PathBuf, FileId)>,
    inclusions: usize,
    /// Every object-like macro defined so far, by name.
    macros: HashMap<String, Macro>,
    expansions: usize,
    /// Every expansion this input has made.
    expansions_in_all: usize,
    /// How many expansions of each name are open, and of all names: what the
    /// rule that a macro is not expanded inside itself asks of, kept as a
    /// count rather than found by walking the frames, which made a chain of
    /// distinct macros cost the square of its depth. Found by review.
    open_macros: HashMap<String, usize>,
    open_expansions: usize,
}

/// An object-like macro.
struct Macro {
    /// Its replacement list, in its `#define`: what each expansion holds.
    spelled: Span,
    /// The `#define`, for the note on a redefinition.
    defined: Span,
    /// Its replacement list's tokens, each with whether whitespace came
    /// before it: what two definitions are compared by (C17 6.10.3 p1).
    tokens: Vec<(String, bool)>,
}

/// How a refusal of `cut` says what was cut: the thing, as the reader wrote
/// it, the label at the cut, and where to say it was included, which is only
/// for an inclusion, since the renderer says where an expansion came from by
/// itself.
pub(crate) fn cut_words(sources: &SourceMap, cut: Cut) -> (String, String, Option<Span>) {
    let edge = match cut.edge {
        Edge::Start => "begins",
        Edge::End => "ends",
    };
    match sources.macro_name(cut.instance) {
        Some(name) => (
            format!("this use of `{name}`"),
            format!("{edge} partway through `{name}`'s expansion"),
            None,
        ),
        None => (
            "this `#include`".to_owned(),
            format!("{edge} partway through an included file"),
            sources.at(cut.instance),
        ),
    }
}

/// The tokens the parser reads: one input's, every `#include` spliced in, and
/// no directive among them.
///
/// Only [`preprocess`] builds one, since the field is private, and
/// [`crate::parser::parse`] takes nothing else. So a caller that lexes and
/// parses without preprocessing does not compile, where it used to hand the
/// parser every directive to be dropped without a word: seven test helpers
/// did, and after `#ifdef` that would compile both of its groups. No directive
/// is in one because `read_includes` reads each it meets, splicing an
/// `#include`, recording a macro or reporting what it does not read, and
/// pushes none of them.
///
/// Mutation: drop the `preprocess` call from any caller of `parse`. It is
/// `E0308`; building one by hand outside this module is `E0603`.
#[derive(Debug)]
pub struct Preprocessed(Vec<Token>);

impl Preprocessed {
    /// The tokens, in order, ending in the input's end of file where the
    /// scan had one.
    pub fn tokens(&self) -> &[Token] {
        &self.0
    }
}

/// Splice every `#include "file"` in `tokens`, the scan of `file`, record
/// every `#define` and `#undef` and expand what they define, and report every
/// other directive.
///
/// The end-of-file token is the input's own: an included file's is dropped,
/// since the parser reads the first one it meets as the end.
pub fn preprocess(
    file: FileId,
    tokens: &[Token],
    sources: &mut SourceMap,
    diagnostics: &mut DiagnosticSink,
) -> Preprocessed {
    let mut reading = Reading {
        open: Vec::new(),
        inclusions: 0,
        macros: HashMap::new(),
        expansions: 0,
        expansions_in_all: 0,
        open_macros: HashMap::new(),
        open_expansions: 0,
    };
    if let Some(path) = path_of(sources.file(file).name()) {
        reading.open.push((canonical(path), file));
    }

    let mut spliced = Vec::with_capacity(tokens.len());
    read_includes(tokens, sources, &mut reading, diagnostics, &mut spliced);
    spliced.extend(tokens.iter().copied().filter(|token| token.is_eof()));
    Preprocessed(spliced)
}

/// Append `tokens` to `spliced`, an `#include "file"` replaced by that file's
/// and a macro's name by its expansion, rescanned, and no end of file.
///
/// One frame per file being read, the input's at the bottom, each holding its
/// tokens and how far they have been read; an inclusion and an expansion are
/// each a frame. A stack of its own rather than the
/// native one: recursing once per file overflowed the main thread's stack on
/// a chain of 160 headers, well inside [`MAX_DEPTH`], and where it overflowed
/// moved whenever a frame grew. Found by review. The reading ends because a
/// file being read is never read again, a chain of different files stops at
/// [`MAX_DEPTH`], and a tree of them at [`MAX_INCLUSIONS`].
fn read_includes(
    tokens: &[Token],
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
    spliced: &mut Vec<Token>,
) {
    let mut frames = vec![Frame {
        tokens: tokens.to_vec(),
        cursor: 0,
        instance: None,
        extent: None,
        conditionals: Vec::new(),
        skipped_depth: 0,
    }];
    while let Some(frame) = frames.last_mut() {
        let next = frame.tokens.get(frame.cursor).copied();
        frame.cursor += 1;
        let Some(token) = next else {
            let Some(frame) = frames.pop() else { break };
            for open in &frame.conditionals {
                diagnostics.report(
                    Diagnostic::error(format!("`{}` is not closed in this file", open.directive))
                        .with_code(UNBALANCED)
                        .with_label(Label::primary(open.opened, "opened here"))
                        .with_note("a conditional ends with an `#endif` in the file it begins in"),
                );
            }
            if let Some(instance) = frame.instance {
                match &instance {
                    Instance::Inclusion { .. } => {
                        reading.open.pop();
                    }
                    Instance::Expansion { name, .. } => {
                        reading.open_expansions -= 1;
                        if let Some(open) = reading.open_macros.get_mut(name) {
                            *open -= 1;
                        }
                    }
                }
                if let Some((first, last)) = frame.extent {
                    sources.set_extent(instance.file(), first, last);
                    // What the frame below was handed in its place, now that
                    // it handed over anything at all.
                    emitted(&mut frames, instance.used());
                }
            }
            continue;
        };
        let skipping = frames.last().is_some_and(Frame::skipping);
        match token.kind {
            TokenKind::Eof => {}
            TokenKind::Directive => {
                // Owned, because reading the file it names adds to the map.
                let raw = sources.snippet(token.span).to_owned();
                // A splice in a comment is a comment going on past the line:
                // C splices lines before it reads comments (C17 5.1.1.2 p1,
                // phases 2 and 3), so `// \` carries the comment, and not
                // code, onto the line below. Asked of the line before its
                // comments are made spaces, which takes the splice with them
                // and read `#include "a.h" // \` as an include. Found by
                // review, after it had been blessed.
                let blank =
                    blanked(&raw).filter(|text| self::spliced(text) || !self::spliced(&raw));
                let Some(text) = blank else {
                    let text = raw;
                    diagnostics.report(
                        refused(
                            &text,
                            token.span,
                            UNSUPPORTED_DIRECTIVE,
                            "a comment that goes on past a directive's line is not read yet",
                        )
                        .with_note(
                            "C makes the comment one space, so the directive goes on after \
                             it ends, and the lines inside it are not code",
                        ),
                    );
                    continue;
                };
                let form = conditional_form(&text);
                // A splice joins the line below to this one before any
                // directive is read (C17 5.1.1.2 p1, phase 2), and the lexer
                // read that line on its own, as a directive where it begins
                // with `#`. So it is refused wherever a directive is read at
                // all, a skipped group's included: there `#define X \` above
                // `#else` ended a group C does not end, and the code after it
                // was compiled where C has none. Found by review.
                if self::spliced(&text) && (skipping || form.is_some()) {
                    let message = if form.is_some() {
                        "a line splice or a trigraph in a conditional directive is not read yet"
                    } else {
                        "a line splice or a trigraph in a directive in a skipped group is not \
                         read yet"
                    };
                    diagnostics.report(
                        refused(&text, token.span, UNSUPPORTED_SPLICE, message).with_note(
                            "C can join the line below to this one, and it would be read on \
                             its own instead",
                        ),
                    );
                }
                if let Some(form) = form {
                    if let Some(frame) = frames.last_mut() {
                        conditional(form, &text, token.span, frame, reading, diagnostics);
                    }
                    continue;
                }
                // A group that is skipped has its directives read only as far
                // as their names, to count the conditionals in it, and the
                // rest ignored (C17 6.10.1 p6): `#include <stdio.h>` or a
                // function-like `#define` there is not reported.
                if skipping {
                    continue;
                }
                match include_form(&text) {
                    IncludeForm::Quoted { name, quote } => {
                        let quoted = Span::new(
                            token.span.file(),
                            token.span.start() + quote,
                            token.span.start() + quote + name.len() as u32 + 2,
                        );
                        if let Some((file, included)) =
                            include(token.span, quoted, name, sources, reading, diagnostics)
                        {
                            frames.push(Frame {
                                tokens: included,
                                cursor: 0,
                                instance: Some(Instance::Inclusion {
                                    file,
                                    directive: token.span,
                                }),
                                extent: None,
                                conditionals: Vec::new(),
                                skipped_depth: 0,
                            });
                        }
                    }
                    IncludeForm::Spliced => diagnostics.report(
                        refused(
                            &text,
                            token.span,
                            UNSUPPORTED_SPLICE,
                            "a line splice or a trigraph in an `#include` is not read yet",
                        )
                        .with_note(
                            "C can join the line below to this one, and it would be \
                             compiled as code instead",
                        ),
                    ),
                    IncludeForm::Angled => diagnostics.report(
                        refused(
                            &text,
                            token.span,
                            UNSUPPORTED_DIRECTIVE,
                            "`#include <...>` is not read yet",
                        )
                        .with_note("only `#include \"...\"` is read yet"),
                    ),
                    IncludeForm::Unread => diagnostics.report(
                        refused(
                            &text,
                            token.span,
                            UNSUPPORTED_DIRECTIVE,
                            "this `#include` is not read yet",
                        )
                        .with_note(
                            "only `#include \"...\"` is read yet, with nothing but a comment \
                             after it on its line",
                        ),
                    ),
                    IncludeForm::Other => match macro_form(&text) {
                        MacroForm::Define {
                            name,
                            name_at,
                            body,
                        } => define(
                            &text,
                            token.span,
                            name,
                            name_at,
                            body,
                            sources,
                            reading,
                            diagnostics,
                        ),
                        MacroForm::Undef { name } => {
                            reading.macros.remove(name);
                        }
                        MacroForm::Unread { directive } => diagnostics.report(
                            refused(
                                &text,
                                token.span,
                                UNSUPPORTED_DIRECTIVE,
                                &format!("this `{directive}` is not read yet"),
                            )
                            .with_note(if directive == "#undef" {
                                "an `#undef` is read when its name is ASCII letters, digits \
                                 and `_`, with nothing after it"
                            } else {
                                "a `#define` is read when its name is ASCII letters, digits \
                                 and `_`, followed by a space, `(` or the end of the line"
                            }),
                        ),
                        MacroForm::Defined => diagnostics.report(
                            refused(
                                &text,
                                token.span,
                                UNSUPPORTED_DIRECTIVE,
                                "`defined` is not read as a macro's name",
                            )
                            .with_note(
                                "C17 6.10.8 p2 says it shall not be the subject of a `#define` \
                                 or an `#undef`",
                            ),
                        ),
                        MacroForm::FunctionLike => diagnostics.report(
                            refused(
                                &text,
                                token.span,
                                UNSUPPORTED_DIRECTIVE,
                                "function-like macros are not read yet",
                            )
                            .with_note("only an object-like `#define` is read yet"),
                        ),
                        MacroForm::Spliced => diagnostics.report(
                            refused(
                                &text,
                                token.span,
                                UNSUPPORTED_SPLICE,
                                "a line splice or a trigraph in a `#define` is not read yet",
                            )
                            .with_note(
                                "C can join the line below to this one, and it would be \
                                 compiled as code instead",
                            ),
                        ),
                        MacroForm::Other => diagnostics.report(
                            refused(
                                &text,
                                token.span,
                                UNSUPPORTED_DIRECTIVE,
                                "preprocessor directives are not supported yet",
                            )
                            // Which directives, rather than which stage.
                            // `docs/frontend.md` puts the preprocessor in its
                            // Stage 3 and `docs/roadmap.md` pulls four of them
                            // forward, so a stage number is both a number the
                            // reader does not have and one that stopped being
                            // the whole answer.
                            .with_note(
                                "`#include \"...\"`, an object-like `#define` and `#undef`, \
                                 `#ifdef`, `#ifndef`, `#else` and `#endif` are read",
                            )
                            .with_note("the line is one token and is not compiled"),
                        ),
                    },
                }
            }
            // Neither handed over nor expanded, so not in any extent either.
            _ if skipping => {}
            // An annotation's spelling too: `_Nonnull` is an identifier while
            // preprocessing, and `#define _Nonnull` with nothing after it is
            // how a header makes the annotation vanish for a compiler without
            // it. Left unexpanded, the promise C no longer makes was kept and
            // a dereference written out as `SC0403` passed in silence. Found
            // by review.
            TokenKind::Identifier | TokenKind::Keyword(_) | TokenKind::Annotation(_) => {
                match expansion(token, sources, reading, diagnostics) {
                    Some(frame) => frames.push(frame),
                    None => {
                        emitted(&mut frames, token.span);
                        spliced.push(token);
                    }
                }
            }
            _ => {
                emitted(&mut frames, token.span);
                spliced.push(token);
            }
        }
    }
}

/// The frame `token` expands to, if it names a macro, or `None` where it is
/// left as it is.
///
/// A keyword or an annotation is a name here too, since preprocessing knows
/// only identifiers (C17 6.4 p1), so `#define int long` and `#define
/// _Nonnull` are macros. A name is left as it is
/// where an expansion of it is open, which is the rule that a macro is not
/// replaced inside its own replacement (6.10.3.4 p2): an object-like macro's
/// rescan reads only its replacement list, so the open expansions are
/// exactly the replacements in progress.
///
/// Mutation: drop the open-expansion check. `#define X X + 1` expands for
/// ever, and the bound refuses it.
fn expansion(
    token: Token,
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> Option<Frame> {
    let name = sources.snippet(token.span);
    let spelled = reading.macros.get(name)?.spelled;
    if reading.open_macros.get(name).is_some_and(|open| *open > 0) {
        return None;
    }
    let name = name.to_owned();

    // A use written outside any expansion starts a tree of its own.
    if reading.open_expansions == 0 {
        reading.expansions = 0;
    }
    if reading.expansions_in_all >= MAX_EXPANSIONS_IN_ALL {
        if reading.expansions_in_all == MAX_EXPANSIONS_IN_ALL {
            reading.expansions_in_all += 1;
            diagnostics.report(
                Diagnostic::error("this input expands too many macros")
                    .with_code(TOO_MANY_EXPANSIONS)
                    .with_label(Label::primary(token.span, "expanded here"))
                    .with_note(format!(
                        "{MAX_EXPANSIONS_IN_ALL} expansions have already been read for it in all"
                    )),
            );
        }
        return None;
    }
    // Once, at the first past the bound, as `SC0112` is said.
    if reading.expansions >= MAX_EXPANSIONS {
        if reading.expansions == MAX_EXPANSIONS {
            reading.expansions += 1;
            diagnostics.report(
                Diagnostic::error("this use expands too many macros")
                    .with_code(TOO_MANY_EXPANSIONS)
                    .with_label(Label::primary(token.span, "expanded here"))
                    .with_note(format!(
                        "{MAX_EXPANSIONS} expansions have already been read for the use \
                         this is in"
                    )),
            );
        }
        return None;
    }
    reading.expansions += 1;
    reading.expansions_in_all += 1;
    *reading.open_macros.entry(name.clone()).or_insert(0) += 1;
    reading.open_expansions += 1;

    let file = sources.expand(spelled, token.span);
    // Lexed here, where it is used, so a character no token can begin is
    // reported at each use, which is where C makes it an error.
    let tokens = lex(file, &sources.file_owned(file), diagnostics);
    Some(Frame {
        tokens,
        cursor: 0,
        instance: Some(Instance::Expansion {
            file,
            at: token.span,
            name,
        }),
        extent: None,
        conditionals: Vec::new(),
        skipped_depth: 0,
    })
}

/// Record `#define name body`, or refuse it.
///
/// The body is scanned once, through a source map of its own, for two
/// things: an `#` or `##`, which this does not read, and its tokens, which a
/// second definition of the name is compared by. A second definition the same
/// is accepted, as C17 6.10.3 p2 requires, which a header with a `#define`
/// included twice needs; a different one is refused.
#[allow(clippy::too_many_arguments)]
fn define(
    text: &str,
    directive: Span,
    name: &str,
    name_at: u32,
    body: (u32, u32),
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) {
    let file = directive.file();
    let spelled = Span::new(file, directive.start() + body.0, directive.start() + body.1);
    let named = Span::new(
        file,
        directive.start() + name_at,
        directive.start() + name_at + name.len() as u32,
    );

    let mut scratch = SourceMap::new();
    let scanned = scratch.add_virtual("replacement list", sources.snippet(spelled));
    let scan = lex(scanned, scratch.file(scanned), &mut DiagnosticSink::new());
    let operator = scan.iter().any(|token| {
        matches!(
            token.kind,
            TokenKind::Punct(Punct::Hash | Punct::HashHash) | TokenKind::Directive
        )
    });
    if operator {
        diagnostics.report(
            refused(
                text,
                directive,
                UNSUPPORTED_DIRECTIVE,
                "`#` and `##` in a `#define` are not read yet",
            )
            .with_note("only a replacement list without them is read yet"),
        );
        return;
    }
    let mut tokens = Vec::new();
    let mut end = 0;
    for token in scan.iter().filter(|token| !token.is_eof()) {
        tokens.push((
            scratch.snippet(token.span).to_owned(),
            !tokens.is_empty() && token.span.start() > end,
        ));
        end = token.span.end();
    }

    if let Some(earlier) = reading.macros.get(name) {
        if earlier.tokens != tokens {
            diagnostics.report(
                Diagnostic::error(format!("`{name}` is defined again differently"))
                    .with_code(REDEFINED)
                    .with_label(Label::primary(named, "defined again here"))
                    .with_label(Label::secondary(earlier.defined, "defined here first"))
                    .with_note("a macro may be defined again only with the same replacement list"),
            );
        }
        return;
    }
    reading.macros.insert(
        name.to_owned(),
        Macro {
            spelled,
            defined: directive,
            tokens,
        },
    );
}

/// What a directive that is not an `#include` is, where it is a macro's.
#[derive(Debug, PartialEq, Eq)]
enum MacroForm<'a> {
    /// `#define name body`, with the offsets of the name and of the body's
    /// start and end in the directive's text.
    Define {
        name: &'a str,
        name_at: u32,
        body: (u32, u32),
    },
    /// `#undef name`.
    Undef { name: &'a str },
    /// `#define name(`, a function-like macro.
    FunctionLike,
    /// `#define defined` or `#undef defined`.
    Defined,
    /// A `#define` or an `#undef` whose name is not one this reads, such as
    /// one with a `$` in it, or an `#undef` with more after its name.
    Unread { directive: &'static str },
    /// A `#define` on a line that ends in a splice or holds a trigraph, for
    /// the reason [`IncludeForm::Spliced`] gives.
    Spliced,
    /// Anything else.
    Other,
}

/// Read a `#define` or an `#undef`.
fn macro_form(text: &str) -> MacroForm<'_> {
    let blank = [' ', '\t', '\u{b}', '\u{c}', '\r'];
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_';

    let Some(rest) = text.strip_prefix('#').or_else(|| text.strip_prefix("%:")) else {
        return MacroForm::Other;
    };
    let rest = rest.trim_start_matches(blank);
    let (define, rest) = if let Some(rest) = rest.strip_prefix("define") {
        (true, rest)
    } else if let Some(rest) = rest.strip_prefix("undef") {
        (false, rest)
    } else {
        return MacroForm::Other;
    };
    // `#defined` is a directive of another name.
    if rest.starts_with(identifier) {
        return MacroForm::Other;
    }
    let rest = rest.trim_start_matches(blank);
    let length = rest.len() - rest.trim_start_matches(identifier).len();
    let name = &rest[..length];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return MacroForm::Unread {
            directive: if define { "#define" } else { "#undef" },
        };
    }
    if name == "defined" {
        return MacroForm::Defined;
    }
    // What follows the name has to be where the name ends: a blank, the end,
    // or, for `#define`, the `(` of a function-like macro. Anything else is
    // a character this does not take for part of a name, `$` or a letter
    // outside ASCII among them, and reading the name as ending before it read
    // `#undef A$B` as `#undef A`, undefining a macro nobody named. Found by
    // review. C17 6.10.3 p3 asks for whitespace between an object-like
    // macro's name and its replacement list anyway.
    let after = &rest[length..];
    if !define {
        if after.trim_matches(blank).is_empty() {
            return MacroForm::Undef { name };
        }
        return MacroForm::Unread {
            directive: "#undef",
        };
    }
    if spliced(text) {
        return MacroForm::Spliced;
    }
    if after.starts_with('(') {
        return MacroForm::FunctionLike;
    }
    if !(after.is_empty() || after.starts_with(blank)) {
        return MacroForm::Unread {
            directive: "#define",
        };
    }
    let name_at = (text.len() - rest.len()) as u32;
    let body = after.trim_start_matches(blank);
    let start = text.len() - body.len();
    let end = start + body.trim_end_matches(blank).len();
    MacroForm::Define {
        name,
        name_at,
        body: (start as u32, end as u32),
    }
}

/// `text`, a directive's line, with each comment in it made spaces of the
/// same length, or `None` where a block comment in it does not end on it.
///
/// C makes a comment one space before directives are read (C17 5.1.1.2 p1,
/// phase 3), so a comment between `#` and `define`, or after a name, is
/// whitespace like any other; the same length keeps every offset into the
/// line where it was, so spans taken from the result are spans of the line.
/// A string or character literal is passed over, since `/*` in one opens
/// nothing. A comment that does not end on the line is refused rather than
/// read: the lexer ends a directive at its newline, so the lines inside the
/// comment would be compiled as code, and `#define X /*` above a null test
/// left the test to be believed where C has none. Found by review.
fn blanked(text: &str) -> Option<String> {
    let mut bytes = text.as_bytes().to_vec();
    let mut at = 0;
    let mut quote = None;
    while at < bytes.len() {
        match (quote, bytes[at]) {
            // An escape in a literal, whose next byte closes nothing.
            (Some(_), b'\\') if at + 1 < bytes.len() => {
                at += 2;
                continue;
            }
            (Some(open), byte) if byte == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(bytes[at]),
            (None, b'/') if bytes.get(at + 1) == Some(&b'/') => {
                bytes[at..].fill(b' ');
                break;
            }
            (None, b'/') if bytes.get(at + 1) == Some(&b'*') => {
                let end = text[at + 2..].find("*/")? + at + 4;
                bytes[at..end].fill(b' ');
                at = end;
                continue;
            }
            (None, _) => {}
        }
        at += 1;
    }
    Some(String::from_utf8(bytes).expect("only whole comments are made spaces"))
}

/// Whether a directive's line ends in a splice or holds a trigraph, either of
/// which changes what the line is before a directive is found (C17 5.1.1.2
/// p1, phases 1 and 2), and which the lexer does not perform in a directive.
fn spliced(text: &str) -> bool {
    let trigraph = text
        .as_bytes()
        .windows(3)
        .any(|three| three[..2] == *b"??" && b"=()/'<>!-".contains(&three[2]));
    text.trim_end_matches([' ', '\t', '\r']).ends_with('\\') || trigraph
}

/// A conditional directive, read as far as this reads one.
#[derive(Debug, PartialEq, Eq)]
enum ConditionalForm<'a> {
    /// `#if`, refused: its expression is not read yet.
    If,
    /// `#ifdef name` or, `negated`, `#ifndef name`.
    Ifdef {
        negated: bool,
        name: Named<'a>,
        extra: Option<(u32, u32)>,
    },
    /// `#elif`, refused for the reason `#if` is.
    Elif,
    Else {
        extra: Option<(u32, u32)>,
    },
    Endif {
        extra: Option<(u32, u32)>,
    },
}

/// The name an `#ifdef` or an `#ifndef` asks about.
#[derive(Debug, PartialEq, Eq)]
enum Named<'a> {
    Read(&'a str),
    /// None, or one beginning with a digit: C has no such directive.
    Missing,
    /// One this does not read, with a `$` or a letter outside ASCII in it,
    /// for the reason [`MacroForm::Unread`] gives.
    Unread,
}

/// Read `text`, a directive's line with its comments made spaces, as a
/// conditional directive, or `None` where it is another directive.
///
/// `extra` is where in `text` tokens follow what the directive takes, which
/// C's grammar does not allow and which is warned about and ignored.
fn conditional_form(text: &str) -> Option<ConditionalForm<'_>> {
    let blank = [' ', '\t', '\u{b}', '\u{c}', '\r'];
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_';
    // A name ends at a blank, the end, or a character that is not part of
    // any identifier, such as `(`; `$` or a letter outside ASCII would be.
    let ends = |rest: &str| !rest.starts_with(|c: char| identifier(c) || c == '$' || !c.is_ascii());
    // Where tokens go on after what the directive takes, if they do.
    let extra = |rest: &str| {
        let after = rest.trim_matches(blank);
        (!after.is_empty()).then(|| {
            let start = text.len() - rest.trim_start_matches(blank).len();
            (start as u32, (start + after.len()) as u32)
        })
    };

    let rest = text.strip_prefix('#').or_else(|| text.strip_prefix("%:"))?;
    let rest = rest.trim_start_matches(blank);
    let length = rest.len() - rest.trim_start_matches(identifier).len();
    let (word, rest) = rest.split_at(length);
    if !ends(rest) {
        return None;
    }
    let negated = match word {
        "if" => return Some(ConditionalForm::If),
        "elif" => return Some(ConditionalForm::Elif),
        "else" => return Some(ConditionalForm::Else { extra: extra(rest) }),
        "endif" => return Some(ConditionalForm::Endif { extra: extra(rest) }),
        "ifdef" => false,
        "ifndef" => true,
        _ => return None,
    };
    let rest = rest.trim_start_matches(blank);
    let length = rest.len() - rest.trim_start_matches(identifier).len();
    let (name, after) = rest.split_at(length);
    let name = if name.is_empty() && !ends(rest) {
        Named::Unread
    } else if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        Named::Missing
    } else if !ends(after) {
        Named::Unread
    } else {
        Named::Read(name)
    };
    Some(ConditionalForm::Ifdef {
        negated,
        name,
        extra: extra(after),
    })
}

/// Act on a conditional directive in `frame`, the frame of the file it is
/// written in.
///
/// A refused one, `#if`, `#elif` or an `#ifdef` this cannot read, still opens
/// or ends a level, so its `#endif` is matched rather than reported, and none
/// of its groups from there is compiled: compiling one would compile code
/// under a condition nobody evaluated. The refusal is an error, so nothing
/// past it is believed either way.
fn conditional(
    form: ConditionalForm,
    text: &str,
    directive: Span,
    frame: &mut Frame,
    reading: &Reading,
    diagnostics: &mut DiagnosticSink,
) {
    let opens = matches!(form, ConditionalForm::If | ConditionalForm::Ifdef { .. });
    // Inside a conditional opened in a skipped group, only the level counts.
    if frame.skipped_depth > 0 || (opens && frame.skipping()) {
        if opens {
            frame.skipped_depth += 1;
        } else if matches!(form, ConditionalForm::Endif { .. }) && frame.skipped_depth > 0 {
            frame.skipped_depth -= 1;
        } else if frame.skipped_depth == 0 {
            // `#else`, `#elif` or `#endif` of the skipped conditional itself.
            act(form, text, directive, frame, reading, diagnostics);
        }
        return;
    }
    act(form, text, directive, frame, reading, diagnostics);
}

/// Whether C reserves `name` for the implementation in every use: one that
/// begins with `__`, or with `_` and a capital letter (C17 7.1.3 p1).
fn reserved(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next() == Some('_')
        && characters
            .next()
            .is_some_and(|second| second == '_' || second.is_ascii_uppercase())
}

/// Act on a conditional directive that is not inside a skipped group's own
/// nested conditional: open a level, choose a group, or close one.
fn act(
    form: ConditionalForm,
    text: &str,
    directive: Span,
    frame: &mut Frame,
    reading: &Reading,
    diagnostics: &mut DiagnosticSink,
) {
    let at = hash(text, directive);
    // A spliced line is refused by the frame loop, and acted on here as far
    // as the line goes.
    let spliced = spliced(text);
    let ignored = |extra: Option<(u32, u32)>, diagnostics: &mut DiagnosticSink, what: &str| {
        // Not on a spliced line, whose `\` is no token and is reported.
        if let Some((start, end)) = extra.filter(|_| !spliced) {
            let span = Span::new(
                directive.file(),
                directive.start() + start,
                directive.start() + end,
            );
            diagnostics.report(
                Diagnostic::warning(format!("tokens after `{what}` are ignored"))
                    .with_code(EXTRA_TOKENS)
                    .with_label(Label::primary(span, "ignored"))
                    .with_note("a comment there says the same without a warning"),
            );
        }
    };
    let open = |frame: &mut Frame, directive: &'static str, taking: Option<bool>| {
        frame.conditionals.push(Conditional {
            directive,
            opened: at,
            otherwise: None,
            settled: taking.is_none_or(|taking| taking),
            taking: taking.unwrap_or(false),
        });
    };
    let unmatched = |diagnostics: &mut DiagnosticSink, message: &str| {
        diagnostics.report(
            refused(text, directive, UNBALANCED, message)
                .with_note("no `#if`, `#ifdef` or `#ifndef` is open in this file"),
        );
    };
    match form {
        ConditionalForm::If => {
            diagnostics.report(
                refused(
                    text,
                    directive,
                    UNSUPPORTED_DIRECTIVE,
                    "`#if` is not read yet",
                )
                .with_note(
                    "only `#ifdef` and `#ifndef` are read yet, and none of this conditional's \
                     groups is compiled",
                ),
            );
            open(frame, "#if", None);
        }
        ConditionalForm::Ifdef {
            negated,
            name,
            extra,
        } => {
            let what = if negated { "#ifndef" } else { "#ifdef" };
            match name {
                // A name C reserves for the implementation, which may have
                // defined it (C17 7.1.3 p1), and which this cannot answer:
                // no predefined macro exists here, so `__STDC_VERSION__` was
                // not defined, and the group a compiler builds was skipped.
                // A double free under `#ifdef __STDC_VERSION__` was analysed
                // away. Found by review. A name the source defines first is
                // answered. An include guard named `_FOO_H` is refused at
                // every inclusion, since the `#define` inside it is in a
                // group this does not compile. A name the implementation
                // defines without reserving it, `unix` in a GNU mode, is not
                // caught.
                Named::Read(name) if reserved(name) && !reading.macros.contains_key(name) => {
                    diagnostics.report(
                        refused(
                            text,
                            directive,
                            UNSUPPORTED_DIRECTIVE,
                            &format!(
                                "this `{what}` asks about a name the implementation may define"
                            ),
                        )
                        .with_note(format!(
                            "`{name}` is reserved for the implementation (C17 7.1.3), and \
                             predefined macros are not read yet, so none of this conditional's \
                             groups is compiled"
                        )),
                    );
                    open(frame, what, None);
                }
                Named::Read(name) => {
                    ignored(extra, diagnostics, what);
                    open(
                        frame,
                        what,
                        Some(reading.macros.contains_key(name) != negated),
                    );
                }
                Named::Missing => {
                    diagnostics.report(
                        refused(
                            text,
                            directive,
                            UNBALANCED,
                            &format!("`{what}` names no macro"),
                        )
                        .with_note(format!(
                            "`{what}` takes one name, and none of this conditional's groups \
                                 is compiled"
                        )),
                    );
                    open(frame, what, None);
                }
                Named::Unread => {
                    diagnostics.report(
                        refused(
                            text,
                            directive,
                            UNSUPPORTED_DIRECTIVE,
                            &format!("this `{what}` is not read yet"),
                        )
                        .with_note(format!(
                            "an `{what}` is read when its name is ASCII letters, digits and `_`, \
                             and none of this conditional's groups is compiled"
                        )),
                    );
                    open(frame, what, None);
                }
            }
        }
        ConditionalForm::Elif => {
            let Some(conditional) = frame.conditionals.last_mut() else {
                unmatched(diagnostics, "this `#elif` belongs to no conditional");
                return;
            };
            if let Some(otherwise) = conditional.otherwise {
                diagnostics.report(
                    refused(
                        text,
                        directive,
                        UNBALANCED,
                        "an `#elif` after this conditional's `#else`",
                    )
                    .with_label(Label::secondary(otherwise, "`#else` here")),
                );
            } else {
                diagnostics.report(
                    refused(
                        text,
                        directive,
                        UNSUPPORTED_DIRECTIVE,
                        "`#elif` is not read yet",
                    )
                    .with_note(
                        "only `#ifdef` and `#ifndef` are read yet, and none of this \
                             conditional's groups from here is compiled",
                    ),
                );
            }
            conditional.settled = true;
            conditional.taking = false;
        }
        ConditionalForm::Else { extra } => {
            let Some(conditional) = frame.conditionals.last_mut() else {
                unmatched(diagnostics, "this `#else` belongs to no conditional");
                return;
            };
            ignored(extra, diagnostics, "#else");
            if let Some(otherwise) = conditional.otherwise {
                diagnostics.report(
                    refused(
                        text,
                        directive,
                        UNBALANCED,
                        "this conditional already has an `#else`",
                    )
                    .with_label(Label::secondary(otherwise, "first `#else` here"))
                    .with_note("the group after this one is not compiled"),
                );
                conditional.taking = false;
                return;
            }
            conditional.otherwise = Some(at);
            conditional.taking = !conditional.settled;
            conditional.settled = true;
        }
        ConditionalForm::Endif { extra } => {
            if frame.conditionals.pop().is_none() {
                unmatched(diagnostics, "this `#endif` closes no conditional");
                return;
            }
            ignored(extra, diagnostics, "#endif");
        }
    }
}

/// One file being read: its tokens, how far they have been read, and, for an
/// inclusion, which file it is, the `#include` it was read for, and the
/// first and last thing of its own the parser has been handed so far.
///
/// The inclusion is said by the frame rather than by its place above the
/// input's, so that a frame of another kind, a macro's expansion, cannot pop
/// a file that is still being read: one did, in a prototype, and the cycle it
/// hid was refused as too many inclusions instead.
///
/// Each frame holds the conditionals open in its own file, because each
/// source file is a preprocessing file of its own (C17 6.10 p1) and an
/// if-section cannot begin in a header and end in its includer. An
/// expansion's frame never holds a directive, so its stack stays empty.
struct Frame {
    tokens: Vec<Token>,
    cursor: usize,
    instance: Option<Instance>,
    extent: Option<(Span, Span)>,
    conditionals: Vec<Conditional>,
    /// Conditionals opened inside a skipped group, which are counted and not
    /// read (C17 6.10.1 p6).
    skipped_depth: usize,
}

impl Frame {
    /// Whether the group being read is skipped.
    fn skipping(&self) -> bool {
        self.conditionals
            .last()
            .is_some_and(|conditional| !conditional.taking)
    }
}

/// One open `#if`, `#ifdef` or `#ifndef` in the file a frame reads.
struct Conditional {
    /// `#if`, `#ifdef` or `#ifndef`, for the report of one never closed.
    directive: &'static str,
    /// The `#` that opened it, where one never closed is reported.
    opened: Span,
    /// Its `#else`, where a second one is reported.
    otherwise: Option<Span>,
    /// Whether a group of it has been compiled or it was refused, either of
    /// which skips every later group.
    settled: bool,
    /// Whether the group being read is compiled.
    taking: bool,
}

/// What a frame above the input's is.
enum Instance {
    /// An inclusion: its file, and the `#include` it was read for.
    Inclusion { file: FileId, directive: Span },
    /// An expansion: its file, the name it was used as, and that name, which
    /// is what keeps it from being expanded again inside itself.
    Expansion {
        file: FileId,
        at: Span,
        name: String,
    },
}

impl Instance {
    fn file(&self) -> FileId {
        match self {
            Self::Inclusion { file, .. } | Self::Expansion { file, .. } => *file,
        }
    }

    /// What the frame below was handed in its place.
    fn used(&self) -> Span {
        match self {
            Self::Inclusion { directive, .. } => *directive,
            Self::Expansion { at, .. } => *at,
        }
    }
}

/// Note that the innermost open frame handed `token` to the parser.
///
/// Each frame below it is handed the `#include` or the name the one above
/// was read for, once that one ends and if it handed over anything, which is
/// when the frame loop calls this again with that use. So an inclusion's
/// extent is what of it the parser reads, an included file standing as its
/// directive: a directive that hands nothing over, `#define` or an
/// `#include` of an empty file, is not in it. Taken from the scan instead,
/// such a line was the first token of a header whose code a node holds
/// whole, and its lift would have refused C. Found by review. See
/// `SourceMap::set_extent`.
///
/// Only the innermost frame, rather than every frame below it at each token:
/// that was a walk as deep as the open expansions for every token, and a
/// chain of distinct macros 32000 deep took 40 s. Found by review.
fn emitted(frames: &mut [Frame], token: Span) {
    if let Some(frame) = frames.last_mut() {
        let first = frame.extent.map_or(token, |(first, _)| first);
        frame.extent = Some((first, token));
    }
}

/// Read the file `name` names, relative to the file the directive is in, and
/// give back the file and its tokens, with it pushed onto the files being
/// read; or report why not, and give back nothing.
///
/// `directive` is the whole line and `quoted` the name with its quotes, which
/// is where a report about the file is put.
fn include(
    directive: Span,
    quoted: Span,
    name: &str,
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> Option<(FileId, Vec<Token>)> {
    let unreadable = |note: String| {
        Diagnostic::error("cannot read the included file")
            .with_code(UNREADABLE_INCLUSION)
            .with_label(Label::primary(quoted, "included here"))
            .with_note(note)
    };

    let Some(includer) = path_of(sources.file(directive.file()).name()) else {
        diagnostics.report(unreadable(format!(
            "`{name}` is found from the directory of the file that includes it, and this \
             file has no directory"
        )));
        return None;
    };
    let path = resolved(includer, name);

    if reading.open.len() == MAX_DEPTH {
        diagnostics.report(
            Diagnostic::error("this `#include` is nested too deeply")
                .with_code(TOO_DEEP)
                .with_label(Label::primary(quoted, "included here"))
                .with_note(format!("{MAX_DEPTH} files are already being read here")),
        );
        return None;
    }
    // Once, at the first one past it, rather than at each of the thousands
    // after it, which would bury the report it is.
    if reading.inclusions >= MAX_INCLUSIONS {
        if reading.inclusions > MAX_INCLUSIONS {
            return None;
        }
        reading.inclusions += 1;
        diagnostics.report(
            Diagnostic::error("this input includes too many files")
                .with_code(TOO_MANY)
                .with_label(Label::primary(quoted, "included here"))
                .with_note(format!(
                    "{MAX_INCLUSIONS} inclusions have already been read for it, and \
                     include guards are not read yet"
                )),
        );
        return None;
    }

    let real = canonical(&path);
    if let Some(&(_, open)) = reading.open.iter().find(|(being, _)| *being == real) {
        // Of the `#include`, not of the file it is in: the file being read may
        // be another one, which includes this one through others.
        let refusal =
            Diagnostic::error("this `#include` would read a file that is already being read")
                .with_code(RECURSIVE_INCLUSION)
                .with_label(Label::primary(quoted, "already being read"));
        // Where the open one began to be read: the directive that included
        // it, or nowhere, if it is the input. Asked of the file rather than
        // of its place on the stack, which says the same only while the input
        // is the first thing pushed.
        let refusal = match sources.file(open).name() {
            FileName::Inclusion { at, .. } => refusal
                .with_label(Label::secondary(*at, "where it began to be read"))
                .with_note(format!(
                    "`{}` has not finished, so reading it here would never end",
                    path.display()
                )),
            FileName::Real(_) | FileName::Virtual(_) => refusal.with_note(format!(
                "`{}` is the input, so reading it here would never end",
                path.display()
            )),
            FileName::Expansion { .. } => {
                unreachable!("an expansion is never on the stack of files being read")
            }
        };
        diagnostics.report(refusal);
        return None;
    }

    // Asked before the file is opened, because opening is what never returns
    // for a console or a pipe: `#include "CON"` waited on the terminal until
    // it was killed. Found by review. A device that cannot even be asked, as
    // Windows answers for `CON`, is refused with what it said.
    let not_a_file = match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() || metadata.is_dir() => None,
        Ok(_) => Some("this is not a regular file, and safec reads a source file".to_owned()),
        Err(error) => Some(reason(&path, &error, Access::Read)),
    };
    if let Some(why) = not_a_file {
        diagnostics.report(unreadable(format!(
            "`{name}` was looked for at `{}`: {why}",
            path.display()
        )));
        return None;
    }

    let file = match sources.include(&path, directive) {
        Ok(file) => file,
        Err(error) => {
            // The name as written, which is what the reader can find in the
            // directive above. Where it was looked for follows it.
            let why = if error.kind() == std::io::ErrorKind::InvalidData {
                "it is not valid UTF-8, and safec reads source files as UTF-8".to_owned()
            } else {
                reason(&path, &error, Access::Read)
            };
            diagnostics.report(unreadable(format!(
                "`{name}` was looked for at `{}`: {why}",
                path.display()
            )));
            return None;
        }
    };

    reading.inclusions += 1;
    let tokens = lex(file, &sources.file_owned(file), diagnostics);
    reading.open.push((real, file));
    Some((file, tokens))
}

/// What an `#include` is spelled as, which decides what is done with it.
#[derive(Debug, PartialEq, Eq)]
enum IncludeForm<'a> {
    /// `#include "name"`, with `quote` the offset of the opening quote in
    /// the directive's text.
    Quoted { name: &'a str, quote: u32 },
    /// `#include <name>`.
    Angled,
    /// `#include "name"` on a line that ends in a splice or holds a trigraph.
    ///
    /// Both change what the line is before any directive is found (C17
    /// 5.1.1.2 p1, phases 1 and 2), and the lexer ends a directive at its
    /// newline without performing either. Read as it stands, `// \` after the
    /// name left the line below to be compiled where C makes it part of the
    /// comment, and a null test there went missing from the safety check.
    /// Found by review.
    Spliced,
    /// An `#include` that is none of those, such as one naming a macro or one
    /// with more than a comment after its name.
    Unread,
    /// Any other directive.
    Other,
}

/// Read a directive's text, from its `#` to the end of its line.
///
/// Space and horizontal tab may stand between the `#`, the name and the file
/// (C17 6.10 p5). A vertical tab and a form feed are taken as spaces, which
/// the same paragraph allows translation phase 3 to have done, and a comment
/// may follow the file. A `\r` is whitespace here too, because the
/// line of a file saved with CRLF ends in one, and a header that did not read
/// on Windows would be a header that did not read for most of the people who
/// write one.
fn include_form(text: &str) -> IncludeForm<'_> {
    let blank = [' ', '\t', '\u{b}', '\u{c}', '\r'];

    let Some(rest) = text.strip_prefix('#').or_else(|| text.strip_prefix("%:")) else {
        return IncludeForm::Other;
    };
    let Some(rest) = rest.trim_start_matches(blank).strip_prefix("include") else {
        return IncludeForm::Other;
    };
    // `#included` is a directive of another name, and is not refused as an
    // `#include` of a form not read.
    if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        return IncludeForm::Other;
    }
    let rest = rest.trim_start_matches(blank);
    if rest.starts_with('<') {
        return IncludeForm::Angled;
    }
    let Some(quoted) = rest.strip_prefix('"') else {
        return IncludeForm::Unread;
    };
    let Some((name, mut tail)) = quoted.split_once('"') else {
        return IncludeForm::Unread;
    };

    loop {
        tail = tail.trim_start_matches(blank);
        if tail.is_empty() || tail.starts_with("//") {
            break;
        }
        match tail
            .strip_prefix("/*")
            .and_then(|comment| comment.split_once("*/"))
        {
            Some((_, after)) => tail = after,
            None => return IncludeForm::Unread,
        }
    }

    if spliced(text) {
        return IncludeForm::Spliced;
    }

    IncludeForm::Quoted {
        name,
        quote: (text.len() - rest.len()) as u32,
    }
}

/// The report for a directive this does not read, at its `#`.
///
/// The `#`, not the line. A caret under a long macro definition is a wall, and
/// the line is quoted above it anyway.
fn refused(text: &str, directive: Span, code: Code, message: &str) -> Diagnostic {
    Diagnostic::error(message)
        .with_code(code)
        .with_label(Label::primary(
            hash(text, directive),
            "directive begins here",
        ))
}

/// The `#` that begins `directive`, whose text is `text`.
fn hash(text: &str, directive: Span) -> Span {
    // Two bytes where the `#` is written `%:`.
    let hash = if text.starts_with("%:") { 2 } else { 1 };
    Span::new(
        directive.file(),
        directive.start(),
        directive.start() + hash,
    )
}

/// The path a file was read from, if it was read from one.
fn path_of(name: &FileName) -> Option<&Path> {
    match name {
        FileName::Real(path) | FileName::Inclusion { path, .. } => Some(path),
        // No `#include` is ever in one, since a replacement list holds no
        // directive, so nothing is resolved from it.
        FileName::Virtual(_) | FileName::Expansion { .. } => None,
    }
}

/// `name` found from the directory `includer` is in.
///
/// Joined with a `/` rather than by [`Path::join`], which writes `\` on
/// Windows: the path reaches a diagnostic, and a diagnostic says the same thing
/// on every host. Windows reads a `/` as a separator, so nothing it opens
/// changes.
fn resolved(includer: &Path, name: &str) -> PathBuf {
    let directory = includer.parent().unwrap_or(Path::new(""));
    if Path::new(name).has_root() || directory.as_os_str().is_empty() {
        return PathBuf::from(name);
    }
    let mut path = OsString::from(directory.as_os_str());
    path.push("/");
    path.push(name);
    PathBuf::from(path)
}

/// What a cycle is found by: the file itself, whatever path reached it.
///
/// A file that cannot be canonicalized cannot be read either, so its own
/// spelling is enough to stand for it until the read fails.
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spelling C allows of the one form read.
    ///
    /// Mutation: take `\r` out of `blank`. The line a CRLF file ends with is
    /// not read.
    #[test]
    fn an_include_is_read_however_it_is_spaced() {
        for (text, name) in [
            ("#include \"a.h\"", "a.h"),
            ("#  include\t\"a.h\"", "a.h"),
            ("#include\"a.h\"", "a.h"),
            ("%:include \"a.h\"", "a.h"),
            ("#include \"a.h\"\r", "a.h"),
            ("#include \"a.h\" // why", "a.h"),
            ("#include \"a.h\" /* why */ ", "a.h"),
            ("#include \"dir/a b.h\"", "dir/a b.h"),
        ] {
            let IncludeForm::Quoted { name: read, quote } = include_form(text) else {
                panic!(
                    "{text:?} is not read as an include: {:?}",
                    include_form(text)
                );
            };
            assert_eq!(read, name, "{text:?}");
            assert_eq!(&text[quote as usize..quote as usize + 1], "\"", "{text:?}");
        }
    }

    /// And the ones beside it that are not it.
    ///
    /// Mutation: accept anything after the closing quote. `"a.h" x` is read.
    /// Mutation: answer an `#include` of another form as any other
    /// directive. Its note says `#include "..."` is read, under a refusal of
    /// one. Mutation: drop the test that `include` ends the name. `#included`
    /// is refused as an `#include`.
    #[test]
    fn what_is_not_a_quoted_include_is_not_read_as_one() {
        for (text, expected) in [
            ("#include <stdio.h>", IncludeForm::Angled),
            ("#  include<stdio.h>", IncludeForm::Angled),
            ("#define X 1", IncludeForm::Other),
            ("#included \"a.h\"", IncludeForm::Other),
            ("#include HEADER", IncludeForm::Unread),
            ("#include \"a.h", IncludeForm::Unread),
            ("#include \"a.h\" x", IncludeForm::Unread),
            ("#include \"a.h\"\rint z;", IncludeForm::Unread),
            ("#include \"a.h\" /* open", IncludeForm::Unread),
            ("#include \"a.h\" // \\", IncludeForm::Spliced),
            ("#include \"a.h\" // \\ \r", IncludeForm::Spliced),
            ("#include \"a.h\" // ??/", IncludeForm::Spliced),
            ("#include \"a??=.h\"", IncludeForm::Spliced),
            ("#", IncludeForm::Other),
        ] {
            assert_eq!(include_form(text), expected, "{text:?}");
        }
    }

    /// The directory is joined with `/` on every host, so a diagnostic naming
    /// the path is the same on Windows as elsewhere.
    ///
    /// Mutation: resolve with `Path::join`. Fails on Windows, which is where
    /// the separator differs.
    #[test]
    fn a_name_is_joined_to_the_including_file_s_directory_with_a_slash() {
        assert_eq!(resolved(Path::new("a.c"), "a.h"), PathBuf::from("a.h"));
        assert_eq!(resolved(Path::new("src/a.c"), "a.h").as_os_str(), "src/a.h");
        assert_eq!(
            resolved(Path::new("src/a.c"), "/usr/a.h"),
            PathBuf::from("/usr/a.h")
        );
    }

    /// A file with no path has no directory to find a name from, and is not
    /// taken to be in the one the compiler happens to run in.
    ///
    /// Mutation: resolve against an empty directory for a file with no path.
    /// The note says the name was looked for, at a path nobody wrote.
    #[test]
    fn an_include_in_a_file_with_no_path_is_refused() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "#include \"a.h\"\n");
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);

        let [refused] = diagnostics.diagnostics() else {
            panic!("{:?}", diagnostics.diagnostics());
        };
        assert_eq!(
            refused.code().map(|code| code.to_string()).as_deref(),
            Some("SC0109")
        );
        assert_eq!(
            refused.notes(),
            [
                "`a.h` is found from the directory of the file that includes it, and this file \
              has no directory"
            ]
        );
    }

    /// The codes reported for `main.c` in a directory of its own holding
    /// `files`, each a path under it and what it holds.
    fn reported(name: &str, files: &[(String, String)]) -> Vec<String> {
        let directory = std::env::temp_dir().join(format!("safec_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        for (path, contents) in files {
            let path = directory.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        let mut sources = SourceMap::new();
        let file = sources.load(directory.join("main.c")).unwrap();
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);
        let _ = fs::remove_dir_all(&directory);

        diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect()
    }

    /// A chain of different files, each including the next, is refused past
    /// the bound rather than read until the native stack runs out, and read
    /// up to it: `MAX_DEPTH` files at once, the input among them.
    ///
    /// Run on a thread whose stack is a quarter of a megabyte, smaller than
    /// the main thread's anywhere this runs, so that reading the chain the
    /// bound allows is asked of a stack that could not hold one frame per file.
    ///
    /// Mutation: drop the check of `MAX_DEPTH`. The longer chain is read to
    /// its end and nothing is reported. Mutation: refuse one file sooner or one
    /// later. One of the two chains answers wrongly. Mutation: recurse once
    /// per file, as `read_includes` did. The thread overflows its stack.
    #[test]
    fn includes_nested_past_the_bound_are_refused() {
        // Named, so that an overflow, which aborts the whole test binary,
        // still says which test it was.
        std::thread::Builder::new()
            .name("includes_nested_past_the_bound_are_refused".to_owned())
            .stack_size(256 * 1024)
            .spawn(nested_past_the_bound)
            .unwrap()
            .join()
            .unwrap();
    }

    fn nested_past_the_bound() {
        let chain = |headers: usize| {
            let mut files = vec![("main.c".to_owned(), "#include \"0.h\"\n".to_owned())];
            for at in 0..headers - 1 {
                files.push((format!("{at}.h"), format!("#include \"{}.h\"\n", at + 1)));
            }
            files.push((format!("{}.h", headers - 1), "int z;\n".to_owned()));
            files
        };

        assert_eq!(
            reported("deep_enough", &chain(MAX_DEPTH - 1)),
            Vec::<String>::new()
        );
        assert_eq!(reported("too_deep", &chain(MAX_DEPTH)), ["SC0111"]);
    }

    /// A header that includes the next one twice, thirteen deep, is refused
    /// once its inclusions pass the bound, and said once.
    ///
    /// Mutation: drop the bound. Every inclusion is read and nothing is
    /// reported. Mutation: report every inclusion past it. One is reported
    /// for each file still open, since a refused one reads nothing more.
    #[test]
    fn a_tree_of_inclusions_past_the_bound_is_refused_once() {
        let levels = 13;
        let mut files = vec![("main.c".to_owned(), "#include \"0.h\"\n".to_owned())];
        for at in 0..levels {
            let next = format!("#include \"{}.h\"\n", at + 1);
            files.push((format!("{at}.h"), next.repeat(2)));
        }
        files.push((format!("{levels}.h"), "int z;\n".to_owned()));

        assert_eq!(reported("wide", &files), ["SC0112"]);
    }

    /// A device is refused before it is opened, since opening one can wait
    /// for ever: `CON` waited on the terminal.
    ///
    /// Mutation: drop the question asked before opening. The device reads as
    /// an empty file and nothing is reported.
    #[test]
    fn an_include_of_a_device_is_refused() {
        let device = if cfg!(windows) {
            "//./NUL"
        } else {
            "/dev/null"
        };
        let files = [("main.c".to_owned(), format!("#include \"{device}\"\n"))];

        assert_eq!(reported("device", &files), ["SC0109"]);
    }

    /// The preprocessor records each inclusion's first and last token, so a
    /// span at the start of an included file lifts to its `#include`. The
    /// header opens with a comment, so the file's first byte is not its first
    /// token.
    ///
    /// Mutation: drop the `set_extent` call in `read_includes`. The lift
    /// panics, since the inclusion has no extent.
    #[test]
    fn a_span_at_the_start_of_an_included_file_lifts_to_its_include() {
        let directory = std::env::temp_dir().join(format!("safec_extent_{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("main.c"), "#include \"a.h\"\n").unwrap();
        fs::write(directory.join("a.h"), "/* c */ int z;\n").unwrap();

        let mut sources = SourceMap::new();
        let main = sources.load(directory.join("main.c")).unwrap();
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(main, &sources.file_owned(main), &mut diagnostics);
        let preprocessed = preprocess(main, &tokens, &mut sources, &mut diagnostics);
        let _ = fs::remove_dir_all(&directory);

        let int = preprocessed.tokens()[0].span;
        assert_eq!((int.start(), int.end()), (8, 11));
        assert_eq!(sources.lift_start(int, main), Span::new(main, 0, 14));
    }

    /// An inclusion's extent is what of it the parser reads: a first line that
    /// hands nothing over is not its start, and a nested inclusion that does
    /// stands as its `#include`.
    ///
    /// `a.h` opens with an `#include` of an empty file and then `int z;`, so
    /// a node holding all of `a.h` starts at `int`; `b.h` opens with an
    /// `#include` of `c.h`, which holds `int w;`, so `c.h`'s `int` lifts
    /// through `b.h`'s directive.
    ///
    /// Mutation: take the extent from the inclusion's own scan, directives
    /// included. `a.h` then starts at the `#include` of the empty file, and
    /// the first lift panics.
    #[test]
    fn an_inclusion_starts_at_the_first_thing_the_parser_reads_of_it() {
        let directory = std::env::temp_dir().join(format!("safec_reads_{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("main.c"),
            "#include \"a.h\"\n#include \"b.h\"\n",
        )
        .unwrap();
        fs::write(directory.join("a.h"), "#include \"empty.h\"\nint z;\n").unwrap();
        fs::write(directory.join("empty.h"), "").unwrap();
        fs::write(directory.join("b.h"), "#include \"c.h\"\n").unwrap();
        fs::write(directory.join("c.h"), "int w;\n").unwrap();

        let mut sources = SourceMap::new();
        let main = sources.load(directory.join("main.c")).unwrap();
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(main, &sources.file_owned(main), &mut diagnostics);
        let preprocessed = preprocess(main, &tokens, &mut sources, &mut diagnostics);
        let _ = fs::remove_dir_all(&directory);
        assert!(!diagnostics.has_errors(), "{:?}", diagnostics.diagnostics());

        let read = preprocessed.tokens();
        assert_eq!(
            sources.lift_start(read[0].span, main),
            Span::new(main, 0, 14)
        );
        assert_eq!(sources.lift_end(read[2].span, main), Span::new(main, 0, 14));
        assert_eq!(
            sources.lift_start(read[3].span, main),
            Span::new(main, 15, 29)
        );
    }

    /// An included file's own includes are found beside it, not beside the
    /// input: `sub/b.h` writes `c.h` and means `sub/c.h`.
    ///
    /// Mutation: resolve every include against the input's directory. `c.h`
    /// is looked for beside `main.c`, where there is none.
    #[test]
    fn an_include_in_an_included_file_is_found_beside_that_file() {
        let files = [
            ("main.c", "#include \"sub/b.h\"\n"),
            ("sub/b.h", "#include \"c.h\"\n"),
            ("sub/c.h", "int c;\n"),
        ]
        .map(|(path, contents)| (path.to_owned(), contents.to_owned()));

        assert_eq!(reported("nested", &files), Vec::<String>::new());
    }

    /// No directive is left in what the parser is handed, whichever way it
    /// was refused: what `Preprocessed` promises, asked of every arm.
    ///
    /// Through the driver nothing would notice, since a reported directive
    /// closes the gate before the parse and the parser's assertion is never
    /// reached. Found by review. Mutation: have any one arm of
    /// `read_includes` push the directive it reports. Its row fails.
    #[test]
    fn no_directive_is_left_in_what_is_preprocessed() {
        for text in [
            "#pragma pack(4)\n",
            "#include <a.h>\n",
            "#include X\n",
            "#include \"a.h\" // \\\nint x;\n",
            "#include \"a.h\"\n",
            "#define F(x) x\n",
            "#define G a ## b\n",
            "#define H \\\nint x;\n",
        ] {
            let mut sources = SourceMap::new();
            let file = sources.add_virtual("t.c", text);
            let mut diagnostics = DiagnosticSink::new();
            let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
            let preprocessed = preprocess(file, &tokens, &mut sources, &mut diagnostics);

            assert!(diagnostics.has_errors(), "{text:?} was not refused");
            assert!(
                preprocessed
                    .tokens()
                    .iter()
                    .all(|token| token.kind != TokenKind::Directive),
                "{text:?}"
            );
        }
    }

    /// A macro whose body uses the next one twice, sixteen deep, is refused
    /// once its use's expansions pass the bound for one use, and said once.
    /// 131,071 expansions, under the bound for the input, so only the bound
    /// for one use is asked.
    ///
    /// Mutation: drop the bound for one use. Every expansion is read and
    /// nothing is reported. Mutation: report every expansion past it. More
    /// than one is.
    #[test]
    fn a_tree_of_expansions_past_the_bound_is_refused_once() {
        let mut text = String::from("#define M0 1\n");
        for at in 1..=16 {
            text.push_str(&format!("#define M{at} M{} M{}\n", at - 1, at - 1));
        }
        text.push_str("int x = M16;\n");
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);

        let codes: Vec<String> = diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect();
        assert_eq!(codes, ["SC0113"]);
    }

    /// The bound is per use: a macro used once on each of more lines than
    /// the bound allows expansions is read, since each use is a tree of one.
    ///
    /// Mutation: count every expansion in the input. The last use is
    /// refused as `SC0113`.
    #[test]
    fn a_macro_used_on_many_lines_is_not_bounded_as_one_tree() {
        let mut text = String::from("#define ONE 1\n");
        for at in 0..=MAX_EXPANSIONS {
            text.push_str(&format!("int v{at} = ONE;\n"));
        }
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);

        assert!(
            !diagnostics.has_errors(),
            "{:?}",
            diagnostics.diagnostics().first()
        );
    }

    /// Many uses of trees under the bound for one use are refused once
    /// their expansions pass the bound for the input.
    ///
    /// Mutation: drop the bound for the input. Every use is read and
    /// nothing is reported.
    #[test]
    fn many_uses_of_large_trees_are_refused_once_past_the_bound_for_the_input() {
        let mut text = String::from("#define M0 1\n");
        for at in 1..=12 {
            text.push_str(&format!("#define M{at} (M{} + M{})\n", at - 1, at - 1));
        }
        let uses = MAX_EXPANSIONS_IN_ALL / ((1 << 13) - 1) + 1;
        for at in 0..uses {
            text.push_str(&format!("int v{at} = M12;\n"));
        }
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);

        let codes: Vec<String> = diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect();
        assert_eq!(codes, ["SC0113"]);
    }

    /// Each conditional directive is read by its name, a name that only
    /// begins with one is not one, and what follows is where it is.
    ///
    /// Mutation: drop the check that the directive's name ends where a name
    /// does. `#if$` is an `#if`. Mutation: start `extra` before the blanks
    /// in front of it. Its offsets move.
    #[test]
    fn a_conditional_directive_is_read_by_its_name() {
        use ConditionalForm::*;
        assert_eq!(conditional_form("#if 1"), Some(If));
        assert_eq!(conditional_form("%:elif X"), Some(Elif));
        assert_eq!(conditional_form("#  else"), Some(Else { extra: None }));
        assert_eq!(conditional_form("#endif     "), Some(Endif { extra: None }));
        assert_eq!(
            conditional_form("#endif X  "),
            Some(Endif {
                extra: Some((7, 8))
            })
        );
        assert_eq!(
            conditional_form("#ifdef A"),
            Some(Ifdef {
                negated: false,
                name: Named::Read("A"),
                extra: None
            })
        );
        assert_eq!(
            conditional_form("#ifndef A B"),
            Some(Ifdef {
                negated: true,
                name: Named::Read("A"),
                extra: Some((10, 11))
            })
        );
        assert_eq!(
            conditional_form("#ifdef"),
            Some(Ifdef {
                negated: false,
                name: Named::Missing,
                extra: None
            })
        );
        assert_eq!(
            conditional_form("#ifdef 1"),
            Some(Ifdef {
                negated: false,
                name: Named::Missing,
                extra: None
            })
        );
        assert_eq!(
            conditional_form("#ifdef A$B"),
            Some(Ifdef {
                negated: false,
                name: Named::Unread,
                extra: Some((8, 10))
            })
        );
        assert_eq!(conditional_form("#ifdefine X"), None);
        assert_eq!(conditional_form("#if$"), None);
        assert_eq!(conditional_form("#include \"a.h\""), None);
        let blank = blanked("#endif /* U */").unwrap();
        assert_eq!(conditional_form(&blank), Some(Endif { extra: None }));
    }

    /// No group of a refused `#if`, `#elif` or `#ifdef`, and no group after
    /// a second `#else`, is handed to the parser.
    ///
    /// Asked of the tokens rather than of a run, because any error before the
    /// parser stops the input there, so a run cannot show which group was
    /// handed over. Mutation: compile the group after a refused `#if`. `a`
    /// is handed over. Mutation: take a second `#else` as the first is
    /// taken. `e` is. Mutation: take the group of an `#ifdef` whose name is
    /// not read. `g` is.
    #[test]
    fn no_group_of_a_refused_or_doubled_conditional_is_handed_over() {
        let text = "#if 1\nint a;\n#elif 2\nint b;\n#else\nint c;\n#endif\n\
                    #ifdef U\n#else\nint d;\n#else\nint e;\n#endif\nint f;\n\
                    #ifdef A$B\nint g;\n#endif\n";
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        let handed = preprocess(file, &tokens, &mut sources, &mut diagnostics);

        let spelled: Vec<&str> = handed
            .tokens()
            .iter()
            .filter(|token| !token.is_eof())
            .map(|token| sources.snippet(token.span))
            .collect();
        assert_eq!(spelled, ["int", "d", ";", "int", "f", ";"]);
    }

    /// The code moved from the lexer with the report, and is the handle a
    /// reader kept, so it is asserted by itself here as the lexer's are there.
    #[test]
    fn a_directive_that_is_not_read_keeps_the_code_it_was_assigned() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "#pragma pack(4)\n");
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(file, &tokens, &mut sources, &mut diagnostics);

        let codes: Vec<String> = diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect();
        assert_eq!(codes, ["SC0104"]);
    }
}
