//! `#pragma` and `_Pragma`, `#line` and GNU's line markers, `#error` and
//! `#warning`: the directives
//! that choose no group and define no macro (C17 6.10.4 to 6.10.6 and
//! 6.10.9, and C23's `#warning`). Each is read only in a group that is
//! compiled; in a skipped one a directive is read only as far as its name.

use super::held::Sources;
use super::line::{Line, Name};
use super::{
    Frame, Reading, UNSUPPORTED_DIRECTIVE, Uncollected, begin_use, canonical, collected, condition,
    hash, refused, replaced, spliced, too_deep, used_at,
};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::lexer::UNSUPPORTED_SPLICE;
use crate::token::{Token, TokenKind};
use safec_ir::source::{FileName, Span};

/// A pragma this does not read, ignored as C17 6.10.6 p1 allows. See
/// ADR-0129.
const UNREAD_PRAGMA: Code = Code::new("SC0118");
/// `#error` in a group that is compiled (C17 6.10.5).
const ERROR_DIRECTIVE: Code = Code::new("SC0121");
/// `#warning` in a group that is compiled (C23 6.10.7).
const WARNING_DIRECTIVE: Code = Code::new("SC0122");

/// Why a name [`Name::Glued`] is refused rather than read.
pub(super) const GLUED_NOTE: &str = "a `$`, a universal character name or a letter outside ASCII \
                                     goes on with a name, and this does not read such names yet";

/// Read `text`, the line of the directive at `directive` with its comments
/// made spaces, if it is one of this module's, and say whether it was.
pub(super) fn read(
    directive_line: &Line,
    directive: Span,
    sources: &mut Sources<'_>,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> bool {
    let text = directive_line.text();
    let Some((word, end, marker)) = owned(directive_line) else {
        return false;
    };
    let rest = &text[end..];
    let what = if marker {
        "line marker".to_owned()
    } else {
        format!("`#{word}`")
    };
    // A splice joins the line below to this one before any directive is
    // read (C17 5.1.1.2 p1, phase 2), and the lexer read that line on its
    // own, as code: `#pragma GCC diagnostic \` would leave `push` behind.
    if spliced(text) {
        diagnostics.report(
            refused(
                text,
                directive,
                UNSUPPORTED_SPLICE,
                &format!("a line splice or a trigraph in a {what} is not read yet"),
            )
            .with_note("C can join the line below to this one, and it would be read on its own"),
        );
        return true;
    }
    match word {
        // The line is the marker's first operand, so it is read from it.
        _ if marker => line(
            Form::Marker,
            (directive_line, directive),
            (end - word.len()) as u32,
            sources,
            reading,
            diagnostics,
        ),
        "pragma" => pragma(
            (directive_line, end),
            hash(text, directive),
            sources,
            reading,
            diagnostics,
        ),
        "line" => line(
            Form::Line,
            (directive_line, directive),
            (text.len() - rest.len()) as u32,
            sources,
            reading,
            diagnostics,
        ),
        _ => {
            // The line after the name is the message, as `clang` says it.
            let message = rest.trim();
            let report = match (word, message.is_empty()) {
                ("error", true) => Diagnostic::error("`#error`"),
                ("error", false) => Diagnostic::error(message),
                (_, true) => Diagnostic::warning("`#warning`"),
                (_, false) => Diagnostic::warning(message),
            };
            let code = if word == "error" {
                ERROR_DIRECTIVE
            } else {
                WARNING_DIRECTIVE
            };
            diagnostics.report(
                report
                    .with_code(code)
                    .with_label(Label::primary(hash(text, directive), "written here")),
            );
        }
    }
    true
}

/// Whether `line` is one of this module's directives, which [`read`] reads.
pub(super) fn owns(line: &Line) -> bool {
    owned(line).is_some()
}

/// The directive's word, the offset past it, and whether it is a line
/// marker's number, where `line` is one of this module's directives.
fn owned(line: &Line) -> Option<(&str, usize, bool)> {
    let (word, end) = line.directive()?;
    // GNU's `# 40 "gen.c"`, which `cc -E` writes and `clang` reads.
    let marker = !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_digit());
    (marker || matches!(word, "pragma" | "line" | "error" | "warning"))
        .then_some((word, end, marker))
}

/// Whether `text`, a directive's line in a group that is skipped, chooses a
/// group this does not read: C23's `#elifdef` and `#elifndef`, which
/// `clang` acts on in C17 too. Passed over, the group one chose was never
/// taken, and a `free` in it that `clang` compiles was not checked. Found by
/// architecture review 12. Only the names that choose a group are asked
/// about: a skipped group holds other text C passes over (C17 6.10.1 p6),
/// and real headers write some, FreeBSD's `sys/queue.h` a `#warn`, which
/// refusing every unknown name refused. Found by review.
pub(super) fn chooses_a_group(line: &Line) -> bool {
    matches!(line.directive(), Some(("elifdef" | "elifndef", _)))
}

/// What a pragma comes to. See ADR-0129.
#[derive(Debug, PartialEq, Eq)]
enum Class {
    /// `once`.
    Once,
    /// One whose meaning ignoring it changes in no program here.
    Ignored,
    /// One whose meaning ignoring it would change, by what it is called.
    Refused(String),
    /// `GCC error` or `GCC warning`, a message `clang` reports as `#error`
    /// or `#warning` would: ignored, a build `clang` stops went on.
    Message {
        /// Whether it is `GCC error`.
        error: bool,
    },
    /// One not named in ADR-0129.
    Unknown,
}

/// The first two names of the pragma in `line` from `from`, the text after
/// `pragma`, and the offset just past them; `None` where either is
/// [`Name::Glued`]. A name is read only where one is: `#pragma "once"` has
/// none, where reading past the quote made it `once` and a header `clang`
/// reads twice was read once. Found by review. And one the lexer cannot read
/// whole is not read as a shorter one: `once$` is a name of its own to
/// `clang`, which ignores it as unknown, and read as `once` it read a header
/// once that `clang` reads twice. Found by architecture review 12.
fn names(line: &Line, from: usize) -> Option<(&str, &str, usize)> {
    match line.name_at(from) {
        Name::Read(first, end) => match line.name_at(end) {
            Name::Read(second, after) => Some((first, second, after)),
            Name::Not => Some((first, "", end)),
            Name::Glued | Name::Undecided => None,
        },
        Name::Not => Some(("", "", from)),
        Name::Glued | Name::Undecided => None,
    }
}

/// The class of the pragma in `line` from `from`, the text after `pragma`:
/// by its first name, and by the name after it where the first is `STDC`,
/// `GCC` or `clang`, which are namespaces, or `options`. `None` where a name
/// is [`Name::Glued`].
fn classify(line: &Line, from: usize) -> Option<Class> {
    let (first, second, _) = names(line, from)?;
    Some(match (first, second) {
        ("once", _) => Class::Once,
        ("STDC", "FP_CONTRACT" | "FENV_ACCESS" | "CX_LIMITED_RANGE")
        | ("GCC" | "clang", "diagnostic")
        | ("GCC", "system_header")
        | ("warning" | "message" | "region" | "endregion", _) => Class::Ignored,
        ("GCC", "error" | "warning") => Class::Message {
            error: second == "error",
        },
        // `GCC visibility` hides what it covers from the dynamic linker, so
        // ignoring it exports symbols `clang` would not: linking changed, as
        // `pack` changes layout. ADR-0132 moves it here from the ignored.
        // `clang attribute` and `clang section` give what they cover an
        // attribute or a section, the same change. See ADR-0133.
        ("GCC", "poison" | "visibility")
        | ("clang", "assume_nonnull" | "attribute" | "section")
        | ("options", "align") => Class::Refused(format!("{first} {second}")),
        (
            "pack" | "align" | "ms_struct" | "push_macro" | "pop_macro" | "redefine_extname"
            | "weak" | "comment" | "section" | "code_seg" | "data_seg" | "bss_seg" | "const_seg"
            | "init_seg",
            _,
        ) => Class::Refused(first.to_owned()),
        _ => Class::Unknown,
    })
}

/// Act on the pragma `rest`, the text after `pragma`, written at `at`: a
/// `#pragma`'s `#`, or a `_Pragma`'s string literal.
fn pragma(
    (line, from): (&Line, usize),
    at: Span,
    sources: &mut Sources<'_>,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) {
    let Some(class) = classify(line, from) else {
        diagnostics.report(
            Diagnostic::error("this pragma's name goes on past what is read")
                .with_code(UNSUPPORTED_DIRECTIVE)
                .with_label(Label::primary(at, "written here"))
                .with_note(GLUED_NOTE),
        );
        return;
    };
    match class {
        // The file of its use: a `_Pragma` a macro writes marks the file
        // the macro is used in, as it would had it been written there. Only
        // an included file: `clang 22.1.8` reads the input again where it is
        // included after `#pragma once`, and warns, so the input marked was
        // a program `clang` refuses for defining it all twice.
        Class::Once => match sources.file(used_at(at, sources).file()).name() {
            FileName::Inclusion { path, .. } => {
                reading.once.insert(canonical(path));
            }
            _ => diagnostics.report(
                Diagnostic::warning("`#pragma once` in the input is not read, and is ignored")
                    .with_code(UNREAD_PRAGMA)
                    .with_label(Label::primary(at, "written here"))
                    .with_note("an `#include` of the input reads it again"),
            ),
        },
        Class::Ignored => {}
        Class::Message { error } => {
            let (.., end) = names(line, from).expect("classified by its names");
            let after = line.text()[end..].trim();
            let literal = after
                .strip_prefix('(')
                .and_then(|inner| inner.strip_suffix(')'))
                .map_or(after, str::trim);
            let report = match destringized(literal, Escapes::QuotesAndBackslashes) {
                Some(message) if error => Diagnostic::error(message).with_code(ERROR_DIRECTIVE),
                Some(message) => Diagnostic::warning(message).with_code(WARNING_DIRECTIVE),
                // `clang` stops here too.
                None => Diagnostic::error(format!(
                    "this `#pragma GCC {}` is not read",
                    if error { "error" } else { "warning" }
                ))
                .with_code(UNSUPPORTED_DIRECTIVE)
                .with_note("it is read with a string literal after it, or one in `(` and `)`"),
            };
            diagnostics.report(report.with_label(Label::primary(at, "written here")));
        }
        Class::Refused(what) => diagnostics.report(
            Diagnostic::error(format!("`#pragma {what}` is not read"))
                .with_code(UNSUPPORTED_DIRECTIVE)
                .with_label(Label::primary(at, "written here"))
                .with_note("ignoring it would change what the program is"),
        ),
        Class::Unknown => diagnostics.report(
            Diagnostic::warning("this `#pragma` is not read, and is ignored")
                .with_code(UNREAD_PRAGMA)
                .with_label(Label::primary(at, "written here"))
                .with_note("C17 6.10.6 p1 lets a compiler ignore a pragma it does not know"),
        ),
    }
}

/// `_Pragma`, at `token`, a name the innermost of `frames` was reading: the
/// pragma its operand spells (C17 6.10.9). The `(` and the `)` are read as a
/// call's are, past an expansion that ends first, so `#define P _Pragma`
/// used as `P("once")` is read, and what is between them is replaced as an
/// argument is, so `_Pragma(STR(x))` is, as `clang` reads both; what that
/// comes to has to be one string literal with no prefix but `L`, which C
/// deletes. Refused otherwise, the report made, what was read taken.
pub(super) fn operator(
    frames: &mut Vec<Frame>,
    token: Token,
    sources: &mut Sources<'_>,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) {
    let unread = |diagnostics: &mut DiagnosticSink| {
        diagnostics.report(
            Diagnostic::error("this `_Pragma` is not read")
                .with_code(UNSUPPORTED_DIRECTIVE)
                .with_label(Label::primary(token.span, "written here"))
                .with_note(
                    "`_Pragma` is read with `(`, what replaces to one string literal with no \
                     prefix but `L`, and `)` after it",
                ),
        );
    };
    // One operand, its commas in it.
    let operand = match collected(frames, token, &|_| false, sources, reading, diagnostics) {
        Ok(collected) => collected
            .arguments
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        Err(Uncollected::NoCall) => return unread(diagnostics),
        Err(Uncollected::Refused) => return,
    };
    begin_use(reading);
    if too_deep(token, reading, diagnostics) {
        return;
    }
    let leading = operand.first().is_some_and(|&(_, leading)| leading);
    let tokens = operand.into_iter().map(|(token, _)| token).collect();
    let errors = diagnostics.error_count();
    let replaced = replaced(tokens, leading, sources, reading, diagnostics);
    if diagnostics.error_count() > errors {
        return;
    }
    let read = match replaced.as_slice() {
        [(literal, _)] if literal.kind == TokenKind::String => {
            // C17 6.10.9 p1 deletes an `L` prefix before the rest is read.
            let spelled = sources.snippet(literal.span);
            let spelled = spelled.strip_prefix('L').unwrap_or(spelled);
            destringized(spelled, Escapes::QuotesAndBackslashes).map(|text| (*literal, text))
        }
        _ => None,
    };
    // Its text is read through translation phase 3 (C17 6.10.9 p1), each
    // comment one space, as a `#pragma` line's is. Classified with its
    // comments in it, `_Pragma("/**/push_macro(\"N\")")` had no name and
    // was ignored, where ADR-0129 refuses it, and `clang` read a program
    // with a heap overflow safec passed. Found by the hunt before #618
    // closed. A comment the text does not end is refused, as on a line.
    let read = read.and_then(|(literal, text)| Some((literal, Line::operand(&text)?)));
    match read {
        Some((literal, line)) => pragma((&line, 0), literal.span, sources, reading, diagnostics),
        None => unread(diagnostics),
    }
}

/// Which escape sequences [`destringized`] reads in a literal.
#[derive(Clone, Copy)]
enum Escapes {
    /// `\"` and `\\` read, every other kept as written: what `_Pragma`
    /// does (C17 6.10.9 p1).
    QuotesAndBackslashes,
    /// Every escape read as a string literal's is (C17 6.4.4.4), and the
    /// text refused where its bytes are not UTF-8: a `#line` or line
    /// marker's name, which `clang` reads so. Refused where any escape but
    /// `\"` and `\\` was in it, a `.i` naming a header outside ASCII, which
    /// `clang -E` writes as `\303\251`, was not read. Found by review.
    Every,
}

/// The text of `literal`, a string literal with no prefix, its quotes
/// dropped and its escapes read as `escapes` says; `None` for any other.
fn destringized(literal: &str, escapes: Escapes) -> Option<String> {
    let body = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut bytes = Vec::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    let push = |bytes: &mut Vec<u8>, c: char| {
        bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
    };
    // Up to `most` digits of `radix` next, as one value, and how many there
    // were; `None` past what `u32` holds.
    let digits = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
                  radix: u32,
                  most: usize|
     -> Option<(u32, usize)> {
        let mut value = 0_u32;
        let mut count = 0;
        while count < most {
            let Some(digit) = chars.peek().and_then(|c| c.to_digit(radix)) else {
                break;
            };
            value = value.checked_mul(radix)?.checked_add(digit)?;
            chars.next();
            count += 1;
        }
        Some((value, count))
    };
    let byte = |value: u32| u8::try_from(value).ok();
    while let Some(c) = chars.next() {
        if c != '\\' {
            push(&mut bytes, c);
            continue;
        }
        let escaped = chars.next()?;
        match (escaped, escapes) {
            ('"' | '\\', _) => push(&mut bytes, escaped),
            (other, Escapes::QuotesAndBackslashes) => {
                push(&mut bytes, '\\');
                push(&mut bytes, other);
            }
            (simple, Escapes::Every) => match simple {
                '\'' | '?' => push(&mut bytes, simple),
                'a' => bytes.push(0x07),
                'b' => bytes.push(0x08),
                'f' => bytes.push(0x0c),
                'n' => bytes.push(b'\n'),
                'r' => bytes.push(b'\r'),
                't' => bytes.push(b'\t'),
                'v' => bytes.push(0x0b),
                // One to three octal digits, this one the first.
                '0'..='7' => {
                    let (rest, count) = digits(&mut chars, 8, 2)?;
                    let first = simple.to_digit(8)? * 8_u32.pow(u32::try_from(count).ok()?);
                    bytes.push(byte(first + rest)?);
                }
                'x' => match digits(&mut chars, 16, usize::MAX)? {
                    (value, 1..) => bytes.push(byte(value)?),
                    _ => return None,
                },
                'u' | 'U' => {
                    let most = if simple == 'u' { 4 } else { 8 };
                    let (value, count) = digits(&mut chars, 16, most)?;
                    if count != most {
                        return None;
                    }
                    push(&mut bytes, char::from_u32(value)?);
                }
                _ => return None,
            },
        }
    }
    String::from_utf8(bytes).ok()
}

/// The greatest line `#line` may name (C17 6.10.4 p3).
const LAST_LINE: u64 = 2_147_483_647;

/// The greatest line a line marker may name, which is not C's and so not
/// bound by C's limit: `clang` reads up to this and refuses past it. Found
/// by review.
const LAST_MARKED_LINE: u64 = 4_294_967_295;

/// Which directive renumbers the lines after it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    /// `#line N "name"` (C17 6.10.4).
    Line,
    /// GNU's `# N "name" flags`, measured against `clang 22.1.8`: line `0`
    /// is read, since GCC writes `# 0 "x.c"` at the head of a `.i` and
    /// `clang` reads it; and after the name, `1` or `2`, then `3`, then `4`
    /// after a `3`, `1` opening a level of its file and `2` closing one, `3`
    /// and `4` read and ignored, since what they change here is no more than
    /// which warnings a header gets. See #618's second design, Q3.
    Marker,
}

/// `#line N` or `#line N "name"`, or a line marker, as `form` says: the
/// line `text` of the directive at `directive`, its operands from `after`,
/// macro-replaced first (C17 6.10.4 p5; `clang` replaces a marker's too):
/// the line after it is line `N` to `__LINE__`, and the file is `name` to
/// `__FILE__`, and to nothing else. A diagnostic points at the line where
/// it is in the file, the text a reader can open.
fn line(
    form: Form,
    (directive_line, directive): (&Line, Span),
    after: u32,
    sources: &mut Sources<'_>,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) {
    let text = directive_line.text();
    let (what, lowest, highest) = match form {
        Form::Line => ("this `#line`", 1, LAST_LINE),
        Form::Marker => ("this line marker", 0, LAST_MARKED_LINE),
    };
    let refuse = |diagnostics: &mut DiagnosticSink, why: String| {
        let read_as = match form {
            Form::Line => format!(
                "`#line` is read as a line from 1 to {LAST_LINE} in decimal digits, and a \
                 string literal with no prefix after it if it names the file"
            ),
            Form::Marker => format!(
                "a line marker is read as a line from 0 to {LAST_MARKED_LINE} in decimal \
                 digits, a string literal with no prefix if it names the file, and after that \
                 `1` or `2`, then `3`, then `4` after a `3`"
            ),
        };
        diagnostics.report(
            refused(
                text,
                directive,
                UNSUPPORTED_DIRECTIVE,
                &format!("{what} is not read"),
            )
            .with_note(why)
            .with_note(read_as),
        );
    };

    // Replaced as `#if`'s line is, as a use of its own.
    let tokens = condition::line(directive_line, directive, (after, text.len() as u32));
    let errors = diagnostics.error_count();
    begin_use(reading);
    let tokens = replaced(tokens, false, sources, reading, diagnostics);
    if diagnostics.error_count() > errors {
        return;
    }

    let (number, name, flags) = match tokens.as_slice() {
        [(number, _)] => (*number, None, &[][..]),
        [(number, _), (name, _)] => (*number, Some(*name), &[][..]),
        [(number, _), (name, _), flags @ ..] if form == Form::Marker => {
            (*number, Some(*name), flags)
        }
        [] => return refuse(diagnostics, "it names no line".to_owned()),
        _ => return refuse(diagnostics, "it goes on past a line and a name".to_owned()),
    };
    // Decimal digits alone, whatever the token: `parse` takes nothing else
    // but a sign, and no token here begins with one. `08` is eight, as C
    // reads it.
    let digits = sources.snippet(number.span);
    let line = digits
        .parse::<u64>()
        .ok()
        .filter(|line| (lowest..=highest).contains(line));
    let Some(line) = line else {
        return refuse(diagnostics, format!("`{digits}` is not such a line"));
    };
    let name = match name {
        None => None,
        Some(name) => {
            let written = sources.snippet(name.span);
            let Some(read) = destringized(written, Escapes::Every) else {
                return refuse(
                    diagnostics,
                    format!(
                        "`{written}` is not a string literal with no prefix whose escapes make \
                         UTF-8 text"
                    ),
                );
            };
            Some(read)
        }
    };
    // `1` or `2`, then `3`, then `4` after a `3`, as `clang` reads them.
    // Read as any rising run of `1` to `4`, `4` alone and `1 2` were. Found
    // by review.
    let mut read: Vec<u8> = Vec::with_capacity(flags.len());
    for (flag, _) in flags {
        let written = sources.snippet(flag.span);
        let wanted = match (written.parse::<u8>(), read.last()) {
            (Ok(flag @ (1 | 2)), None) | (Ok(flag @ 3), None | Some(1 | 2)) => Some(flag),
            (Ok(4), Some(3)) => Some(4),
            _ => None,
        };
        let Some(flag) = wanted else {
            return refuse(diagnostics, format!("`{written}` is not a flag here"));
        };
        read.push(flag);
    }
    let levels = reading.marker_levels.entry(directive.file()).or_default();
    match read.first() {
        Some(2) if *levels == 0 => {
            return refuse(
                diagnostics,
                "its flag `2` closes a level no flag `1` in its file opened".to_owned(),
            );
        }
        Some(2) => *levels -= 1,
        Some(1) => *levels += 1,
        _ => {}
    }

    // Counted from the line after the directive's, which is line `line`.
    let physical = sources.file(directive.file()).line_index(directive.start()) as i64 + 2;
    let renumbered = reading.renumbered.entry(directive.file()).or_default();
    renumbered.by = line as i64 - physical;
    // `#line N` alone keeps the name an earlier one gave (C17 6.10.4 p4).
    if name.is_some() {
        renumbered.name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The class of `text`, a pragma's text after `pragma`.
    fn classify(text: &str) -> Option<Class> {
        let line = Line::operand(text).expect("a pragma with no comment left open");
        super::classify(&line, 0)
    }

    /// Each class of ADR-0129 is answered for the pragmas it names, the
    /// namespaced ones by their second name, `GCC visibility` refused as
    /// ADR-0132 has it, a name read only where it begins, and no class for
    /// a name the lexer cannot read whole.
    ///
    /// Mutations: move any name to another class, read a namespace's
    /// pragma by its first name alone, read a name past a quote, read a
    /// name a `$`, a universal character name or a letter outside ASCII
    /// touches as the shorter name before it, or refuse one a lone `\` or
    /// white space outside ASCII touches; its assertion fails.
    #[test]
    fn a_pragma_is_classified_as_adr_0129_lists_it() {
        assert_eq!(classify(" once"), Some(Class::Once));
        assert_eq!(classify(" once extra"), Some(Class::Once));
        // `clang` ends a name at a `\` that begins no universal character
        // name, at `@`, and at white space outside ASCII.
        assert_eq!(classify(" once\\q"), Some(Class::Once));
        assert_eq!(classify(" once@"), Some(Class::Once));
        assert_eq!(
            classify("pack\u{a0}(4)"),
            Some(Class::Refused("pack".to_owned()))
        );
        assert_eq!(
            classify("\u{a0}pack(4)"),
            Some(Class::Refused("pack".to_owned()))
        );
        assert_eq!(classify("region \u{521d}"), Some(Class::Ignored));
        assert_eq!(
            classify("GCC error \"x\""),
            Some(Class::Message { error: true })
        );
        assert_eq!(
            classify("GCC warning(\"x\")"),
            Some(Class::Message { error: false })
        );
        for ignored in [
            "STDC FP_CONTRACT ON",
            "STDC FENV_ACCESS OFF",
            "STDC CX_LIMITED_RANGE DEFAULT",
            "GCC diagnostic push",
            "clang diagnostic ignored \"-Wx\"",
            "GCC system_header",
            "warning(disable: 4996)",
            "message(\"hi\")",
            "region x",
            "endregion",
        ] {
            assert_eq!(classify(ignored), Some(Class::Ignored), "{ignored}");
        }
        for (refused, what) in [
            ("pack(4)", "pack"),
            ("align(8)", "align"),
            ("options align=packed", "options align"),
            ("ms_struct on", "ms_struct"),
            ("push_macro(\"X\")", "push_macro"),
            ("pop_macro(\"X\")", "pop_macro"),
            ("redefine_extname a b", "redefine_extname"),
            ("weak f", "weak"),
            ("comment(lib, \"x\")", "comment"),
            ("section(\"s\")", "section"),
            ("code_seg(\"s\")", "code_seg"),
            ("data_seg(\"s\")", "data_seg"),
            ("bss_seg(\"s\")", "bss_seg"),
            ("const_seg(\"s\")", "const_seg"),
            ("init_seg(lib)", "init_seg"),
            ("GCC poison x", "GCC poison"),
            ("GCC visibility push(hidden)", "GCC visibility"),
            (
                "clang attribute push(__attribute__((cold)), apply_to = function)",
                "clang attribute",
            ),
            ("clang section bss=\"b\"", "clang section"),
            ("clang assume_nonnull begin", "clang assume_nonnull"),
        ] {
            assert_eq!(
                classify(refused),
                Some(Class::Refused(what.to_owned())),
                "{refused}"
            );
        }
        for unknown in [
            "",
            "STDC FENV_ROUND FE_UPWARD",
            "GCC optimize(\"O3\")",
            "clang loop vectorize(enable)",
            "options other",
            "omp parallel",
            "\"once\"",
            "(once)",
            "$x",
            "foo @bar",
        ] {
            assert_eq!(classify(unknown), Some(Class::Unknown), "{unknown}");
        }
        // `clang` reads each of these as a longer name than this does, so
        // none is read as the shorter one.
        for glued in ["once$", " once\\u00e9", "GCC poison$ x", "pack\u{e9}(4)"] {
            assert_eq!(classify(glued), None, "{glued}");
        }
    }

    /// `_Pragma`'s literal is read as C17 6.10.9 says, `\"` and `\\` and no
    /// other; a `#line` or line marker's name has every escape read as a
    /// string literal's, as `clang` reads it, and is refused where that is
    /// not UTF-8 or an escape is not C's; and a literal with a prefix is
    /// neither.
    ///
    /// Mutations: keep the quotes, read `\"` as written, keep `\t` in a name
    /// as written, read an octal escape past three digits, read `\u` with
    /// fewer than four digits, or take bytes that are not UTF-8; an
    /// assertion fails.
    #[test]
    fn a_literal_is_destringized_as_its_use_reads_it() {
        let pragma = Escapes::QuotesAndBackslashes;
        let line = Escapes::Every;
        assert_eq!(destringized("\"once\"", pragma).as_deref(), Some("once"));
        assert_eq!(
            destringized(r#""message(\"a\\b\")""#, pragma).as_deref(),
            Some(r#"message("a\b")"#)
        );
        assert_eq!(destringized(r#""a\tb""#, pragma).as_deref(), Some(r"a\tb"));
        assert_eq!(
            destringized(r#""C:\\dir\\x.y""#, line).as_deref(),
            Some(r"C:\dir\x.y")
        );
        assert_eq!(destringized(r#""t\tz.c""#, line).as_deref(), Some("t\tz.c"));
        assert_eq!(
            destringized(r#""\303\251 d'x.h""#, line).as_deref(),
            Some("\u{e9} d'x.h")
        );
        assert_eq!(
            destringized(r#""a\x41\101\0061\?.c""#, line).as_deref(),
            Some("aAA\u{6}1?.c")
        );
        assert_eq!(
            destringized(r#""é\U0001F600.c""#, line).as_deref(),
            Some("\u{e9}\u{1F600}.c")
        );
        assert_eq!(destringized(r#""\u0e9.c""#, line), None);
        assert_eq!(destringized(r#""\303.c""#, line), None);
        assert_eq!(destringized(r#""\x.c""#, line), None);
        assert_eq!(destringized(r#""\q.c""#, line), None);
        assert_eq!(destringized("L\"once\"", pragma), None);
        assert_eq!(destringized("u8\"x.c\"", line), None);
    }
}
