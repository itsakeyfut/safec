//! Directives, read between the lexer and the parser.
//!
//! What it reads is `#include "file"`, whose file's tokens, themselves
//! preprocessed, take the directive's place; `#define` and `#undef`, whose
//! macros' names are replaced by their replacement lists, a function-like
//! one's with its arguments written in, rescanned (C17 6.10.3); and
//! `#ifdef`, `#ifndef`, `#else` and `#endif`, which choose the groups that
//! are compiled (6.10.1). Every other directive is refused as `SC0104`,
//! `#include <file>` among them, which needs a search path and a system
//! header this compiler cannot read yet anyway. `#if` and `#elif` are read
//! too, their expressions evaluated in `condition`. `#` and `##` in a replacement list make a string literal and
//! join two tokens (6.10.3.2, 6.10.3.3), and a macro whose parameters end in
//! `...` takes the arguments past them as `__VA_ARGS__` (6.10.3 p12), with
//! GNU's `, ## __VA_ARGS__` and C23's `__VA_OPT__`.
//!
//! Each inclusion and each expansion is a file of its own in the source map,
//! so a diagnostic in a header names the header, one in an expansion is shown
//! at its `#define` and its use, and two of either never share a span. See
//! ADR-0054, and ADR-0128 for a function-like macro's call, whose arguments
//! are written into the one file its expansion is. The translation unit is preprocessed whole before it is
//! parsed, for the reason that record gives.

mod condition;

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label, Remedy};
use crate::driver::{Access, reason};
use crate::lexer::{
    UNSUPPORTED_SPLICE, is_identifier, lex, lex_replacement, unexpected_characters,
};
use crate::options::CommandLineMacro;
use crate::token::{Punct, Token, TokenKind};
use safec_ir::source::{Cut, Edge, FileId, FileName, Piece, SourceMap, Span};
use safec_ir::target::Target;

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
/// A `#define` or an `#undef` of a name the implementation predefines.
const PREDEFINED_AGAIN: Code = Code::new("SC0117");
/// A macro written or used wrong: a call with no `)`, or another number of
/// arguments than it has parameters; a `#` with no parameter after it, a
/// `##` with nothing on one side; a `##` that makes no one token. One code,
/// because the fix is the same: the macro or its call, written again.
const MALFORMED_MACRO: Code = Code::new("SC0119");

/// An `#if` or an `#elif` whose expression is not one C17 6.10.1 allows:
/// none, a token no integer constant expression holds, a constant too large,
/// or, where evaluated, a division by zero, a signed overflow or a shift too
/// far (6.6 p3, p4).
const MALFORMED_CONDITION: Code = Code::new("SC0120");

/// What a variadic macro's arguments past its named parameters are named in
/// its replacement list (C17 6.10.3 p12), and the name of its last parameter
/// in [`Parameters::names`].
const VA_ARGS: &str = "__VA_ARGS__";
/// C23's `__VA_OPT__ ( ... )`, read in a variadic macro's list (C23
/// 6.10.5.2).
const VA_OPT: &str = "__VA_OPT__";

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

/// How deep one call's arguments may hold calls, one inside the next.
///
/// Each level is read by a frame loop of its own, on the native stack, so
/// past a depth the process overflowed it with nothing said: `I(I(...I(1)))`
/// a thousand deep did, where C17 5.2.4.1 asks for 63 levels of nested
/// parentheses. Past this the innermost call is refused, once per call
/// refused. Found by review.
const MAX_ARGUMENT_DEPTH: usize = 200;

/// How many bytes the calls of one use may write into their expansions.
///
/// [`MAX_EXPANSIONS`] counts expansions, and a call copies its argument
/// once per use of its parameter, so a chain of calls each using its
/// parameter twice doubles what it copies at every level while counting a
/// handful of expansions: twenty-four such levels took 36 s and printed a
/// 27 MB diagnostic. Found by review. This sits far above what a real
/// program's macros write for one use.
const MAX_SUBSTITUTED: usize = 1 << 22;

/// How many bytes the calls of one input may write in all, for the reason
/// [`MAX_EXPANSIONS_IN_ALL`] bounds an input as well as a use.
const MAX_SUBSTITUTED_IN_ALL: usize = 1 << 26;

/// How many bytes the `##`s of one expansion may make for the scan to read.
///
/// Each join is scanned whole to ask whether it is one token, and a chain
/// joins onto what it has made so far, so `a##a##...##a` scans the square
/// of its length: 8,000 joins took 2.4 s a use, and forty uses ran past a
/// minute with nothing said. Found by review. A join real code makes is a
/// name or a number, far under this; what is scanned also counts toward
/// [`MAX_SUBSTITUTED`] and [`MAX_SUBSTITUTED_IN_ALL`] like written text.
const MAX_PASTED: usize = 1 << 16;

/// The files being read, the macros defined, and how many inclusions and
/// expansions there have been so far.
///
/// `Clone`, because a [`Prelude`] is one of these before any input is read,
/// and every input starts from a copy of it.
#[derive(Clone, Debug)]
struct Reading {
    /// Outermost first, each by its canonical path, so a cycle is found
    /// whatever spelling reached it.
    open: Vec<(PathBuf, FileId)>,
    inclusions: usize,
    /// Every object-like macro defined so far, by name.
    macros: HashMap<String, Macro>,
    expansions: usize,
    /// What the input is compiled for, which decides what is predefined.
    target: Target,
    /// When the translation started, in seconds since 1970 in UTC, which
    /// `__DATE__` and `__TIME__` write: `SOURCE_DATE_EPOCH` where it is set,
    /// the clock otherwise, read once per build; or the value of a
    /// `SOURCE_DATE_EPOCH` that is not a number of seconds. See ADR-0130.
    started: Result<u64, String>,
    /// Every expansion this input has made.
    expansions_in_all: usize,
    /// How many expansions of each name are open, and of all names: what the
    /// rule that a macro is not expanded inside itself asks of, kept as a
    /// count rather than found by walking the frames, which made a chain of
    /// distinct macros cost the square of its depth. Found by review.
    open_macros: HashMap<String, usize>,
    open_expansions: usize,
    /// Every name `-D` or `-U` named, or the source has undefined, which
    /// `#ifdef` and `#ifndef` answer whether or not the compiler might
    /// predefine it. See ADR-0059.
    settled: HashSet<String>,
    /// Where, in each file, a name sits that was not replaced because its
    /// macro was being replaced, by its file and its start. It is never
    /// replaced (C17 6.10.3.4 p2), even once copied out of an argument into
    /// a call's expansion, where its macro may no longer be open: with
    /// `#define x x+1` and `#define id(y) y`, `id(x)` is `x+1`, and
    /// forgetting the mark made it `x+1+1`. See ADR-0128.
    painted: HashSet<(FileId, u32)>,
    /// Where, in each expansion, a token sits that a space was written
    /// before only to keep it apart from what is before it, by its file and
    /// its start. C has no white space there, and `#` reads none: with
    /// `#define VERSION MAJOR.MINOR`, `XSTR(VERSION)` is `"1.2"`, where
    /// reading the space made it `"1 . 2"`. Found by review.
    unspaced: HashSet<(FileId, u32)>,
    /// How many arguments are being replaced, one inside another. A use
    /// starts a tree of its own only outside every expansion and every
    /// argument, so a call's arguments count toward the bound of the use
    /// they are in.
    arguments: usize,
    /// Bytes the calls of the use being read have written, which
    /// [`MAX_SUBSTITUTED`] bounds, and those of the whole input, which
    /// [`MAX_SUBSTITUTED_IN_ALL`] does.
    substituted: usize,
    substituted_in_all: usize,
}

/// An object-like macro.
#[derive(Clone, Debug)]
struct Macro {
    /// Its replacement list, in its `#define`: what each expansion holds.
    spelled: Span,
    /// The `#define`, for the note on a redefinition.
    defined: Span,
    /// Its replacement list's tokens, each with whether whitespace came
    /// before it: what two definitions are compared by (C17 6.10.3 p1).
    tokens: Vec<(String, bool)>,
    origin: Origin,
    /// `None` for an object-like macro.
    parameters: Option<Parameters>,
    /// What a use writes as something other than the list's own text, in
    /// order: each parameter, each `#`, each run joined by `##`. Empty for
    /// most object-like macros, whose expansion is the list byte for byte.
    edits: Vec<Edit>,
}

/// A part of a replacement list a use writes as something other than its own
/// text, by its start and end counted from the list's start.
#[derive(Clone, Debug)]
enum Edit {
    /// A parameter, written as its argument replaced (C17 6.10.3.1).
    /// `spaced`, here and below, is whether white space is before it in the
    /// list, which its first token has.
    Parameter {
        start: u32,
        end: u32,
        index: usize,
        spaced: bool,
    },
    /// `#` and a parameter, written as its argument as a string literal
    /// (6.10.3.2). `operator` is the `#`, what the literal is copied from.
    Stringized {
        start: u32,
        end: u32,
        index: usize,
        operator: (u32, u32),
        spaced: bool,
    },
    /// GNU's `, ## __VA_ARGS__`: the comma and the variable arguments as
    /// written where they were given, even empty, and nothing where they
    /// were left out. `comma` is the comma, what it is copied from. `rest`
    /// are operands joined on after `__VA_ARGS__`, as in `, ## __VA_ARGS__
    /// ## 1`, and `operator` the first `##`: where the arguments were given
    /// they join onto them, and where they were left out the run is an
    /// ordinary join of the comma and a placemarker.
    Comma {
        start: u32,
        end: u32,
        index: usize,
        comma: (u32, u32),
        rest: Vec<(Operand, bool)>,
        operator: (u32, u32),
        spaced: bool,
    },
    /// `__VA_OPT__ ( ... )`, or `#` and one: `operand` is an
    /// [`Operand::Optional`].
    Optional {
        start: u32,
        end: u32,
        operand: Operand,
        spaced: bool,
    },
    /// Operands joined by `##`, written as one text with each join's two
    /// tokens made one (6.10.3.3). `operator` is the first `##`, what the
    /// text is copied from.
    Pasted {
        start: u32,
        end: u32,
        /// Each with whether white space is before it in the list.
        operands: Vec<(Operand, bool)>,
        operator: (u32, u32),
        spaced: bool,
    },
}

/// One side of a `##`.
#[derive(Clone, Debug)]
enum Operand {
    /// A parameter: its argument as written, not replaced.
    Parameter(usize),
    /// `#` and a parameter.
    Stringized(usize),
    /// A token of the list, by its start and end.
    Token(u32, u32),
    /// `__VA_OPT__ ( content )`: the content, written as a list is, where
    /// the variable arguments, `index`, have a token once replaced, and
    /// nothing where they have none (C23 6.10.5.2). `inner` are its edits,
    /// counted from the content's start; `stringized` where `#` is before
    /// it, and `operator` is that `#` and `__VA_OPT__`, or `__VA_OPT__`.
    Optional {
        index: usize,
        content: (u32, u32),
        inner: Vec<Edit>,
        stringized: bool,
        operator: (u32, u32),
    },
}

/// `operands` counted from `by` bytes later in the list.
fn shifted_operands(operands: Vec<(Operand, bool)>, by: u32) -> Vec<(Operand, bool)> {
    operands
        .into_iter()
        .map(|(operand, spaced)| {
            let operand = match operand {
                Operand::Token(start, end) => Operand::Token(start - by, end - by),
                other => other,
            };
            (operand, spaced)
        })
        .collect()
}

impl Edit {
    /// This edit counted from `by` bytes later in the list.
    fn shifted(self, by: u32) -> Self {
        let less = |at: u32| at - by;
        match self {
            Self::Parameter {
                start,
                end,
                index,
                spaced,
            } => Self::Parameter {
                start: less(start),
                end: less(end),
                index,
                spaced,
            },
            Self::Stringized {
                start,
                end,
                index,
                operator,
                spaced,
            } => Self::Stringized {
                start: less(start),
                end: less(end),
                index,
                operator: (less(operator.0), less(operator.1)),
                spaced,
            },
            Self::Comma {
                start,
                end,
                index,
                comma,
                rest,
                operator,
                spaced,
            } => Self::Comma {
                start: less(start),
                end: less(end),
                index,
                comma: (less(comma.0), less(comma.1)),
                rest: shifted_operands(rest, by),
                operator: (less(operator.0), less(operator.1)),
                spaced,
            },
            Self::Optional { .. } => {
                unreachable!("a `__VA_OPT__` is never inside another, which `edits_in` refuses")
            }
            Self::Pasted {
                start,
                end,
                operands,
                operator,
                spaced,
            } => Self::Pasted {
                start: less(start),
                end: less(end),
                operands: shifted_operands(operands, by),
                operator: (less(operator.0), less(operator.1)),
                spaced,
            },
        }
    }

    /// Whether this edit, or one inside it, writes argument `index` replaced.
    fn replaces(&self, index: usize) -> bool {
        match self {
            Self::Parameter { index: used, .. } => *used == index,
            Self::Optional {
                operand:
                    Operand::Optional {
                        index: used, inner, ..
                    },
                ..
            } => *used == index || inner.iter().any(|edit| edit.replaces(index)),
            // The operand of `##` is written as it was (C17 6.10.3.1 p1),
            // and so is GNU's comma's: replaced first, `F(0, F(0, 5))` with
            // `#define F(a, ...) f(a, ## __VA_ARGS__)` called the inner `F`
            // where it is the function. Found by review.
            Self::Optional { .. }
            | Self::Stringized { .. }
            | Self::Comma { .. }
            | Self::Pasted { .. } => false,
        }
    }

    /// Whether this edit is GNU's `, ## __VA_ARGS__`.
    fn is_comma(&self) -> bool {
        matches!(self, Self::Comma { .. })
    }
}

/// A function-like macro's parameters, and where its replacement list uses
/// them.
#[derive(Clone, Debug)]
struct Parameters {
    /// In order, a variadic macro's last named [`VA_ARGS`]. Compared, with
    /// the tokens, by a second definition (C17 6.10.3 p2).
    names: Vec<String>,
}

impl Parameters {
    /// Whether `...` ends the parameters.
    fn variadic(&self) -> bool {
        self.names.last().is_some_and(|name| name == VA_ARGS)
    }
}

/// What a name the frame loop meets becomes.
enum Replaced {
    /// An expansion, read in the name's place.
    Frame(Frame),
    /// The name itself, handed on.
    Kept,
    /// Nothing: a call refused, whose name and arguments are dropped with the
    /// report already made.
    Dropped,
}

/// What defined a macro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// A `#define` in the source.
    Written,
    /// The implementation, in `<built-in>`: see [`predefined`].
    Predefined,
    /// The implementation, with a value that is the place or the time it is
    /// used: one of [`DYNAMIC`], which `#ifdef` answers and nothing expands
    /// yet.
    Dynamic,
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
/// Only [`preprocess_with`] builds one, which [`preprocess`] calls for a
/// caller with one input, since the field is private, and
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

/// What every input of one build starts from: the macros the target
/// predefines and what the command line said, read once.
///
/// Each input is preprocessed from a copy, so nothing one input defines
/// reaches the next, and a refusal of a `-D` or `-U` is the build's, reported
/// once rather than once per input.
#[derive(Debug)]
pub struct Prelude {
    reading: Reading,
}

/// Read the macros `target` predefines, and then what `macros`, the command
/// line's `-D` and `-U`, says, one after another in the order of the list, into
/// the state every input of a build starts from.
///
/// # Panics
///
/// If a name in `macros` is not an ASCII identifier, or a value holds a line
/// break. `Cli::into_options` refuses both, so only a caller that built the
/// list by hand can get here, and its list would otherwise be read as other
/// directives than it says.
pub fn prelude(
    target: Target,
    macros: &[CommandLineMacro],
    sources: &mut SourceMap,
    diagnostics: &mut DiagnosticSink,
) -> Prelude {
    let mut reading = Reading {
        open: Vec::new(),
        inclusions: 0,
        macros: HashMap::new(),
        expansions: 0,
        target,
        expansions_in_all: 0,
        open_macros: HashMap::new(),
        open_expansions: 0,
        settled: macros.iter().map(|given| given.name().to_owned()).collect(),
        started: started(),
        painted: HashSet::new(),
        unspaced: HashSet::new(),
        arguments: 0,
        substituted: 0,
        substituted_in_all: 0,
    };
    // Ordinary `#define` lines in a file of their own, so an expansion of one
    // is placed like any other (ADR-0054) and a label in it names
    // `<built-in>`.
    let text = predefined(target);
    let built_in = sources.add_virtual("built-in", text);
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let directive = line.trim_end_matches('\n');
        let span = Span::new(built_in, at, at + directive.len() as u32);
        let MacroForm::Define {
            name,
            name_at,
            body,
            parameters,
        } = macro_form(directive)
        else {
            panic!("a predefined line that is not a `#define`: {directive}");
        };
        let origin = Origin::Predefined;
        define(
            directive,
            span,
            name,
            name_at,
            body,
            parameters,
            origin,
            sources,
            &mut reading,
            diagnostics,
        );
        at += line.len() as u32;
    }
    let nowhere = Span::new(built_in, 0, 0);
    for name in DYNAMIC {
        reading.macros.insert(
            name.to_owned(),
            Macro {
                spelled: nowhere,
                defined: nowhere,
                tokens: Vec::new(),
                origin: Origin::Dynamic,
                parameters: None,
                edits: Vec::new(),
            },
        );
    }
    command_line(macros, sources, &mut reading, diagnostics);
    // Refused once for the build, used or not, as `clang` refuses it.
    if let Err(value) = &reading.started {
        diagnostics.report(
            Diagnostic::error(format!(
                "`SOURCE_DATE_EPOCH` is `{value}`, which is not a number of seconds from 0 to \
                 {LAST_SECOND}"
            ))
            .with_note("`__DATE__` and `__TIME__` are written from it"),
        );
    }
    Prelude { reading }
}

/// Splice every `#include "file"` in `tokens`, the scan of `file`, record
/// every `#define` and `#undef` and expand what they define, and report every
/// other directive, starting from a copy of `prelude`.
///
/// The end-of-file token is the input's own: an included file's is dropped,
/// since the parser reads the first one it meets as the end.
pub fn preprocess_with(
    file: FileId,
    tokens: &[Token],
    prelude: &Prelude,
    sources: &mut SourceMap,
    diagnostics: &mut DiagnosticSink,
) -> Preprocessed {
    let mut reading = prelude.reading.clone();
    if let Some(path) = path_of(sources.file(file).name()) {
        reading.open.push((canonical(path), file));
    }
    let mut spliced = Vec::with_capacity(tokens.len());
    read_includes(tokens, sources, &mut reading, diagnostics, &mut spliced);
    spliced.extend(tokens.iter().copied().filter(|token| token.is_eof()));
    Preprocessed(spliced)
}

/// [`prelude`] and then [`preprocess_with`], for a caller with one input.
///
/// A build with several reads the prelude once and preprocesses each input
/// from it, which is what the driver does.
///
/// # Panics
///
/// As [`prelude`] does.
pub fn preprocess(
    file: FileId,
    tokens: &[Token],
    target: Target,
    macros: &[CommandLineMacro],
    sources: &mut SourceMap,
    diagnostics: &mut DiagnosticSink,
) -> Preprocessed {
    let prelude = prelude(target, macros, sources, diagnostics);
    preprocess_with(file, tokens, &prelude, sources, diagnostics)
}

/// Read what `-D` and `-U` said as the lines of a file of their own, `command
/// line`, through the same path as a `#define` or an `#undef` in the source.
/// So a refusal of one points at `<command line>`, as one of a predefinition
/// points at `<built-in>`, and a `-D` meets a later `#define` of its name as
/// two `#define`s meet.
///
/// Two things the source may do and the command line may not, each refused as
/// `SC0117` with its line not read. See ADR-0059:
///
/// - define or undefine a name C17 6.10.8 lists, whether or not this target
///   predefines it. Asked of the name rather than left to the `#define` and
///   `#undef` paths, which ask only of a name already defined: on MSVC
///   `__STDC__` is not, so `-U __STDC__` settled it and a double free under
///   `#ifdef __STDC__` was analysed away. Found by review;
/// - undefine another name the target predefines, such as `_WIN32` for MSVC.
///   `cc` accepts that, and nothing here needs it yet.
fn command_line(
    macros: &[CommandLineMacro],
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) {
    if macros.is_empty() {
        return;
    }
    let mut text = String::new();
    // Each line not to be read: where it starts, the macro, and where its name
    // starts in the line.
    let mut not_read: Vec<(u32, &CommandLineMacro, u32)> = Vec::new();
    for given in macros {
        let name = given.name();
        assert!(
            is_identifier(name),
            "a command-line macro's name is not an ASCII identifier: {name:?}"
        );
        let start = text.len() as u32;
        let refused = standard(name)
            || matches!(given, CommandLineMacro::Undefined { .. })
                && reading
                    .macros
                    .get(name)
                    .is_some_and(|earlier| earlier.origin == Origin::Predefined);
        match given {
            CommandLineMacro::Defined { value, .. } => {
                assert!(
                    !value.contains(['\n', '\r']),
                    "a command-line macro's value holds a line break: {value:?}"
                );
                if refused {
                    not_read.push((start, given, "#define ".len() as u32));
                }
                text.push_str(&format!("#define {name} {value}"));
            }
            CommandLineMacro::Undefined { .. } => {
                if refused {
                    not_read.push((start, given, "#undef ".len() as u32));
                }
                text.push_str(&format!("#undef {name}"));
            }
        }
        text.push('\n');
    }

    let file = sources.add_virtual("command line", &text);
    for (start, given, offset) in &not_read {
        let name = given.name();
        let named = Span::new(file, start + offset, start + offset + name.len() as u32);
        let what = match given {
            CommandLineMacro::Defined { .. } => "defined again here",
            CommandLineMacro::Undefined { .. } => "undefined here",
        };
        let report = match reading.macros.get(name) {
            Some(earlier) if standard(name) => predefined_again(name, named, what, earlier),
            Some(earlier) => {
                Diagnostic::error(format!("`{name}` is defined by the implementation"))
                    .with_code(PREDEFINED_AGAIN)
                    .with_label(Label::primary(named, what))
                    .with_label(Label::secondary(earlier.defined, "predefined here"))
                    .with_note(
                        "this target predefines it, and the command line cannot undefine what the \
                     target predefines",
                    )
            }
            None => Diagnostic::error(format!("`{name}` is the implementation's to define"))
                .with_code(PREDEFINED_AGAIN)
                .with_label(Label::primary(
                    named,
                    match given {
                        CommandLineMacro::Defined { .. } => "defined here",
                        CommandLineMacro::Undefined { .. } => "undefined here",
                    },
                ))
                .with_note(
                    "C17 6.10.8 lists it, and the command line cannot settle what that \
                     subclause leaves to the implementation",
                ),
        };
        diagnostics.report(report);
    }
    let scan: Vec<Token> = lex(file, sources.file(file), diagnostics)
        .into_iter()
        .filter(|token| {
            !not_read
                .iter()
                .any(|(start, _, _)| token.span.start() == *start)
        })
        .collect();
    let mut handed_over = Vec::new();
    read_includes(&scan, sources, reading, diagnostics, &mut handed_over);
    // Every line is a directive, so nothing reaches the parser from here.
    assert!(
        handed_over.is_empty(),
        "the command line handed the parser tokens: {text:?}"
    );
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
        leading: false,
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
                closed(&instance, reading);
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
                    // A conditional refused here still opens or ends its
                    // level, as `conditional` says a refused one does: left
                    // out, its `#endif` was reported as closing nothing, a
                    // caret on a directive that was right. Found by review.
                    if let (Some(form), Some(frame)) = (conditional_form(&text), frames.last_mut())
                    {
                        conditional(
                            form,
                            &text,
                            token.span,
                            frame,
                            (sources, reading),
                            diagnostics,
                            true,
                        );
                    }
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
                        conditional(
                            form,
                            &text,
                            token.span,
                            frame,
                            (sources, reading),
                            diagnostics,
                            false,
                        );
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
                                leading: false,
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
                            parameters,
                        } => define(
                            &text,
                            token.span,
                            name,
                            name_at,
                            body,
                            parameters,
                            Origin::Written,
                            sources,
                            reading,
                            diagnostics,
                        ),
                        MacroForm::Undef { name } => match reading.macros.get(name) {
                            Some(earlier)
                                if earlier.origin != Origin::Written && standard(name) =>
                            {
                                // The name is the last thing on the line,
                                // since an `#undef` with more is not read.
                                let start = token.span.start() + text.trim_end().len() as u32
                                    - name.len() as u32;
                                let named =
                                    Span::new(token.span.file(), start, start + name.len() as u32);
                                diagnostics.report(predefined_again(
                                    name,
                                    named,
                                    "undefined here",
                                    earlier,
                                ));
                            }
                            // Undefined, the name is undefined to every
                            // compiler from here, whatever it predefines, so
                            // `#ifdef` and code answer it. Left unsettled,
                            // `#undef unix` then `int unix;` was refused.
                            // Found by review. See ADR-0055.
                            _ => {
                                reading.macros.remove(name);
                                reading.settled.insert(name.to_owned());
                            }
                        },
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
                        MacroForm::Parameters => diagnostics.report(
                            refused(
                                &text,
                                token.span,
                                UNSUPPORTED_DIRECTIVE,
                                "this parameter list is not read",
                            )
                            .with_note(
                                "a function-like macro's parameters are read when each is a \
                                 name of ASCII letters, digits and `_`, none named twice, \
                                 between `(` and `)`",
                            ),
                        ),
                        MacroForm::Variadic => diagnostics.report(
                            refused(
                                &text,
                                token.span,
                                UNSUPPORTED_DIRECTIVE,
                                "this variadic parameter list is not read",
                            )
                            .with_note(
                                "`...` is read as the last parameter; a named one such as \
                                 `args...` is not read yet",
                            ),
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
                                "`#include \"...\"`, `#define` and `#undef`, `#ifdef`, \
                                 `#ifndef`, `#else` and `#endif` are read",
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
                match replacement(&mut frames, token, sources, reading, diagnostics) {
                    Replaced::Frame(frame) => frames.push(frame),
                    Replaced::Kept => {
                        emitted(&mut frames, token.span);
                        spliced.push(token);
                    }
                    Replaced::Dropped => {}
                }
            }
            // Reported here, where it is compiled, and not where it was
            // scanned: each character no token can begin is a preprocessing
            // token, and the arm above has already passed over them in a
            // skipped group, where C ignores them (C17 6.10.1 p6). Handed on
            // all the same, so the stream still accounts for its text.
            TokenKind::Unknown => {
                if let Some(diagnostic) =
                    unexpected_characters(token.span, sources.snippet(token.span))
                {
                    diagnostics.report(diagnostic);
                }
                emitted(&mut frames, token.span);
                spliced.push(token);
            }
            _ => {
                emitted(&mut frames, token.span);
                spliced.push(token);
            }
        }
    }
}

/// What `token`, a name the innermost of `frames` was reading, becomes.
///
/// A keyword or an annotation is a name here too, since preprocessing knows
/// only identifiers (C17 6.4 p1), so `#define int long` and `#define
/// _Nonnull` are macros. A name is kept where an expansion of it is open,
/// which is the rule that a macro is not replaced inside its own replacement
/// (6.10.3.4 p2): a rescan reads only its replacement, since a call whose
/// `(` lies past it is refused, so the open expansions are exactly the
/// replacements in progress. Such a name is marked, and kept wherever it is
/// copied to after, as that rule says. See [`Reading::painted`].
///
/// Mutation: drop the open-expansion check. `#define X X + 1` expands for
/// ever, and the bound refuses it.
fn replacement(
    frames: &mut [Frame],
    token: Token,
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> Replaced {
    let leading = frames
        .last()
        .is_some_and(|frame| spaced(frame, frame.cursor - 1, reading));
    let name = sources.snippet(token.span);
    // Each is read only where a variadic macro's list names it, and there
    // it is replaced before anything is read (C17 6.10.3 p5, C23 6.10.5.2).
    if name == VA_ARGS || name == VA_OPT {
        diagnostics.report(
            Diagnostic::error(format!(
                "`{name}` is read only in a variadic macro's replacement list"
            ))
            .with_code(MALFORMED_MACRO)
            .with_label(Label::primary(token.span, "written here")),
        );
        return Replaced::Dropped;
    }
    let Some(defined) = reading.macros.get(name) else {
        // A name one compiler for the target predefines and another does
        // not has no one meaning in code either: refused as `#ifdef`
        // refuses it, where it was otherwise undeclared to one build and a
        // value to the other. Only the unsettled names: a reserved name no
        // target predefines, `__func__` or a builtin's, is code's own.
        // See ADR-0055.
        if !reading.settled.contains(name) && unsettled(reading.target).contains(&name) {
            diagnostics.report(
                Diagnostic::error(format!("`{name}` is a name the implementation may define"))
                    .with_code(UNSUPPORTED_DIRECTIVE)
                    .with_label(Label::primary(token.span, "used here"))
                    .with_note(format!(
                        "`{name}` is defined in some builds for this target and not in others"
                    ))
                    .with_remedy(Remedy::new(format!(
                        "say whether the build defines it: `-D {name}` if it does, `-U {name}` if \
                     it does not"
                    ))),
            );
            return Replaced::Dropped;
        }
        return Replaced::Kept;
    };
    if defined.origin == Origin::Dynamic {
        return dynamic(token, leading, sources, reading, diagnostics)
            .map_or(Replaced::Dropped, Replaced::Frame);
    }
    let at = (token.span.file(), token.span.start());
    if reading.painted.contains(&at) {
        return Replaced::Kept;
    }
    if reading.open_macros.get(name).is_some_and(|open| *open > 0) {
        reading.painted.insert(at);
        return Replaced::Kept;
    }
    match defined.parameters.clone() {
        Some(parameters) => {
            let edits = defined.edits.clone();
            call(
                frames,
                (token, leading),
                (parameters, edits),
                sources,
                reading,
                diagnostics,
            )
        }
        None => expansion((token, leading), sources, reading, diagnostics)
            .map_or(Replaced::Kept, Replaced::Frame),
    }
}

/// The frame `token`, the name of an object-like macro whose expansion is
/// not open, expands to, or `None` where it is refused and left as it is.
fn expansion(
    (token, leading): (Token, bool),
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> Option<Frame> {
    let name = sources.snippet(token.span);
    let defined = reading.macros.get(name)?;
    let spelled = defined.spelled;
    let edits = defined.edits.clone();
    let name = name.to_owned();
    begin_use(reading);
    // A list with `##` in it is written out, its joins made; any other is
    // copied as it is.
    if !edits.is_empty() {
        // No argument to copy, so nothing it writes is marked, and it
        // writes no more than its list; what its joins scanned is what the
        // bounds on written text count.
        let written = match substituted(spelled, &edits, (&[], &[], false), sources, reading) {
            Ok(written) => written,
            Err(unwritten) => {
                diagnostics.report(unwritten_report(token, unwritten));
                return None;
            }
        };
        if !written_within_bounds(token, written.scanned, reading, diagnostics)
            || !admitted(token, &name, reading, diagnostics)
        {
            return None;
        }
        let file = sources.substitute(written.text, written.pieces, token.span);
        reading
            .unspaced
            .extend(written.unspaced.into_iter().map(|offset| (file, offset)));
        return Some(instance(
            file,
            (token.span, leading),
            name,
            sources,
            diagnostics,
        ));
    }
    if !admitted(token, &name, reading, diagnostics) {
        return None;
    }
    let file = sources.expand(spelled, token.span);
    Some(instance(
        file,
        (token.span, leading),
        name,
        sources,
        diagnostics,
    ))
}

/// The report for an expansion at `token` that `substituted` did not write.
fn unwritten_report(token: Token, unwritten: Unwritten) -> Diagnostic {
    match unwritten {
        Unwritten::Unpasted(pasted) => {
            Diagnostic::error(format!("`##` makes `{pasted}`, which is not one token"))
                .with_code(MALFORMED_MACRO)
                .with_label(Label::primary(token.span, "expanded here"))
                .with_note("C17 6.10.3.3 p3 leaves a join that makes no one token undefined")
        }
        Unwritten::TooLong => Diagnostic::error("this expansion joins too much with `##`")
            .with_code(TOO_MANY_EXPANSIONS)
            .with_label(Label::primary(token.span, "expanded here"))
            .with_note(format!(
                "its joins made more than {MAX_PASTED} bytes of text to read"
            )),
    }
}

/// The expansion of `token`, one of [`DYNAMIC`]: the place or the time of
/// its use, written as `clang` writes it (ADR-0130). Its one piece is copied
/// from the use, which is where a report in it points.
fn dynamic(
    token: Token,
    leading: bool,
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> Option<Frame> {
    let name = sources.snippet(token.span).to_owned();
    let text = match name.as_str() {
        "__LINE__" => {
            let at = used_at(token.span, sources);
            (sources.file(at.file()).line_index(at.end()) + 1).to_string()
        }
        "__FILE__" => {
            let at = used_at(token.span, sources);
            quoted(&presumed(at.file(), sources))
        }
        _ => {
            // An invalid one was refused for the build, in `prelude`.
            let Ok(seconds) = reading.started else {
                return None;
            };
            let (date, time) = civil(seconds);
            if name == "__DATE__" { date } else { time }
        }
    };
    begin_use(reading);
    if !admitted(token, &name, reading, diagnostics) {
        return None;
    }
    let pieces = vec![Piece {
        offset: 0,
        from: token.span,
    }];
    let file = sources.substitute(text, pieces, token.span);
    Some(instance(
        file,
        (token.span, leading),
        name,
        sources,
        diagnostics,
    ))
}

/// Where `span`, a token's, is used, as `__LINE__` and `__FILE__` say it
/// (measured with `clang 22.1.8 -E`): a token of an expansion where the
/// outermost use of its macro ends, the `)` of a call over two lines, even
/// one a `##` copied from an argument. One written in an argument is
/// replaced where it was written, before any expansion holds it, and so is
/// its own place.
fn used_at(mut span: Span, sources: &SourceMap) -> Span {
    while let FileName::Expansion { at, .. } = sources.file(span.file()).name() {
        span = Span::new(at.file(), at.end(), at.end());
    }
    span
}

/// `name` as a string literal: a `\\` before each `\\` and `"` in it.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The name `__FILE__` gives `file`, as `clang 22.1.8 -E` writes it: an
/// input as it was named, and an included file as the directory of its
/// includer's name, `.` where that has none, joined to the name the
/// `#include` wrote by the host's separator: `.\sub/h.h` on Windows for
/// `#include "sub/h.h"` in `inc.c`. A diagnostic names it otherwise, the
/// same on every host. See ADR-0131.
fn presumed(file: FileId, sources: &SourceMap) -> String {
    match sources.file(file).name() {
        FileName::Inclusion { at, .. } => {
            let includer = presumed(at.file(), sources);
            let directive = sources.snippet(*at);
            let written = directive
                .split('"')
                .nth(1)
                .expect("an inclusion is read from a quoted `#include`");
            let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
            let directory = includer
                .rfind(separators)
                .map_or(".", |end| &includer[..end]);
            let separator = if cfg!(windows) { '\\' } else { '/' };
            format!("{directory}{separator}{written}")
        }
        name => name.to_string(),
    }
}

/// When the translation started, for [`Reading::started`].
fn started() -> Result<u64, String> {
    started_from(std::env::var("SOURCE_DATE_EPOCH").ok())
}

/// The last second whose `__DATE__` has a four-digit year, 9999-12-31
/// 23:59:59, past which `"Mmm dd yyyy"` (C17 6.10.8.1) has no spelling; the
/// bound `clang` puts on `SOURCE_DATE_EPOCH` too.
const LAST_SECOND: u64 = 253_402_300_799;

/// [`started`], given what `SOURCE_DATE_EPOCH` is set to, if it is: decimal
/// digits alone, no sign and no white space, up to [`LAST_SECOND`], as
/// `clang` reads it. Read more loosely, `" 42"` and a year past 9999 were
/// dates `clang` refuses. Found by review.
fn started_from(epoch: Option<String>) -> Result<u64, String> {
    match epoch {
        Some(value) => Some(&value)
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| *seconds <= LAST_SECOND)
            .ok_or(value),
        None => Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())),
    }
}

/// `__DATE__` and `__TIME__` for `seconds` since 1970 in UTC: `"Mmm dd
/// yyyy"`, the day padded with a space, and `"hh:mm:ss"` (C17 6.10.8.1).
///
/// The civil date of a day count, by the proleptic Gregorian calendar's
/// 400-year cycle of 146,097 days, counted from 1 March so a leap day ends
/// its year.
fn civil(seconds: u64) -> (String, String) {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = seconds / 86_400;
    let of_day = seconds % 86_400;
    // Days since 0000-03-01, 719,468 of them before 1970-01-01.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let of_era = shifted % 146_097;
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    let date = format!("\"{} {day:>2} {year}\"", MONTHS[month as usize - 1]);
    let time = format!(
        "\"{:02}:{:02}:{:02}\"",
        of_day / 3_600,
        of_day / 60 % 60,
        of_day % 60
    );
    (date, time)
}

/// Start a tree of expansions for a use written outside any expansion and
/// any argument, which [`MAX_EXPANSIONS`] bounds on its own.
fn begin_use(reading: &mut Reading) {
    if reading.open_expansions == 0 && reading.arguments == 0 {
        reading.expansions = 0;
        reading.substituted = 0;
    }
}

/// Count an expansion of `name`, used at `token`, and open it; or refuse it
/// past a bound, said once.
fn admitted(
    token: Token,
    name: &str,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> bool {
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
        return false;
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
        return false;
    }
    reading.expansions += 1;
    reading.expansions_in_all += 1;
    *reading.open_macros.entry(name.to_owned()).or_insert(0) += 1;
    reading.open_expansions += 1;
    true
}

/// The frame that reads `file`, an expansion of `name` used at `at`.
fn instance(
    file: FileId,
    (at, leading): (Span, bool),
    name: String,
    sources: &SourceMap,
    diagnostics: &mut DiagnosticSink,
) -> Frame {
    // Lexed here, where it is used, so a character no token can begin is
    // reported at each use, which is where C makes it an error. That is
    // `read_includes`' to do for every frame, this one included.
    let tokens = lex_replacement(file, &sources.file_owned(file), diagnostics);
    Frame {
        tokens,
        cursor: 0,
        instance: Some(Instance::Expansion { file, at, name }),
        extent: None,
        conditionals: Vec::new(),
        skipped_depth: 0,
        leading,
    }
}

/// Close what `instance` opened, now that its frame has ended.
fn closed(instance: &Instance, reading: &mut Reading) {
    match instance {
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
}

/// A call of the function-like macro `token` names, whose expansion is not
/// open: its arguments read out of the innermost frame, each replaced on its
/// own (C17 6.10.3.1 p1), and written into its replacement list as one
/// expansion, used at the whole call. See ADR-0128.
///
/// `Kept` where no `(` follows the name, since a name with none is not a
/// call (6.10.3 p10).
fn call(
    frames: &mut [Frame],
    (token, leading): (Token, bool),
    (parameters, edits): (Parameters, Vec<Edit>),
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> Replaced {
    let name = sources.snippet(token.span).to_owned();
    let Some((top, below)) = frames.split_last_mut() else {
        return Replaced::Kept;
    };
    let next = |frame: &Frame| {
        frame
            .tokens
            .get(frame.cursor)
            .copied()
            .filter(|next| !next.is_eof())
    };
    let opens = |token: Token| token.kind == TokenKind::Punct(Punct::LeftParen);
    match next(top) {
        Some(after) if opens(after) => {}
        Some(_) => return Replaced::Kept,
        None => {
            // The name ends the file it is read from, and C reads on into
            // what that file was used in for its `(`. A call so spread over
            // two files has no one span for its use (ADR-0128), so it is
            // refused; read as no call it would compile something else.
            let after = below.iter().rev().find_map(next);
            if after.is_some_and(opens) {
                diagnostics.report(
                    Diagnostic::error(
                        "a call whose name ends an expansion or a file is not read yet",
                    )
                    .with_code(UNSUPPORTED_DIRECTIVE)
                    .with_label(Label::primary(token.span, "its name is here"))
                    .with_note(format!(
                        "the `(` after `{name}` is outside what its name was read from"
                    )),
                );
                return Replaced::Dropped;
            }
            return Replaced::Kept;
        }
    }

    // Collected to the `)` that closes the call, split at each comma outside
    // any parentheses (6.10.3 p11).
    // Each token with whether C has white space before it, which `#` reads.
    let mut arguments: Vec<Vec<(Token, bool)>> = vec![Vec::new()];
    let mut depth = 0_usize;
    let mut close = None;
    let mut directive = None;
    let mut at = top.cursor + 1;
    while let Some(&argument) = top.tokens.get(at) {
        if argument.is_eof() {
            break;
        }
        let spaced = spaced(top, at, reading);
        at += 1;
        match argument.kind {
            TokenKind::Punct(Punct::LeftParen) => depth += 1,
            TokenKind::Punct(Punct::RightParen) if depth == 0 => {
                close = Some(argument);
                break;
            }
            TokenKind::Punct(Punct::RightParen) => depth -= 1,
            // A variadic macro's arguments past its named parameters are
            // one, the commas between them in it (C17 6.10.3.1 p2).
            TokenKind::Punct(Punct::Comma)
                if depth == 0
                    && !(parameters.variadic() && arguments.len() == parameters.names.len()) =>
            {
                arguments.push(Vec::new());
                continue;
            }
            TokenKind::Directive => {
                directive.get_or_insert(argument);
            }
            _ => {}
        }
        if let Some(last) = arguments.last_mut() {
            last.push((argument, spaced));
        }
    }
    top.cursor = at;

    let Some(close) = close else {
        // Ending an expansion or an included file, C reads on past it for
        // the `)` (C17 6.10.3.4 p1), and the call then has no one span for
        // its use, as one whose `(` lies past it has none. Only the input
        // ending is a call that is never closed.
        let report = if top.instance.is_some() {
            Diagnostic::error(
                "a call whose arguments run past an expansion or a file is not read yet",
            )
            .with_code(UNSUPPORTED_DIRECTIVE)
            .with_label(Label::primary(token.span, "called here"))
            .with_note(format!(
                "the `)` of `{name}` is outside what its name was read from"
            ))
        } else {
            Diagnostic::error(format!("this call of `{name}` has no `)`"))
                .with_code(MALFORMED_MACRO)
                .with_label(Label::primary(token.span, "called here"))
                .with_note("its arguments run on to the end of the file")
        };
        diagnostics.report(report);
        return Replaced::Dropped;
    };
    // C17 6.10.3 p11 leaves a directive there undefined. Read as a directive
    // it would be one the call's arguments also hold; read as tokens it would
    // be compiled as code.
    if let Some(directive) = directive {
        diagnostics.report(
            Diagnostic::error("a directive inside a macro's arguments is not read")
                .with_code(UNSUPPORTED_DIRECTIVE)
                .with_label(Label::primary(
                    directive.span,
                    "inside the arguments of this call",
                ))
                .with_label(Label::secondary(token.span, "called here"))
                .with_note("C17 6.10.3 p11 leaves what it means undefined"),
        );
        return Replaced::Dropped;
    }
    // `F()` for a macro of no parameters is a call of none.
    if parameters.names.is_empty() && arguments.len() == 1 && arguments[0].is_empty() {
        arguments.clear();
    }
    // A variadic macro given only its named arguments has its variable ones
    // left out, which `, ## __VA_ARGS__` asks of; given them empty, they
    // are there and empty. Both are what `clang` and GCC's manual do.
    let named = parameters.names.len() - usize::from(parameters.variadic());
    let mut omitted = false;
    if parameters.variadic() && arguments.len() == named {
        arguments.push(Vec::new());
        omitted = true;
    }
    // `F()` for `#define F(...) f(a, ## __VA_ARGS__)` leaves them out in
    // GNU's mode and passes them empty in C's, so the comma is there in one
    // build and not the other. A mode is part of the build, as ADR-0055
    // says, and this is refused rather than answered for one.
    if parameters.variadic()
        && named == 0
        && arguments.first().is_some_and(Vec::is_empty)
        && edits.iter().any(Edit::is_comma)
    {
        diagnostics.report(
            Diagnostic::error(format!(
                "this call of `{name}` keeps the comma before `__VA_ARGS__` in one C mode and not \
                 the other"
            ))
            .with_code(UNSUPPORTED_DIRECTIVE)
            .with_label(Label::primary(token.span, "called here"))
            .with_note(
                "a macro whose only parameter is `...`, called with nothing, has its variable \
                 arguments left out under GNU's mode and empty under C's",
            ),
        );
        return Replaced::Dropped;
    }
    if arguments.len() != parameters.names.len() {
        let count = |n: usize| {
            if n == 1 {
                "1 argument".to_owned()
            } else {
                format!("{n} arguments")
            }
        };
        let at_least = if parameters.variadic() {
            "at least "
        } else {
            ""
        };
        diagnostics.report(
            Diagnostic::error(format!(
                "`{name}` takes {at_least}{}, and is given {}",
                count(named),
                arguments.len()
            ))
            .with_code(MALFORMED_MACRO)
            .with_label(Label::primary(
                Span::new(token.span.file(), token.span.start(), close.span.end()),
                "called here",
            )),
        );
        return Replaced::Dropped;
    }

    begin_use(reading);
    if reading.arguments >= MAX_ARGUMENT_DEPTH {
        diagnostics.report(
            Diagnostic::error("this call is nested too deep in other calls' arguments")
                .with_code(TOO_MANY_EXPANSIONS)
                .with_label(Label::primary(token.span, "called here"))
                .with_note(format!(
                    "{MAX_ARGUMENT_DEPTH} calls' arguments are already being read around it"
                )),
        );
        return Replaced::Dropped;
    }
    // Only an argument the body substitutes is replaced: C replaces one before
    // substituting it (C17 6.10.3.1 p1), and one never substituted, or only
    // beside `#` or `##`, is never replaced, so `#define K(x) 7` used as
    // `K(H(1, 2))` asks nothing of `H`. Replacing every one refused that.
    // Found by review.
    let substituted_at = |index: usize| edits.iter().any(|edit| edit.replaces(index));
    let replaced_arguments: Vec<Vec<(Token, bool)>> = arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            if substituted_at(index) {
                let leading = argument.first().is_some_and(|&(_, leading)| leading);
                let tokens = argument.iter().map(|&(token, _)| token).collect();
                replaced(tokens, leading, sources, reading, diagnostics)
            } else {
                Vec::new()
            }
        })
        .collect();
    let Some(spelled) = reading.macros.get(&name).map(|defined| defined.spelled) else {
        unreachable!(
            "`{name}` was a macro when its call began and nothing in an argument can undefine it"
        );
    };
    let written = substituted(
        spelled,
        &edits,
        (&replaced_arguments, &arguments, omitted),
        sources,
        reading,
    );
    let written = match written {
        Ok(written) => written,
        Err(unwritten) => {
            diagnostics.report(unwritten_report(token, unwritten));
            return Replaced::Dropped;
        }
    };
    if !written_within_bounds(
        token,
        written.text.len() + written.scanned,
        reading,
        diagnostics,
    ) || !admitted(token, &name, reading, diagnostics)
    {
        return Replaced::Dropped;
    }
    let at = Span::new(token.span.file(), token.span.start(), close.span.end());
    let file = sources.substitute(written.text, written.pieces, at);
    reading
        .painted
        .extend(written.painted.into_iter().map(|offset| (file, offset)));
    reading
        .unspaced
        .extend(written.unspaced.into_iter().map(|offset| (file, offset)));
    Replaced::Frame(instance(file, (at, leading), name, sources, diagnostics))
}

/// Count `length` bytes a call at `token` writes into its expansion, or
/// refuse it past [`MAX_SUBSTITUTED`] or [`MAX_SUBSTITUTED_IN_ALL`], each
/// said once.
fn written_within_bounds(
    token: Token,
    length: usize,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> bool {
    let use_was_open = reading.substituted <= MAX_SUBSTITUTED;
    let input_was_open = reading.substituted_in_all <= MAX_SUBSTITUTED_IN_ALL;
    reading.substituted = reading.substituted.saturating_add(length);
    reading.substituted_in_all = reading.substituted_in_all.saturating_add(length);
    let (bound, of) = if reading.substituted_in_all > MAX_SUBSTITUTED_IN_ALL {
        if !input_was_open {
            return false;
        }
        (MAX_SUBSTITUTED_IN_ALL, "this input")
    } else if reading.substituted > MAX_SUBSTITUTED {
        if !use_was_open {
            return false;
        }
        (MAX_SUBSTITUTED, "the use this is in")
    } else {
        return true;
    };
    diagnostics.report(
        Diagnostic::error("this call writes too much into its expansion")
            .with_code(TOO_MANY_EXPANSIONS)
            .with_label(Label::primary(token.span, "called here"))
            .with_note(format!(
                "more than {bound} bytes are written into the expansions of {of}"
            )),
    );
    false
}

/// An argument's tokens, each with whether C has white space before it.
type Argument = Vec<(Token, bool)>;

/// What `substituted` writes for an expansion.
#[derive(Default)]
struct Written {
    text: String,
    /// Where each run of `text` was copied from. See ADR-0128.
    pieces: Vec<Piece>,
    /// Where in `text` a name sits that is never replaced. See
    /// [`Reading::painted`].
    painted: Vec<u32>,
    /// Where in `text` a token sits that a space was written before only to
    /// keep it apart from what is before it. See [`Reading::unspaced`].
    unspaced: Vec<u32>,
    /// How many bytes its joins made for the scan to read.
    scanned: usize,
}

/// Why `substituted` wrote nothing.
enum Unwritten {
    /// A `##` made this, which is not one token.
    Unpasted(String),
    /// The `##`s of one expansion made more than [`MAX_PASTED`] bytes of
    /// text to scan, each join scanning all it had made so far.
    TooLong,
}

/// The text of an expansion: the replacement list at `spelled`, each of its
/// `edits` written in: a parameter as its argument's tokens, already
/// replaced (`arguments.0`); `#` and one as that argument as written, a
/// string literal (`arguments.1`); a run joined by `##` with each join's two
/// tokens made one. Each argument token comes with whether C has white space
/// before it.
///
/// The list's runs are copied byte for byte. A token written where C has
/// white space before it gets a space; one where C has none is written
/// touching what is before it only where both came from one file side by
/// side, and otherwise gets a space too, so it cannot run into what is
/// before it (`-x` with `-1` for `x` is `- -1`, where `--1` would be
/// another token), and that space is recorded as no white space, which is
/// what `#` reads (C17 6.10.3.2 p2): `xstr(INCFILE(2).h)` is `"vers2.h"`.
/// What `#` and `##` make is copied from the operator in the `#define`, as
/// ADR-0128 says, and an argument token a join copies unchanged from where
/// it was written.
fn substituted(
    spelled: Span,
    edits: &[Edit],
    arguments: (&[Argument], &[Argument], bool),
    sources: &SourceMap,
    reading: &Reading,
) -> Result<Written, Unwritten> {
    // `omitted` is whether a variadic macro's variable arguments were left
    // out, which `, ## __VA_ARGS__` asks.
    let (replaced, written, omitted) = arguments;
    let body = sources.snippet(spelled);
    let mut out = Written::default();
    let in_body = |start: u32, end: u32| {
        Span::new(
            spelled.file(),
            spelled.start() + start,
            spelled.start() + end,
        )
    };
    let is_painted = |token: Token| {
        reading
            .painted
            .contains(&(token.span.file(), token.span.start()))
    };
    // What goes before a token about to be written: nothing at the start,
    // where it has what its use has; a space where C has white space; and
    // otherwise nothing where it may touch, or a space recorded as none.
    let before = |out: &mut Written, spaced: bool, may_touch: bool| {
        if out.text.is_empty() || out.text.ends_with(char::is_whitespace) {
            return;
        }
        if spaced {
            out.text.push(' ');
        } else if !may_touch {
            out.text.push(' ');
            out.unspaced.push(out.text.len() as u32);
        }
    };
    // A run of the list, copied byte for byte. One that begins with a token
    // where the edit before it ended has no white space between them, so it
    // is set off and recorded as touching.
    let run = |out: &mut Written, start: u32, end: u32| {
        if start >= end {
            return;
        }
        let rest = &body[start as usize..end as usize];
        let white = rest.starts_with(char::is_whitespace)
            || rest.starts_with("/*")
            || rest.starts_with("//");
        if start > 0 && !white {
            before(out, false, false);
        }
        out.pieces.push(Piece {
            offset: out.text.len() as u32,
            from: in_body(start, end),
        });
        out.text.push_str(rest);
    };
    // An argument's tokens, the first with the white space `first` says, or
    // its own: written side by side where they were, so a span over several,
    // `free(p)`, is placed over all of them, and marked where never replaced.
    let write_argument = |out: &mut Written, tokens: &[(Token, bool)], first: Option<bool>| {
        let mut last: Option<Token> = None;
        for &(token, leading) in tokens {
            let touching = last.is_some_and(|last| {
                last.span.file() == token.span.file() && last.span.end() == token.span.start()
            });
            match (last, first) {
                (None, Some(spaced)) => before(out, spaced, false),
                (None, None) => before(out, leading, false),
                (Some(_), _) => before(out, leading, touching),
            }
            match out.pieces.last_mut() {
                Some(piece) if touching && !out.text.ends_with(' ') => {
                    piece.from = Span::new(piece.from.file(), piece.from.start(), token.span.end());
                }
                _ => out.pieces.push(Piece {
                    offset: out.text.len() as u32,
                    from: token.span,
                }),
            }
            if is_painted(token) {
                out.painted.push(out.text.len() as u32);
            }
            out.text.push_str(sources.snippet(token.span));
            last = Some(token);
        }
    };
    // A run of operands joined by `##`, the first with the white space
    // `spaced` says: each operand as the tokens it stands for, an argument
    // as written (C17 6.10.3.1 p1), each with whether C has white space
    // before it and the token it was copied from, where it was copied
    // unchanged. What a join makes is copied from `operator`.
    let paste = |out: &mut Written,
                 operands: &[(Operand, bool)],
                 spaced: bool,
                 operator: (u32, u32)|
     -> Result<(), Unwritten> {
        let mut joined: Vec<(String, bool, Option<Token>)> = Vec::new();
        for &(ref operand, operand_spaced) in operands {
            let tokens: Vec<(String, bool, Option<Token>)> = match *operand {
                Operand::Parameter(index) => written[index]
                    .iter()
                    .enumerate()
                    .map(|(at, &(token, leading))| {
                        let leading = if at == 0 { operand_spaced } else { leading };
                        (sources.snippet(token.span).to_owned(), leading, Some(token))
                    })
                    .collect(),
                Operand::Stringized(index) => {
                    vec![(stringized(&written[index], sources), operand_spaced, None)]
                }
                Operand::Token(start, end) => vec![(
                    body[start as usize..end as usize].to_owned(),
                    operand_spaced,
                    None,
                )],
                Operand::Optional { .. } => {
                    unreachable!("`edits_in` refuses a `##` beside `__VA_OPT__`")
                }
            };
            // An empty argument is a placemarker, and a join with one is the
            // other side (C17 6.10.3.3 p3). What a join makes has the white
            // space its left side has.
            let mut tokens = tokens.into_iter();
            match (joined.pop(), tokens.next()) {
                (Some((left, leading, _)), Some((right, _, _))) => {
                    let pasted = format!("{left}{right}");
                    out.scanned += pasted.len();
                    if out.scanned > MAX_PASTED {
                        return Err(Unwritten::TooLong);
                    }
                    if !one_token(&pasted) {
                        return Err(Unwritten::Unpasted(pasted));
                    }
                    joined.push((pasted, leading, None));
                }
                (left, right) => joined.extend(left.into_iter().chain(right)),
            }
            joined.extend(tokens);
        }
        for (at, (token, leading, copied)) in joined.iter().enumerate() {
            before(out, if at == 0 { spaced } else { *leading }, false);
            out.pieces.push(Piece {
                offset: out.text.len() as u32,
                from: copied.map_or(in_body(operator.0, operator.1), |copied| copied.span),
            });
            if copied.is_some_and(is_painted) {
                out.painted.push(out.text.len() as u32);
            }
            out.text.push_str(token);
        }
        Ok(())
    };
    let mut copied = 0;
    for edit in edits {
        let (start, end, spaced) = match *edit {
            Edit::Parameter {
                start, end, spaced, ..
            }
            | Edit::Stringized {
                start, end, spaced, ..
            }
            | Edit::Comma {
                start, end, spaced, ..
            }
            | Edit::Optional {
                start, end, spaced, ..
            }
            | Edit::Pasted {
                start, end, spaced, ..
            } => (start, end, spaced),
        };
        run(&mut out, copied, start);
        match edit {
            Edit::Comma {
                index,
                comma,
                rest,
                operator,
                ..
            } => {
                let variable = (Operand::Parameter(*index), false);
                if omitted && rest.is_empty() {
                    // The comma goes with the white space before it, as it
                    // does in `clang`: `[a ,##__VA_ARGS__]` is `[1]`.
                    // Kept, `#` read `[1 ]`. Found by review.
                    let kept = out.text.trim_end().len();
                    out.text.truncate(kept);
                    let kept = kept as u32;
                    out.pieces.retain(|piece| piece.offset < kept);
                    out.painted.retain(|&at| at < kept);
                    out.unspaced.retain(|&at| at < kept);
                } else if omitted {
                    // An ordinary join of the comma and a placemarker on.
                    let mut operands = vec![(Operand::Token(comma.0, comma.1), spaced), variable];
                    operands.extend(rest.iter().cloned());
                    paste(&mut out, &operands, spaced, *operator)?;
                } else {
                    before(&mut out, spaced, false);
                    out.pieces.push(Piece {
                        offset: out.text.len() as u32,
                        from: in_body(comma.0, comma.1),
                    });
                    out.text.push(',');
                    // The arguments as written, the operand of `##` they
                    // are, with any joins after them made.
                    let leading = written[*index].first().is_some_and(|&(_, leading)| leading);
                    let mut operands = vec![(Operand::Parameter(*index), leading)];
                    operands.extend(rest.iter().cloned());
                    paste(&mut out, &operands, leading, *operator)?;
                }
            }
            Edit::Optional {
                operand:
                    Operand::Optional {
                        index,
                        content,
                        inner,
                        stringized,
                        operator,
                    },
                ..
            } => {
                // Nothing where the variable arguments replace to no token,
                // which `#` makes `""` of.
                if replaced[*index].is_empty() && *stringized {
                    before(&mut out, spaced, false);
                    out.pieces.push(Piece {
                        offset: out.text.len() as u32,
                        from: in_body(operator.0, operator.1),
                    });
                    out.text.push_str("\"\"");
                } else if !replaced[*index].is_empty() {
                    let within = Span::new(
                        spelled.file(),
                        spelled.start() + content.0,
                        spelled.start() + content.1,
                    );
                    let made = substituted(within, inner, arguments, sources, reading)?;
                    // The joins of every `__VA_OPT__` count toward one bound:
                    // counted apart, two of them each just under it joined
                    // twice as much. Found by review.
                    out.scanned += made.scanned;
                    if out.scanned > MAX_PASTED {
                        return Err(Unwritten::TooLong);
                    }
                    if *stringized {
                        before(&mut out, spaced, false);
                        out.pieces.push(Piece {
                            offset: out.text.len() as u32,
                            from: in_body(operator.0, operator.1),
                        });
                        out.text.push_str(&stringized_text(&made));
                    } else if made.text.trim().is_empty() {
                        // Content of white space alone writes no token, and
                        // no piece, which would share an offset with the
                        // next; its white space stays before the next token,
                        // as `clang` keeps it: `[__VA_OPT__( )]` is `[ ]`.
                        if !made.text.is_empty() && !out.text.is_empty() {
                            out.text.push(' ');
                        }
                    } else {
                        before(&mut out, spaced, false);
                        let base = out.text.len() as u32;
                        out.pieces.extend(made.pieces.iter().map(|piece| Piece {
                            offset: base + piece.offset,
                            from: piece.from,
                        }));
                        out.painted.extend(made.painted.iter().map(|at| base + at));
                        out.unspaced
                            .extend(made.unspaced.iter().map(|at| base + at));
                        out.text.push_str(&made.text);
                    }
                }
            }
            Edit::Optional { .. } => {
                unreachable!("an optional edit holds an optional operand, as `edits_in` builds it")
            }
            // The first token has the white space its parameter has.
            Edit::Parameter { index, .. } => {
                write_argument(&mut out, &replaced[*index], Some(spaced));
            }
            Edit::Stringized {
                index, operator, ..
            } => {
                before(&mut out, spaced, false);
                out.pieces.push(Piece {
                    offset: out.text.len() as u32,
                    from: in_body(operator.0, operator.1),
                });
                out.text.push_str(&stringized(&written[*index], sources));
            }
            Edit::Pasted {
                operands, operator, ..
            } => paste(&mut out, operands, spaced, *operator)?,
        }
        copied = end;
    }
    run(&mut out, copied, body.len() as u32);
    // An empty replacement list, or one of an empty argument alone, is
    // still copied from somewhere.
    if out.pieces.first().is_none_or(|piece| piece.offset != 0) {
        out.pieces.insert(
            0,
            Piece {
                offset: 0,
                from: in_body(0, 0),
            },
        );
    }
    Ok(out)
}

/// `tokens`, an argument as written, each with whether C has white space
/// before it, as the string literal `#` makes of it (C17 6.10.3.2 p2): each
/// token's spelling, one space where white space was and none at either
/// end, a `\` before each `"` and `\` of a string literal or a character
/// constant, quoted.
fn stringized(tokens: &[(Token, bool)], sources: &SourceMap) -> String {
    let mut text = String::from("\"");
    for (at, &(token, leading)) in tokens.iter().enumerate() {
        if at > 0 && leading {
            text.push(' ');
        }
        let spelled = sources.snippet(token.span);
        if matches!(token.kind, TokenKind::String | TokenKind::Character) {
            for c in spelled.chars() {
                if c == '"' || c == '\\' {
                    text.push('\\');
                }
                text.push(c);
            }
        } else {
            text.push_str(spelled);
        }
    }
    text.push('"');
    text
}

/// What `#` makes of `made`, the text a `__VA_OPT__` wrote (C23 6.10.5.2):
/// its tokens as [`stringized`] spells an argument's, white space before one
/// where the text has some that was not written only to keep two apart.
fn stringized_text(made: &Written) -> String {
    let mut scratch = SourceMap::new();
    let file = scratch.add_virtual("__VA_OPT__", made.text.as_str());
    let tokens = lex_replacement(file, scratch.file(file), &mut DiagnosticSink::new());
    let mut spelled: Vec<(Token, bool)> = Vec::new();
    let mut end = None;
    for token in tokens.into_iter().filter(|token| !token.is_eof()) {
        let apart = end.is_some_and(|end| end != token.span.start())
            && !made.unspaced.contains(&token.span.start());
        spelled.push((token, apart));
        end = Some(token.span.end());
    }
    stringized(&spelled, &scratch)
}

/// Whether `text`, two tokens written touching, is one preprocessing token
/// this reads, as a `##` has to make (C17 6.10.3.3 p3). A run of characters
/// no token begins is not: the scan takes it as one, where C makes each
/// character a token of its own. The scan ends every text with its end of
/// file, and touching tokens leave no white space for it to pass over.
fn one_token(text: &str) -> bool {
    let mut scratch = SourceMap::new();
    let file = scratch.add_virtual("pasted", text);
    let tokens = lex_replacement(file, scratch.file(file), &mut DiagnosticSink::new());
    matches!(tokens.as_slice(), [token, _] if token.kind != TokenKind::Unknown)
}

/// `tokens`, an argument, replaced as if they were the rest of a file (C17
/// 6.10.3.1 p1): every macro in them expanded and every expansion read
/// through, and what would have been handed on given back, each token with
/// whether C has white space before it. `leading` is the first's.
///
/// The frame stack is this argument's own, so a name ending it is no call
/// whatever follows the argument, as C reads it in isolation; the macros
/// open around the call stay open, and the bounds count on.
fn replaced(
    tokens: Vec<Token>,
    leading: bool,
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> Vec<(Token, bool)> {
    reading.arguments += 1;
    let mut frames = vec![Frame {
        tokens,
        cursor: 0,
        instance: None,
        extent: None,
        conditionals: Vec::new(),
        skipped_depth: 0,
        leading,
    }];
    let mut handed: Vec<(Token, bool)> = Vec::new();
    // How much had been handed on when each open expansion began, and
    // whether an expansion that ended having handed on nothing had white
    // space before it, which the next token takes, as `clang` gives it:
    // with `#define E`, `[ E]` replaced is `[ ]` to `#`. Dropped, it was
    // `[]`. Found by review.
    let mut begun: Vec<usize> = Vec::new();
    let mut carried = false;
    while let Some(frame) = frames.last_mut() {
        let next = frame.tokens.get(frame.cursor).copied();
        frame.cursor += 1;
        let Some(token) = next else {
            if let Some(Frame {
                instance: Some(instance),
                leading,
                ..
            }) = frames.pop()
            {
                closed(&instance, reading);
                if begun.pop() == Some(handed.len()) {
                    carried |= leading;
                }
            }
            continue;
        };
        let spaced = frames
            .last()
            .is_some_and(|frame| spaced(frame, frame.cursor - 1, reading));
        let mut hand = |token: Token, spaced: bool| {
            handed.push((token, spaced || carried));
            carried = false;
        };
        match token.kind {
            TokenKind::Eof => {}
            TokenKind::Identifier | TokenKind::Keyword(_) | TokenKind::Annotation(_) => {
                match replacement(&mut frames, token, sources, reading, diagnostics) {
                    Replaced::Frame(frame) => {
                        begun.push(handed.len());
                        frames.push(frame);
                    }
                    Replaced::Kept => hand(token, spaced),
                    Replaced::Dropped => {}
                }
            }
            _ => hand(token, spaced),
        }
    }
    reading.arguments -= 1;
    handed
}

/// Whether C has white space before the token at `index` of `frame`, which
/// `#` reads (C17 6.10.3.2 p2): the first has what the frame's use has; any
/// other has it where it does not touch the token before it, unless the
/// space between was written only to keep the two apart. The scan passes a
/// comment over, so one between two tokens leaves them apart, as C makes a
/// comment white space.
fn spaced(frame: &Frame, index: usize, reading: &Reading) -> bool {
    match index
        .checked_sub(1)
        .and_then(|before| frame.tokens.get(before))
    {
        None => frame.leading,
        Some(before) => {
            let token = frame.tokens[index];
            before.span.end() != token.span.start()
                && !reading
                    .unspaced
                    .contains(&(token.span.file(), token.span.start()))
        }
    }
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
    parameters: Option<Vec<&str>>,
    origin: Origin,
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

    // Scanned as an expansion is, so a list that begins with `#` holds the
    // operator rather than a directive. See `lex_replacement`.
    let mut scratch = SourceMap::new();
    let scanned = scratch.add_virtual("replacement list", sources.snippet(spelled));
    let scan = lex_replacement(scanned, scratch.file(scanned), &mut DiagnosticSink::new());
    let edits = match edits(&scan, &scratch, parameters.as_deref()) {
        Ok(edits) => edits,
        Err((code, message)) => {
            diagnostics.report(refused(text, directive, code, message));
            return;
        }
    };
    let mut tokens = Vec::new();
    let mut end = 0;
    for token in scan.iter().filter(|token| !token.is_eof()) {
        tokens.push((
            scratch.snippet(token.span).to_owned(),
            !tokens.is_empty() && token.span.start() > end,
        ));
        end = token.span.end();
    }
    let parameters = parameters.map(|names| Parameters {
        names: names.into_iter().map(str::to_owned).collect(),
    });

    if let Some(earlier) = reading.macros.get(name) {
        if earlier.origin != Origin::Written && standard(name) {
            diagnostics.report(predefined_again(name, named, "defined again here", earlier));
        } else if earlier.tokens != tokens
            || earlier.parameters.as_ref().map(|earlier| &earlier.names)
                != parameters.as_ref().map(|parameters| &parameters.names)
        {
            diagnostics.report(
                Diagnostic::error(format!("`{name}` is defined again differently"))
                    .with_code(REDEFINED)
                    .with_label(Label::primary(named, "defined again here"))
                    .with_label(Label::secondary(earlier.defined, "defined here first"))
                    .with_note(
                        "a macro may be defined again only with the same parameters and \
                         replacement list",
                    ),
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
            origin,
            parameters,
            edits,
        },
    );
}

/// What a use of the replacement list `scan` writes as something other than
/// its own text, by the scan's offsets, which count from the list's start;
/// or why the list is not one C allows. `parameters` is a function-like
/// macro's, and an object-like macro's `#` is a token like any other.
///
/// C17 6.10.3.2 p1 asks each `#` of a function-like macro to be followed by
/// a parameter, and 6.10.3.3 p1 a `##` to stand between two operands.
fn edits(
    scan: &[Token],
    scratch: &SourceMap,
    parameters: Option<&[&str]>,
) -> Result<Vec<Edit>, (Code, &'static str)> {
    let tokens: Vec<Token> = scan
        .iter()
        .copied()
        .filter(|token| !token.is_eof())
        .collect();
    edits_in(&tokens, scratch, parameters, true)
}

/// [`edits`] of `tokens`, a replacement list or, not `outermost`, the
/// content of a `__VA_OPT__` in one.
fn edits_in(
    tokens: &[Token],
    scratch: &SourceMap,
    parameters: Option<&[&str]>,
    outermost: bool,
) -> Result<Vec<Edit>, (Code, &'static str)> {
    // A variadic macro's last parameter is `__VA_ARGS__`, as `macro_form`
    // names it.
    let variadic = parameters
        .and_then(|names| names.last())
        .is_some_and(|name| *name == VA_ARGS);
    let va = parameters.map_or(0, |names| names.len().saturating_sub(1));
    let named = |token: Token, name: &str| {
        token.kind == TokenKind::Identifier && scratch.snippet(token.span) == name
    };
    let parameter = |token: Token| {
        let named = matches!(
            token.kind,
            TokenKind::Identifier | TokenKind::Keyword(_) | TokenKind::Annotation(_)
        );
        parameters.filter(|_| named).and_then(|names| {
            names
                .iter()
                .position(|name| *name == scratch.snippet(token.span))
        })
    };
    // Whether the token at `at` has white space before it in the list.
    let spaced = |at: usize| {
        at.checked_sub(1)
            .is_some_and(|before| tokens[before].span.end() != tokens[at].span.start())
    };
    // `__VA_OPT__ ( content )` at `at`, a stringizing if `hashed`: the
    // operand, and the index just past its `)`.
    let optional =
        |at: usize, hashed: Option<u32>| -> Result<(Operand, usize), (Code, &'static str)> {
            if !variadic {
                return Err((
                    MALFORMED_MACRO,
                    "`__VA_OPT__` is read only in a variadic macro's replacement list",
                ));
            }
            if !outermost {
                return Err((MALFORMED_MACRO, "`__VA_OPT__` is inside another"));
            }
            let open = tokens
                .get(at + 1)
                .filter(|open| open.kind == TokenKind::Punct(Punct::LeftParen))
                .ok_or((MALFORMED_MACRO, "`__VA_OPT__` is followed by no `(`"))?;
            let mut depth = 0_usize;
            let mut close = None;
            for (index, token) in tokens.iter().enumerate().skip(at + 2) {
                match token.kind {
                    TokenKind::Punct(Punct::LeftParen) => depth += 1,
                    TokenKind::Punct(Punct::RightParen) if depth == 0 => {
                        close = Some(index);
                        break;
                    }
                    TokenKind::Punct(Punct::RightParen) => depth -= 1,
                    _ => {}
                }
            }
            let close = close.ok_or((MALFORMED_MACRO, "`__VA_OPT__` has no `)`"))?;
            let content = (open.span.end(), tokens[close].span.start());
            let inner = edits_in(&tokens[at + 2..close], scratch, parameters, false)?
                .into_iter()
                .map(|edit| edit.shifted(content.0))
                .collect();
            let operator = match hashed {
                Some(hash) => (hash, tokens[at].span.end()),
                None => (tokens[at].span.start(), tokens[at].span.end()),
            };
            let operand = Operand::Optional {
                index: va,
                content,
                inner,
                stringized: hashed.is_some(),
                operator,
            };
            Ok((operand, close + 1))
        };
    let mut edits = Vec::new();
    // The run being read: each operand with its start, its end, its first
    // token's end, and whether white space is before it; and the run's
    // first `##`.
    let mut run: Vec<(Operand, u32, u32, u32, bool)> = Vec::new();
    let mut first_join: Option<(u32, u32)> = None;
    let flush = |run: &mut Vec<(Operand, u32, u32, u32, bool)>,
                 first_join: &mut Option<(u32, u32)>,
                 edits: &mut Vec<Edit>| {
        match (run.as_slice(), first_join.take()) {
            ([] | [(Operand::Token(..), ..)], _) => {}
            ([(Operand::Parameter(index), start, end, _, spaced)], _) => {
                edits.push(Edit::Parameter {
                    start: *start,
                    end: *end,
                    index: *index,
                    spaced: *spaced,
                });
            }
            ([(Operand::Stringized(index), start, end, head, spaced)], _) => {
                edits.push(Edit::Stringized {
                    start: *start,
                    end: *end,
                    index: *index,
                    // The `#`: one byte, or two spelled `%:` (C17 6.4.6 p3).
                    operator: (*start, *head),
                    spaced: *spaced,
                });
            }
            ([(operand @ Operand::Optional { .. }, start, end, _, spaced)], _) => {
                edits.push(Edit::Optional {
                    start: *start,
                    end: *end,
                    operand: operand.clone(),
                    spaced: *spaced,
                });
            }
            // GNU's `, ## __VA_ARGS__`: the comma goes where the variable
            // arguments are left out, and nothing is joined where they are
            // not. See `call`.
            (
                [
                    (Operand::Token(comma_start, comma_end), start, _, _, spaced),
                    (Operand::Parameter(index), _, variable_end, _, _),
                    rest @ ..,
                ],
                Some(operator),
            ) if variadic
                && *index == va
                && tokens.iter().any(|token| {
                    token.span.start() == *comma_start
                        && token.kind == TokenKind::Punct(Punct::Comma)
                }) =>
            {
                edits.push(Edit::Comma {
                    start: *start,
                    end: rest.last().map_or(*variable_end, |&(_, _, end, _, _)| end),
                    index: *index,
                    comma: (*comma_start, *comma_end),
                    rest: rest
                        .iter()
                        .map(|(operand, _, _, _, spaced)| (operand.clone(), *spaced))
                        .collect(),
                    operator,
                    spaced: *spaced,
                });
            }
            ([(_, start, _, _, spaced), .., (_, _, end, _, _)], Some(operator)) => {
                edits.push(Edit::Pasted {
                    start: *start,
                    end: *end,
                    operands: run
                        .iter()
                        .map(|(operand, _, _, _, spaced)| (operand.clone(), *spaced))
                        .collect(),
                    operator,
                    spaced: *spaced,
                });
            }
            ([_, _, ..], None) => {
                unreachable!("a run of two operands was joined by a `##`")
            }
        }
        run.clear();
    };
    let beside_optional = (
        UNSUPPORTED_DIRECTIVE,
        "a `##` beside `__VA_OPT__` is not read yet",
    );
    let mut joining = false;
    let mut at = 0;
    while let Some(&token) = tokens.get(at) {
        if token.kind == TokenKind::Punct(Punct::HashHash) {
            if run.is_empty() {
                return Err((
                    MALFORMED_MACRO,
                    "`##` has nothing before it in the replacement list",
                ));
            }
            if joining {
                return Err((
                    MALFORMED_MACRO,
                    "`##` has nothing between it and the `##` before it",
                ));
            }
            if matches!(run.last(), Some((Operand::Optional { .. }, ..))) {
                return Err(beside_optional);
            }
            first_join.get_or_insert((token.span.start(), token.span.end()));
            joining = true;
            at += 1;
            continue;
        }
        let (operand, end, next) =
            if parameters.is_some() && token.kind == TokenKind::Punct(Punct::Hash) {
                match tokens.get(at + 1).copied() {
                    Some(after) if named(after, VA_OPT) => {
                        let (operand, next) = optional(at + 1, Some(token.span.start()))?;
                        (operand, tokens[next - 1].span.end(), next)
                    }
                    Some(after) => match parameter(after) {
                        Some(index) => (Operand::Stringized(index), after.span.end(), at + 2),
                        None => return Err((MALFORMED_MACRO, "`#` is followed by no parameter")),
                    },
                    None => return Err((MALFORMED_MACRO, "`#` is followed by no parameter")),
                }
            } else if named(token, VA_OPT) {
                let (operand, next) = optional(at, None)?;
                (operand, tokens[next - 1].span.end(), next)
            } else if named(token, VA_ARGS) && !variadic {
                return Err((
                    MALFORMED_MACRO,
                    "`__VA_ARGS__` is read only in a variadic macro's replacement list",
                ));
            } else {
                let operand = parameter(token).map_or(
                    Operand::Token(token.span.start(), token.span.end()),
                    Operand::Parameter,
                );
                (operand, token.span.end(), at + 1)
            };
        if joining && matches!(operand, Operand::Optional { .. }) {
            return Err(beside_optional);
        }
        if !joining {
            flush(&mut run, &mut first_join, &mut edits);
        }
        run.push((
            operand,
            token.span.start(),
            end,
            token.span.end(),
            spaced(at),
        ));
        joining = false;
        at = next;
    }
    if joining {
        return Err((
            MALFORMED_MACRO,
            "`##` has nothing after it in the replacement list",
        ));
    }
    flush(&mut run, &mut first_join, &mut edits);
    Ok(edits)
}

/// The report for a `#define` or an `#undef`, `what` at `at`, of `name`, one C17
/// 6.10.8 lists, which the implementation defined as `earlier`. Not of a
/// target's names, `__linux__` among them, which 6.10.8 p2 does not cover and
/// `clang` accepts a definition of; asking of every predefined name refused
/// that. Found by review.
fn predefined_again(name: &str, at: Span, what: &str, earlier: &Macro) -> Diagnostic {
    let report = Diagnostic::error(format!("`{name}` is defined by the implementation"))
        .with_code(PREDEFINED_AGAIN)
        .with_label(Label::primary(at, what))
        .with_note("C17 6.10.8 p2 says it shall not be the subject of a `#define` or an `#undef`");
    // A dynamic macro has no line in `<built-in>` to point at.
    if earlier.origin == Origin::Predefined {
        report.with_label(Label::secondary(earlier.defined, "predefined here"))
    } else {
        report
    }
}

/// The names C17 6.10.8.1 requires whose value is the place or the time of
/// their use, which [`dynamic`] writes.
const DYNAMIC: [&str; 4] = ["__FILE__", "__LINE__", "__DATE__", "__TIME__"];

/// The `#define` lines `target` predefines: what C17 6.10.8.1 requires and
/// what every compiler for the triple agrees on, and nothing that names a
/// compiler. See ADR-0055.
///
/// Each value was measured with `clang 20.1.6 --target=<triple> -std=c17
/// -dM -E` on an empty file. A name is here only where the other compilers
/// for the triple are documented to define it too, `gcc` for Linux and
/// Darwin and `cl.exe` for MSVC; where they are not, it is in [`unsettled`]
/// instead. `wasm32-unknown-unknown` has `clang` alone.
fn predefined(target: Target) -> &'static str {
    match target.triple() {
        "aarch64-apple-darwin" => {
            "#define __STDC__ 1\n\
            #define __STDC_VERSION__ 201710L\n\
            #define __STDC_HOSTED__ 1\n\
            #define __CHAR_BIT__ 8\n\
            #define __SIZEOF_INT__ 4\n\
            #define __SIZEOF_LONG__ 8\n\
            #define __SIZEOF_POINTER__ 8\n\
            #define __LP64__ 1\n\
            #define _LP64 1\n\
            #define __ORDER_LITTLE_ENDIAN__ 1234\n\
            #define __ORDER_BIG_ENDIAN__ 4321\n\
            #define __BYTE_ORDER__ __ORDER_LITTLE_ENDIAN__\n\
            #define __APPLE__ 1\n\
            #define __MACH__ 1\n\
            #define __aarch64__ 1\n"
        }
        "aarch64-unknown-linux-gnu" => {
            "#define __STDC__ 1\n\
            #define __STDC_VERSION__ 201710L\n\
            #define __STDC_HOSTED__ 1\n\
            #define __CHAR_BIT__ 8\n\
            #define __CHAR_UNSIGNED__ 1\n\
            #define __SIZEOF_INT__ 4\n\
            #define __SIZEOF_LONG__ 8\n\
            #define __SIZEOF_POINTER__ 8\n\
            #define __LP64__ 1\n\
            #define _LP64 1\n\
            #define __ORDER_LITTLE_ENDIAN__ 1234\n\
            #define __ORDER_BIG_ENDIAN__ 4321\n\
            #define __BYTE_ORDER__ __ORDER_LITTLE_ENDIAN__\n\
            #define __linux__ 1\n\
            #define __linux 1\n\
            #define __unix__ 1\n\
            #define __unix 1\n\
            #define __gnu_linux__ 1\n\
            #define __ELF__ 1\n\
            #define __aarch64__ 1\n"
        }
        "armv7-unknown-linux-gnueabihf" => {
            "#define __STDC__ 1\n\
            #define __STDC_VERSION__ 201710L\n\
            #define __STDC_HOSTED__ 1\n\
            #define __CHAR_BIT__ 8\n\
            #define __CHAR_UNSIGNED__ 1\n\
            #define __SIZEOF_INT__ 4\n\
            #define __SIZEOF_LONG__ 4\n\
            #define __SIZEOF_POINTER__ 4\n\
            #define __ORDER_LITTLE_ENDIAN__ 1234\n\
            #define __ORDER_BIG_ENDIAN__ 4321\n\
            #define __BYTE_ORDER__ __ORDER_LITTLE_ENDIAN__\n\
            #define __linux__ 1\n\
            #define __linux 1\n\
            #define __unix__ 1\n\
            #define __unix 1\n\
            #define __gnu_linux__ 1\n\
            #define __ELF__ 1\n\
            #define __arm__ 1\n"
        }
        "i686-unknown-linux-gnu" => {
            "#define __STDC__ 1\n\
            #define __STDC_VERSION__ 201710L\n\
            #define __STDC_HOSTED__ 1\n\
            #define __CHAR_BIT__ 8\n\
            #define __SIZEOF_INT__ 4\n\
            #define __SIZEOF_LONG__ 4\n\
            #define __SIZEOF_POINTER__ 4\n\
            #define __ORDER_LITTLE_ENDIAN__ 1234\n\
            #define __ORDER_BIG_ENDIAN__ 4321\n\
            #define __BYTE_ORDER__ __ORDER_LITTLE_ENDIAN__\n\
            #define __linux__ 1\n\
            #define __linux 1\n\
            #define __unix__ 1\n\
            #define __unix 1\n\
            #define __gnu_linux__ 1\n\
            #define __ELF__ 1\n\
            #define __i386__ 1\n\
            #define __i386 1\n"
        }
        "wasm32-unknown-unknown" => {
            "#define __STDC__ 1\n\
            #define __STDC_VERSION__ 201710L\n\
            #define __STDC_HOSTED__ 1\n\
            #define __CHAR_BIT__ 8\n\
            #define __SIZEOF_INT__ 4\n\
            #define __SIZEOF_LONG__ 4\n\
            #define __SIZEOF_POINTER__ 4\n\
            #define __ILP32__ 1\n\
            #define _ILP32 1\n\
            #define __ORDER_LITTLE_ENDIAN__ 1234\n\
            #define __ORDER_BIG_ENDIAN__ 4321\n\
            #define __BYTE_ORDER__ __ORDER_LITTLE_ENDIAN__\n\
            #define __LITTLE_ENDIAN__ 1\n\
            #define __wasm__ 1\n\
            #define __wasm32__ 1\n"
        }
        "x86_64-apple-darwin" => {
            "#define __STDC__ 1\n\
            #define __STDC_VERSION__ 201710L\n\
            #define __STDC_HOSTED__ 1\n\
            #define __CHAR_BIT__ 8\n\
            #define __SIZEOF_INT__ 4\n\
            #define __SIZEOF_LONG__ 8\n\
            #define __SIZEOF_POINTER__ 8\n\
            #define __LP64__ 1\n\
            #define _LP64 1\n\
            #define __ORDER_LITTLE_ENDIAN__ 1234\n\
            #define __ORDER_BIG_ENDIAN__ 4321\n\
            #define __BYTE_ORDER__ __ORDER_LITTLE_ENDIAN__\n\
            #define __APPLE__ 1\n\
            #define __MACH__ 1\n\
            #define __x86_64__ 1\n\
            #define __x86_64 1\n\
            #define __amd64__ 1\n\
            #define __amd64 1\n"
        }
        "x86_64-pc-windows-msvc" => {
            "#define __STDC_HOSTED__ 1\n\
            #define _WIN32 1\n\
            #define _WIN64 1\n\
            #define _M_X64 100\n\
            #define _M_AMD64 100\n"
        }
        "x86_64-unknown-linux-gnu" => {
            "#define __STDC__ 1\n\
            #define __STDC_VERSION__ 201710L\n\
            #define __STDC_HOSTED__ 1\n\
            #define __CHAR_BIT__ 8\n\
            #define __SIZEOF_INT__ 4\n\
            #define __SIZEOF_LONG__ 8\n\
            #define __SIZEOF_POINTER__ 8\n\
            #define __LP64__ 1\n\
            #define _LP64 1\n\
            #define __ORDER_LITTLE_ENDIAN__ 1234\n\
            #define __ORDER_BIG_ENDIAN__ 4321\n\
            #define __BYTE_ORDER__ __ORDER_LITTLE_ENDIAN__\n\
            #define __linux__ 1\n\
            #define __linux 1\n\
            #define __unix__ 1\n\
            #define __unix 1\n\
            #define __gnu_linux__ 1\n\
            #define __ELF__ 1\n\
            #define __x86_64__ 1\n\
            #define __x86_64 1\n\
            #define __amd64__ 1\n\
            #define __amd64 1\n"
        }
        triple => panic!("no predefined macros are written for {triple}"),
    }
}

/// The names one compiler for `target` predefines and another does not,
/// which `#ifdef` refuses rather than answers. See ADR-0055.
///
/// For MSVC, `clang` defines the GNU names, `__x86_64__` and
/// `__SIZEOF_LONG__` among them, which `cl.exe`'s documentation does not
/// list, and `cl.exe` defines `__STDC__` and `__STDC_VERSION__` only under an
/// option. Elsewhere, `clang` defines `__LITTLE_ENDIAN__` and, on a 32-bit
/// target, `__ILP32__`, which `gcc`'s documentation does not name, and on
/// Darwin `__arm64__`, which no `gcc` for Darwin was measured for. Answered
/// from `clang`'s list, `#ifdef __x86_64__` took a group `cl.exe` skips, and a
/// double free in that group was analysed away. Found by review. On Linux,
/// `linux` and `unix`, and `i386` on `i686`, are what `clang` and `gcc`
/// define in their default GNU modes and not under `-std=c17`, which is a
/// disagreement of the same kind, measured with `clang 20.1.6` without
/// `-std`. Answered as undefined, a double free under `#ifdef unix` was
/// analysed away. Found by review.
fn unsettled(target: Target) -> &'static [&'static str] {
    match target.triple() {
        "aarch64-apple-darwin" => &["__LITTLE_ENDIAN__", "__arm64__", "__arm64"],
        "aarch64-unknown-linux-gnu" => &["__LITTLE_ENDIAN__", "linux", "unix"],
        "armv7-unknown-linux-gnueabihf" => {
            &["__ILP32__", "_ILP32", "__LITTLE_ENDIAN__", "linux", "unix"]
        }
        "i686-unknown-linux-gnu" => &[
            "__ILP32__",
            "_ILP32",
            "__LITTLE_ENDIAN__",
            "i386",
            "linux",
            "unix",
        ],
        "wasm32-unknown-unknown" => &[],
        "x86_64-apple-darwin" => &["__LITTLE_ENDIAN__"],
        "x86_64-pc-windows-msvc" => &[
            "__STDC__",
            "__STDC_VERSION__",
            "__CHAR_BIT__",
            "__SIZEOF_INT__",
            "__SIZEOF_LONG__",
            "__SIZEOF_POINTER__",
            "__ORDER_LITTLE_ENDIAN__",
            "__ORDER_BIG_ENDIAN__",
            "__BYTE_ORDER__",
            "__LITTLE_ENDIAN__",
            "__x86_64__",
            "__x86_64",
            "__amd64__",
            "__amd64",
        ],
        "x86_64-unknown-linux-gnu" => &["__LITTLE_ENDIAN__", "linux", "unix"],
        triple => panic!("no predefined macros are written for {triple}"),
    }
}

/// The reserved names no C implementation predefines, whatever the target:
/// `__cplusplus`, which C17 6.10.8 p3 forbids one to predefine (checked
/// against N1570, the C11 draft, whose 6.10.8 C17 keeps). Left to the rule
/// for reserved names, `#ifdef __cplusplus`, around the `extern "C"` most
/// headers wrap their declarations in, refused every such header.
const NEVER: [&str; 1] = ["__cplusplus"];

/// Whether `name` is predefined or unsettled for some target, or never
/// predefined, which makes whether this one predefines it a fact about this
/// one rather than a question it cannot answer. See ADR-0055.
fn known(name: &str) -> bool {
    DYNAMIC.contains(&name)
        || NEVER.contains(&name)
        || Target::ALL.iter().any(|target| {
            unsettled(*target).contains(&name)
                || predefined(*target)
                    .lines()
                    .any(|line| line.split(' ').nth(1) == Some(name))
        })
}

/// Whether C17 6.10.8 names `name`, which 6.10.8 p2 forbids a `#define` or
/// an `#undef` of. Every other predefined name may be defined again the same
/// way, or undefined, as `clang` accepts.
fn standard(name: &str) -> bool {
    DYNAMIC.contains(&name) || matches!(name, "__STDC__" | "__STDC_VERSION__" | "__STDC_HOSTED__")
}

/// What a directive that is not an `#include` is, where it is a macro's.
#[derive(Debug, PartialEq, Eq)]
enum MacroForm<'a> {
    /// `#define name body`, with the offsets of the name and of the body's
    /// start and end in the directive's text.
    /// `parameters` is a function-like macro's, written `name(a, b)`.
    Define {
        name: &'a str,
        name_at: u32,
        body: (u32, u32),
        parameters: Option<Vec<&'a str>>,
    },
    /// `#undef name`.
    Undef { name: &'a str },
    /// `#define name(` with a parameter list this does not read: one not
    /// closed, a parameter that is not an ASCII name, or one named twice,
    /// which C17 6.10.3 p6 forbids.
    Parameters,
    /// `#define name(` with `...` before its last parameter, or a named
    /// variadic parameter such as GNU's `args...`.
    Variadic,
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
    let name_at = (text.len() - rest.len()) as u32;
    // `(` straight after the name makes it function-like (C17 6.10.3 p3),
    // and its list runs to the first `)`, since a parameter holds none.
    let (parameters, after) = if let Some(list) = after.strip_prefix('(') {
        let Some(close) = list.find(')') else {
            return MacroForm::Parameters;
        };
        let mut names = Vec::new();
        if !list[..close].trim_matches(blank).is_empty() {
            let count = list[..close].split(',').count();
            for (at, parameter) in list[..close].split(',').enumerate() {
                let parameter = parameter.trim_matches(blank);
                // `...` last names the rest `__VA_ARGS__` (C17 6.10.3 p12).
                if parameter == "..." && at + 1 == count {
                    names.push(VA_ARGS);
                    continue;
                }
                if parameter.ends_with("...") {
                    return MacroForm::Variadic;
                }
                if parameter.is_empty()
                    || parameter.starts_with(|c: char| c.is_ascii_digit())
                    || !parameter.chars().all(identifier)
                    || parameter == "__VA_ARGS__"
                    || names.contains(&parameter)
                {
                    return MacroForm::Parameters;
                }
                names.push(parameter);
            }
        }
        (Some(names), &list[close + 1..])
    } else if after.is_empty() || after.starts_with(blank) {
        (None, after)
    } else {
        return MacroForm::Unread {
            directive: "#define",
        };
    };
    let body = after.trim_start_matches(blank);
    let start = text.len() - body.len();
    let end = start + body.trim_end_matches(blank).len();
    MacroForm::Define {
        name,
        name_at,
        body: (start as u32, end as u32),
        parameters,
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
    /// `#if`, with where its expression is in the line.
    If {
        expression: (u32, u32),
    },
    /// `#ifdef name` or, `negated`, `#ifndef name`.
    Ifdef {
        negated: bool,
        name: Named<'a>,
        extra: Option<(u32, u32)>,
    },
    /// `#elif`, with where its expression is in the line.
    Elif {
        expression: (u32, u32),
    },
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
    // any identifier, such as `(`; `$` or a letter outside ASCII would be,
    // and so would a `\`, which begins a universal character name (C17
    // 6.4.2.1). Ended there instead, `#ifndef A\u00e9` asked about `A`, and a
    // double free under it was skipped. Found by review.
    let ends = |rest: &str| {
        !rest.starts_with(|c: char| identifier(c) || c == '$' || c == '\\' || !c.is_ascii())
    };
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
        "if" | "elif" => {
            let expression = ((text.len() - rest.len()) as u32, text.len() as u32);
            return Some(if word == "if" {
                ConditionalForm::If { expression }
            } else {
                ConditionalForm::Elif { expression }
            });
        }
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
/// past it is believed either way. `already_refused` is a line refused for
/// something else, which only keeps the level.
#[allow(clippy::too_many_arguments)]
fn conditional(
    form: ConditionalForm,
    text: &str,
    directive: Span,
    frame: &mut Frame,
    (sources, reading): (&mut SourceMap, &mut Reading),
    diagnostics: &mut DiagnosticSink,
    already_refused: bool,
) {
    let opens = matches!(
        form,
        ConditionalForm::If { .. } | ConditionalForm::Ifdef { .. }
    );
    // Inside a conditional opened in a skipped group, only the level counts.
    if frame.skipped_depth > 0 || (opens && frame.skipping()) {
        if opens {
            frame.skipped_depth += 1;
        } else if matches!(form, ConditionalForm::Endif { .. }) && frame.skipped_depth > 0 {
            frame.skipped_depth -= 1;
        } else if frame.skipped_depth == 0 {
            // `#else`, `#elif` or `#endif` of the skipped conditional itself.
            act(
                form,
                text,
                directive,
                frame,
                (sources, reading),
                diagnostics,
                already_refused,
            );
        }
        return;
    }
    act(
        form,
        text,
        directive,
        frame,
        (sources, reading),
        diagnostics,
        already_refused,
    );
}

/// Whether `name` being defined is the compiler's to say rather than
/// the target's or the source's: a name C reserves for the implementation
/// (C17 7.1.3 p1) that no target here predefines, such as `__GNUC__`, or
/// that one compiler for this target predefines and another does not, such
/// as `__x86_64__` for MSVC. Answered as undefined, the group a compiler
/// builds was skipped, and a double free under it analysed away. Found by
/// review. A name the source defines first is answered, and an include
/// guard so named is not: where a compiler predefines its name, every macro
/// the header defines is missing from that build, and a later `#ifdef` of
/// one chose a group it does not. Found by review. `unix`, which a GNU mode
/// defines and `-std=c17` does not, is unsettled the same way, reserved or
/// not. See ADR-0055. A name `-D` or `-U` named is answered instead: the
/// command line says which build this is. See ADR-0059.
///
/// What `#ifdef`, `#ifndef`, `defined` and a name left in an `#if` all ask.
fn undecidable(name: &str, reading: &Reading) -> bool {
    !reading.macros.contains_key(name)
        && !reading.settled.contains(name)
        && (unsettled(reading.target).contains(&name) || (reserved(name) && !known(name)))
}

/// The refusal of `what`, a conditional directive at `directive` whose line
/// is `text`, for asking about `name`, which [`undecidable`] answers for.
fn undecidable_report(text: &str, directive: Span, what: &str, name: &str) -> Diagnostic {
    refused(
        text,
        directive,
        UNSUPPORTED_DIRECTIVE,
        &format!("this `{what}` asks about a name the implementation may define"),
    )
    .with_note(if reserved(name) {
        format!(
            "`{name}` is reserved for the implementation (C17 7.1.3), and whether it is \
             defined depends on the compiler, so none of this conditional's groups is compiled"
        )
    } else {
        format!(
            "`{name}` is defined in some modes a compiler builds for this target and not in \
             others, so none of this conditional's groups is compiled"
        )
    })
    // Both ways out, and the reader to choose between them: either is a
    // claim about the build that nothing here can check, so this does not
    // lean toward one. See ADR-0059. A remedy rather than a note, as
    // `docs/diagnostics.md` says of a way out.
    .with_remedy(Remedy::new(format!(
        "say whether the build defines it: `-D {name}` if it does, `-U {name}` if it does not"
    )))
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
#[allow(clippy::too_many_arguments)]
fn act(
    form: ConditionalForm,
    text: &str,
    directive: Span,
    frame: &mut Frame,
    (sources, reading): (&mut SourceMap, &mut Reading),
    diagnostics: &mut DiagnosticSink,
    already_refused: bool,
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
    if already_refused {
        match form {
            ConditionalForm::If { .. } => open(frame, "#if", None),
            ConditionalForm::Ifdef { negated, .. } => {
                open(frame, if negated { "#ifndef" } else { "#ifdef" }, None)
            }
            ConditionalForm::Elif { .. } | ConditionalForm::Else { .. } => {
                if let Some(conditional) = frame.conditionals.last_mut() {
                    conditional.settled = true;
                    conditional.taking = false;
                }
            }
            ConditionalForm::Endif { .. } => {
                frame.conditionals.pop();
            }
        }
        return;
    }
    let unmatched = |diagnostics: &mut DiagnosticSink, message: &str| {
        diagnostics.report(
            refused(text, directive, UNBALANCED, message)
                .with_note("no `#if`, `#ifdef` or `#ifndef` is open in this file"),
        );
    };
    match form {
        // A spliced line is refused by the frame loop, and its expression,
        // which C would read on into the next line, is not evaluated.
        ConditionalForm::If { expression } => {
            let taking = (!spliced)
                .then(|| {
                    let tokens = condition::line(text, directive, expression);
                    let line = (text, directive, "#if");
                    condition::evaluate(tokens, line, sources, reading, diagnostics)
                })
                .flatten();
            open(frame, "#if", taking);
        }
        ConditionalForm::Ifdef {
            negated,
            name,
            extra,
        } => {
            let what = if negated { "#ifndef" } else { "#ifdef" };
            match name {
                // A name C reserves for the implementation (C17 7.1.3 p1)
                // that no target here predefines, such as `__GNUC__`, or that
                // one compiler for this target predefines and another does
                // not, such as `__x86_64__` for MSVC: its answer depends on
                // the compiler that builds the file rather than on the
                // target, so this cannot give one. Answered as
                // undefined, the group a compiler builds was skipped, and a
                // double free under it analysed away. Found by review. A name
                // the source defines first is answered, and an include guard
                // so named is not: where a compiler predefines its name, every
                // macro the header defines is missing from that build, and a
                // later `#ifdef` of one chose a group it does not. Found by
                // review. `unix`, which a GNU mode defines and `-std=c17` does
                // not, is unsettled the same way, reserved or not. See
                // ADR-0055. A name `-D` or `-U` named is answered instead: the
                // command line says which build this is. See ADR-0059.
                Named::Read(name) if undecidable(name, reading) => {
                    diagnostics.report(undecidable_report(text, directive, what, name));
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
        ConditionalForm::Elif { expression } => {
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
                conditional.settled = true;
                conditional.taking = false;
                return;
            }
            // Evaluated only where no group of its conditional was taken or
            // refused (C17 6.10.1 p6): after a taken group it can hold
            // anything, `1/0` among it.
            if conditional.settled || spliced {
                conditional.settled = true;
                conditional.taking = false;
                return;
            }
            let tokens = condition::line(text, directive, expression);
            let line = (text, directive, "#elif");
            let taking = condition::evaluate(tokens, line, sources, reading, diagnostics);
            if let Some(conditional) = frame.conditionals.last_mut() {
                conditional.settled = taking.is_none_or(|taking| taking);
                conditional.taking = taking.unwrap_or(false);
            }
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
    /// Whether C has white space before the frame's first token: an
    /// expansion's has what its use has. See [`spaced`].
    leading: bool,
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
                // Counted where a header is opened, before its guard is
                // asked, so a guarded header included again counts too. Said
                // so, since a note blaming a missing guard was false for a
                // file that includes one guarded header 4097 times. Found by
                // review.
                .with_note(format!(
                    "{MAX_INCLUSIONS} inclusions have already been read for it, each `#include` \
                     counted whether or not the header's include guard then skips it"
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
        preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );

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
        diagnosed(name, files)
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect()
    }

    /// Everything reported for the files, `main.c` among them, written to a
    /// directory of their own.
    fn diagnosed(name: &str, files: &[(String, String)]) -> DiagnosticSink {
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
        preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );
        let _ = fs::remove_dir_all(&directory);
        diagnostics
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

    /// The refusal says what is counted, in words true of a header with an
    /// include guard included past the bound, which is counted each time
    /// though its guard skips it.
    ///
    /// Mutation: restore the note's old wording, that include guards are not
    /// read. This fails.
    #[test]
    fn too_many_inclusions_says_a_guarded_header_counts_each_time() {
        let files = [
            (
                "g.h".to_owned(),
                "#ifndef G_H\n#define G_H\nint z;\n#endif\n".to_owned(),
            ),
            (
                "main.c".to_owned(),
                "#include \"g.h\"\n".repeat(MAX_INCLUSIONS + 1),
            ),
        ];

        let diagnostics = diagnosed("guarded_note", &files);
        let notes = diagnostics.diagnostics()[0].notes();
        assert_eq!(
            notes,
            [format!(
                "{MAX_INCLUSIONS} inclusions have already been read for it, each `#include` \
                 counted whether or not the header's include guard then skips it"
            )]
        );
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
        let preprocessed = preprocess(
            main,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );
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
        let preprocessed = preprocess(
            main,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );
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
            "#define F(args...) x\n",
            "#define G ## b\n",
            "#define H \\\nint x;\n",
        ] {
            let mut sources = SourceMap::new();
            let file = sources.add_virtual("t.c", text);
            let mut diagnostics = DiagnosticSink::new();
            let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
            let preprocessed = preprocess(
                file,
                &tokens,
                Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
                &[],
                &mut sources,
                &mut diagnostics,
            );

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
        preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );

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
        preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );

        assert!(
            !diagnostics.has_errors(),
            "{:?}",
            diagnostics.diagnostics().first()
        );
    }

    /// What `#` and `##` make is placed at its operator in the `#define`:
    /// the `%:` of a stringizing, and the first `##` of a chain. See
    /// ADR-0128.
    ///
    /// Mutations: take every `#` as one byte; place a chain at its last
    /// `##`; or place a stringizing at its whole operand. One assertion
    /// fails.
    #[test]
    fn what_a_hash_or_a_join_makes_is_placed_at_its_operator() {
        let text = "#define S(x) %:x\n#define T(a, b, c) a ## b ## c\nconst char *s = S(q);\nint T(v, w, z);\n";
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        let preprocessed = preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );
        assert!(!diagnostics.has_errors(), "{:?}", diagnostics.diagnostics());

        let written = |spelled: &str| {
            let token = preprocessed
                .tokens()
                .iter()
                .find(|token| sources.snippet(token.span) == spelled)
                .unwrap_or_else(|| panic!("no `{spelled}` was handed on"));
            sources.written(token.span)
        };
        let string = written("\"q\"");
        assert_eq!(sources.snippet(string), "%:");
        let first_join = text.find("##").expect("the chain has a `##`") as u32;
        assert_eq!(written("vwz"), Span::new(file, first_join, first_join + 2));
    }

    /// `__DATE__` and `__TIME__` of a number of seconds are the UTC civil
    /// date and time: the epoch, a leap day, and the last second of 2099,
    /// a year a century rule skips. See ADR-0130.
    ///
    /// Mutation: count from 1 January rather than 1 March, or pad the day
    /// with a `0`; one fails.
    #[test]
    fn the_date_and_time_of_a_number_of_seconds_are_what_utc_says() {
        assert_eq!(
            civil(0),
            ("\"Jan  1 1970\"".to_owned(), "\"00:00:00\"".to_owned())
        );
        assert_eq!(
            civil(951_782_400),
            ("\"Feb 29 2000\"".to_owned(), "\"00:00:00\"".to_owned())
        );
        assert_eq!(
            civil(4_102_444_799),
            ("\"Dec 31 2099\"".to_owned(), "\"23:59:59\"".to_owned())
        );
    }

    /// Every month is spelled as C17 6.10.8.1 spells it: the first day of
    /// each in 2001, a year with no leap day.
    ///
    /// Mutation: misspell any month in `civil`'s table. Its assertion fails.
    #[test]
    fn every_month_of_a_date_is_spelled_as_c_spells_it() {
        const FIRST_OF_2001: u64 = 978_307_200;
        let months = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let lengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut day = 0;
        for (month, length) in months.iter().zip(lengths) {
            let (date, _) = civil(FIRST_OF_2001 + day * 86_400);
            assert_eq!(date, format!("\"{month}  1 2001\""));
            day += length;
        }
    }

    /// `SOURCE_DATE_EPOCH` is decimal digits alone up to the last second of
    /// 9999, as `clang` reads it, and anything else is kept to be refused
    /// rather than read as now.
    ///
    /// Mutations: trim it, let a sign through, or drop the bound; one of the
    /// refused values is read.
    #[test]
    fn source_date_epoch_is_read_as_seconds_or_refused() {
        assert_eq!(started_from(Some("42".to_owned())), Ok(42));
        assert_eq!(started_from(Some(LAST_SECOND.to_string())), Ok(LAST_SECOND));
        for refused in [" 42", "+5", "", "tomorrow", "253402300800"] {
            assert_eq!(
                started_from(Some(refused.to_owned())),
                Err(refused.to_owned()),
                "{refused:?}"
            );
        }
    }

    /// A name `__FILE__` writes is a string literal of it, a `\\` and a `"`
    /// in it escaped.
    ///
    /// Mutation: escape either one no more. Its assertion fails.
    #[test]
    fn a_file_name_is_quoted_as_a_string_literal() {
        assert_eq!(quoted("a\\b\"c.h"), "\"a\\\\b\\\"c.h\"");
    }

    /// `__FILE__` in an included file is its includer's directory joined to
    /// the name its `#include` wrote, by the host's separator, as `clang`
    /// writes it; and in a header the header includes, that one's directory
    /// again. See ADR-0130.
    ///
    /// Mutations: write the name as the `#include` wrote it, or join with
    /// `/` on Windows; an assertion fails on that host.
    #[test]
    fn a_file_macro_in_a_header_names_it_as_clang_does() {
        let directory = std::env::temp_dir().join(format!("safec_presumed_{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("sub")).unwrap();
        fs::write(directory.join("main.c"), "#include \"sub/h.h\"\n").unwrap();
        fs::write(
            directory.join("sub").join("h.h"),
            "#include \"g.h\"\nconst char *h = __FILE__;\n",
        )
        .unwrap();
        fs::write(
            directory.join("sub").join("g.h"),
            "const char *g = __FILE__;\n",
        )
        .unwrap();

        let mut sources = SourceMap::new();
        let file = sources.load(directory.join("main.c")).unwrap();
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        let handed = preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );
        let _ = fs::remove_dir_all(&directory);
        let strings: Vec<&str> = handed
            .tokens()
            .iter()
            .filter(|token| token.kind == TokenKind::String)
            .map(|token| sources.snippet(token.span))
            .collect();

        let separator = if cfg!(windows) { "\\" } else { "/" };
        let root = directory.display().to_string();
        let header = format!("{root}{separator}sub/h.h");
        let nested = format!("{root}{separator}sub{separator}g.h");
        assert_eq!(strings, [quoted(&nested), quoted(&header)]);
    }

    /// A file of function-like calls, one a line, past the bound for one use,
    /// is read: each call written outside any expansion and argument starts
    /// a use of its own.
    ///
    /// Mutations: drop `begin_use` from `call`, or leave `Reading::arguments`
    /// raised after an argument; the count never starts again and the last
    /// call is refused as `SC0113`.
    #[test]
    fn a_function_like_macro_called_on_many_lines_is_not_bounded_as_one_tree() {
        let mut text = String::from("#define ID(x) x\n");
        for at in 0..=MAX_EXPANSIONS {
            text.push_str(&format!("int v{at} = ID(1);\n"));
        }
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );

        assert!(
            !diagnostics.has_errors(),
            "{:?}",
            diagnostics.diagnostics().first()
        );
    }

    /// A call's arguments count toward the bound of the use they are in:
    /// two arguments each just under it are refused together.
    ///
    /// Mutation: start a use at an expansion inside an argument, in
    /// `begin_use`; each argument is counted on its own and nothing is
    /// refused.
    #[test]
    fn a_call_s_arguments_count_toward_its_use_s_bound() {
        let mut text = String::from("#define M0 1\n");
        for at in 1..=15 {
            text.push_str(&format!("#define M{at} M{} M{}\n", at - 1, at - 1));
        }
        text.push_str("#define P(a, b) a b\nint v[] = { P(M15, M15) };\n");
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );

        let codes: Vec<String> = diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect();
        assert_eq!(codes, ["SC0113"]);
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
        preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );

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
        // The expression is the rest of the line after the name.
        assert_eq!(conditional_form("#if 1"), Some(If { expression: (3, 5) }));
        assert_eq!(
            conditional_form("%:elif X"),
            Some(Elif { expression: (6, 8) })
        );
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
        assert_eq!(
            conditional_form("#ifdef A\\u00e9"),
            Some(Ifdef {
                negated: false,
                name: Named::Unread,
                extra: Some((8, 14))
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
    /// handed over. A refused `#if` is one whose expression divides by zero,
    /// or that asks about a name the implementation may define. Mutation:
    /// compile the group after a refused `#if`. `a`, or `i`, is handed over.
    /// Mutation: leave a refused `#elif` unsettled, or taking. `k`, or `j`,
    /// is.
    /// Mutation: take a second `#else` as the first is taken. `e` is.
    /// Mutation: take the group of an `#ifdef` whose name is not read. `g`
    /// is.
    #[test]
    fn no_group_of_a_refused_or_doubled_conditional_is_handed_over() {
        let text = "#if 1 / 0\nint a;\n#elif 2\nint b;\n#else\nint c;\n#endif\n\
                    #ifdef U\n#else\nint d;\n#else\nint e;\n#endif\nint f;\n\
                    #ifdef A$B\nint g;\n#endif\n\
                    #if __GNUC__\nint h;\n#else\nint i;\n#endif\n\
                    #if 0\n#elif 1 / 0\nint j;\n#else\nint k;\n#endif\n";
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", text);
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        let handed = preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );

        let spelled: Vec<&str> = handed
            .tokens()
            .iter()
            .filter(|token| !token.is_eof())
            .map(|token| sources.snippet(token.span))
            .collect();
        assert_eq!(spelled, ["int", "d", ";", "int", "f", ";"]);
    }

    /// A `-D` whose value ends in `\` is refused alone, and the macro given
    /// after it is still defined.
    ///
    /// Asked of the tokens rather than of a run, for the reason the test above
    /// gives: the refusal stops a run before the parser. A directive the lexer
    /// refuses for a splice still ends at its line break, so one line per
    /// macro is enough. Mutation: separate the macros with a space rather than a
    /// line break. `AFTER`'s `#define` is part of `A`'s line, `AFTER` is never
    /// defined, and `x` is not handed over.
    #[test]
    fn a_value_ending_in_a_splice_is_refused_without_taking_the_next_macro() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "#ifdef AFTER\nint x;\n#endif\n");
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        let macros = [
            CommandLineMacro::Defined {
                name: "A".to_owned(),
                value: "1\\".to_owned(),
            },
            CommandLineMacro::Defined {
                name: "AFTER".to_owned(),
                value: "1".to_owned(),
            },
        ];
        let handed = preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &macros,
            &mut sources,
            &mut diagnostics,
        );

        let spelled: Vec<&str> = handed
            .tokens()
            .iter()
            .filter(|token| !token.is_eof())
            .map(|token| sources.snippet(token.span))
            .collect();
        assert_eq!(spelled, ["int", "x", ";"]);
        assert!(diagnostics.has_errors(), "`-D A=1\\` is refused");
    }

    /// What `target` predefines `name` as, read from its list.
    fn value(triple: &str, name: &str) -> Option<&'static str> {
        let target = Target::from_triple(triple).expect("a known triple");
        predefined(target).lines().find_map(|line| {
            let mut words = line.splitn(3, ' ');
            (words.nth(1) == Some(name)).then(|| words.next().unwrap_or(""))
        })
    }

    /// Two targets' answers, written out as `clang 20.1.6` measured them and
    /// the other compilers' documentation agrees, rather than read back from
    /// the lists, which would hold for any values.
    ///
    /// Mutation: change any of these values in a list, or give one target
    /// another's list, or move a name between a list and [`unsettled`]. One
    /// assertion fails.
    #[test]
    fn every_target_predefines_what_its_compilers_agree_on() {
        let msvc = "x86_64-pc-windows-msvc";
        let arm = "aarch64-unknown-linux-gnu";
        assert_eq!(value(msvc, "_WIN32"), Some("1"));
        assert_eq!(value(msvc, "_WIN64"), Some("1"));
        assert_eq!(value(msvc, "__STDC__"), None);
        assert_eq!(value(msvc, "__SIZEOF_LONG__"), None);
        assert_eq!(value(msvc, "__STDC_HOSTED__"), Some("1"));
        assert_eq!(value(msvc, "__CHAR_UNSIGNED__"), None);
        assert_eq!(value(msvc, "_M_X64"), Some("100"));
        assert_eq!(value(arm, "_WIN32"), None);
        assert_eq!(value(arm, "__STDC__"), Some("1"));
        assert_eq!(value(arm, "__STDC_VERSION__"), Some("201710L"));
        assert_eq!(value(arm, "__SIZEOF_LONG__"), Some("8"));
        assert_eq!(value(arm, "__CHAR_UNSIGNED__"), Some("1"));
        assert_eq!(value(arm, "__aarch64__"), Some("1"));
        assert_eq!(value(arm, "__linux__"), Some("1"));
        assert_eq!(value(arm, "__LITTLE_ENDIAN__"), None);

        let unsettled = |triple| unsettled(Target::from_triple(triple).expect("a known triple"));
        assert!(unsettled(msvc).contains(&"__x86_64__"));
        assert!(unsettled(msvc).contains(&"__SIZEOF_LONG__"));
        assert!(unsettled(msvc).contains(&"__STDC__"));
        assert!(unsettled(msvc).contains(&"__STDC_VERSION__"));
        assert!(!unsettled(msvc).contains(&"_WIN32"));
        assert!(unsettled(arm).contains(&"__LITTLE_ENDIAN__"));
        assert!(!unsettled(arm).contains(&"__aarch64__"));
        assert!(unsettled(arm).contains(&"unix"));
        assert!(!unsettled(msvc).contains(&"unix"));
    }

    /// Every target has a list, and every line of it is a `#define` this
    /// reads, with no report.
    ///
    /// Mutation: drop a target's arm. It panics, naming the triple.
    #[test]
    fn every_target_has_a_list_of_defines() {
        for target in Target::ALL {
            let mut sources = SourceMap::new();
            let file = sources.add_virtual("t.c", "");
            let mut diagnostics = DiagnosticSink::new();
            let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
            preprocess(file, &tokens, *target, &[], &mut sources, &mut diagnostics);
            assert!(
                diagnostics.diagnostics().is_empty(),
                "{}: {:?}",
                target.triple(),
                diagnostics.diagnostics()
            );
        }
    }

    /// A name some target predefines is known everywhere, and a compiler's
    /// name nowhere.
    ///
    /// Mutation: ask only the run's own target. `_WIN32` is not known.
    #[test]
    fn a_name_some_target_predefines_is_known() {
        assert!(known("_WIN32"));
        assert!(known("__linux__"));
        assert!(known("__ILP32__"));
        assert!(known("__LINE__"));
        assert!(!known("__GNUC__"));
        assert!(known("__cplusplus"));
        assert!(!known("_MSC_VER"));
    }

    /// The code moved from the lexer with the report, and is the handle a
    /// reader kept, so it is asserted by itself here as the lexer's are there.
    #[test]
    fn a_directive_that_is_not_read_keeps_the_code_it_was_assigned() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "#pragma pack(4)\n");
        let mut diagnostics = DiagnosticSink::new();
        let tokens = lex(file, &sources.file_owned(file), &mut diagnostics);
        preprocess(
            file,
            &tokens,
            Target::from_triple("x86_64-unknown-linux-gnu").expect("a known triple"),
            &[],
            &mut sources,
            &mut diagnostics,
        );

        let codes: Vec<String> = diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code())
            .map(|code| code.to_string())
            .collect();
        assert_eq!(codes, ["SC0104"]);
    }
}
