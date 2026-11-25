//! `#pragma` and `_Pragma`, `#line`, `#error` and `#warning`: the directives
//! that choose no group and define no macro (C17 6.10.4 to 6.10.6 and
//! 6.10.9, and C23's `#warning`). Each is read only in a group that is
//! compiled; in a skipped one a directive is read only as far as its name.

use super::{
    Frame, Reading, UNSUPPORTED_DIRECTIVE, Uncollected, begin_use, canonical, collected, condition,
    hash, refused, replaced, spliced, too_deep, used_at,
};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::lexer::UNSUPPORTED_SPLICE;
use crate::token::{Token, TokenKind};
use safec_ir::source::{FileName, SourceMap, Span};

/// A pragma this does not read, ignored as C17 6.10.6 p1 allows. See
/// ADR-0129.
const UNREAD_PRAGMA: Code = Code::new("SC0118");
/// `#error` in a group that is compiled (C17 6.10.5).
const ERROR_DIRECTIVE: Code = Code::new("SC0121");
/// `#warning` in a group that is compiled (C23 6.10.7).
const WARNING_DIRECTIVE: Code = Code::new("SC0122");

/// Read `text`, the line of the directive at `directive` with its comments
/// made spaces, if it is one of this module's, and say whether it was.
pub(super) fn read(
    text: &str,
    directive: Span,
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) -> bool {
    let Some((word, rest)) = name(text) else {
        return false;
    };
    if !matches!(word, "pragma" | "line" | "error" | "warning") {
        return false;
    }
    // A splice joins the line below to this one before any directive is
    // read (C17 5.1.1.2 p1, phase 2), and the lexer read that line on its
    // own, as code: `#pragma GCC diagnostic \` would leave `push` behind.
    if spliced(text) {
        diagnostics.report(
            refused(
                text,
                directive,
                UNSUPPORTED_SPLICE,
                &format!("a line splice or a trigraph in a `#{word}` is not read yet"),
            )
            .with_note("C can join the line below to this one, and it would be read on its own"),
        );
        return true;
    }
    match word {
        "pragma" => pragma(rest, hash(text, directive), sources, reading, diagnostics),
        "line" => line(
            (text, directive),
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

/// A directive's name and what follows it, of `text`, a line beginning
/// with `#` or `%:`; `None` where the name goes on in a character an
/// identifier may hold, as `conditional_form` ends one.
fn name(text: &str) -> Option<(&str, &str)> {
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let rest = text.strip_prefix('#').or_else(|| text.strip_prefix("%:"))?;
    let rest = rest.trim_start_matches([' ', '\t', '\u{b}', '\u{c}', '\r']);
    let length = rest.len() - rest.trim_start_matches(identifier).len();
    let (word, rest) = rest.split_at(length);
    let ends = !rest.starts_with(|c: char| identifier(c) || c == '$' || c == '\\' || !c.is_ascii());
    ends.then_some((word, rest))
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

/// The first two names of the pragma `rest`, the text after `pragma`, and
/// what follows them. A name is read only where it begins, after white
/// space: `#pragma "once"` has none, where reading past the quote made it
/// `once` and a header `clang` reads twice was read once. Found by review.
fn names(rest: &str) -> (&str, &str, &str) {
    fn name(text: &str) -> (&str, &str) {
        let text = text.trim_start_matches([' ', '\t', '\u{b}', '\u{c}', '\r']);
        let length = text.len()
            - text
                .trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_')
                .len();
        text.split_at(length)
    }
    let (first, rest) = name(rest);
    let (second, rest) = if first.is_empty() {
        ("", rest)
    } else {
        name(rest)
    };
    (first, second, rest)
}

/// The class of the pragma `rest`, the text after `pragma`: by its first
/// name, and by the name after it where the first is `STDC`, `GCC` or
/// `clang`, which are namespaces, or `options`.
fn classify(rest: &str) -> Class {
    let (first, second, _) = names(rest);
    match (first, second) {
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
        ("GCC", "poison" | "visibility") | ("clang", "assume_nonnull") | ("options", "align") => {
            Class::Refused(format!("{first} {second}"))
        }
        (
            "pack" | "align" | "ms_struct" | "push_macro" | "pop_macro" | "redefine_extname"
            | "weak" | "comment" | "section" | "code_seg" | "data_seg" | "bss_seg" | "const_seg"
            | "init_seg",
            _,
        ) => Class::Refused(first.to_owned()),
        _ => Class::Unknown,
    }
}

/// Act on the pragma `rest`, the text after `pragma`, written at `at`: a
/// `#pragma`'s `#`, or a `_Pragma`'s string literal.
fn pragma(
    rest: &str,
    at: Span,
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) {
    match classify(rest) {
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
            let (.., after) = names(rest);
            let after = after.trim();
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
    sources: &mut SourceMap,
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
    match read {
        Some((literal, text)) => pragma(&text, literal.span, sources, reading, diagnostics),
        None => unread(diagnostics),
    }
}

/// Which escape sequences [`destringized`] reads in a literal.
#[derive(Clone, Copy)]
enum Escapes {
    /// `\"` and `\\` read, every other kept as written: what `_Pragma`
    /// does (C17 6.10.9 p1).
    QuotesAndBackslashes,
    /// `\"` and `\\` read, any other refused: a `#line` name, which
    /// `clang` reads every escape of, `"t\tz.c"` a tab. Read as written,
    /// its `__FILE__` was another string than `clang`'s.
    OnlyQuotesAndBackslashes,
}

/// The text of `literal`, a string literal with no prefix, its quotes
/// dropped and its escapes read as `escapes` says; `None` for any other.
fn destringized(literal: &str, escapes: Escapes) -> Option<String> {
    let body = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut text = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match (chars.next()?, escapes) {
            (escaped @ ('"' | '\\'), _) => text.push(escaped),
            (other, Escapes::QuotesAndBackslashes) => {
                text.push('\\');
                text.push(other);
            }
            (_, Escapes::OnlyQuotesAndBackslashes) => return None,
        }
    }
    Some(text)
}

/// The greatest line `#line` may name (C17 6.10.4 p3).
const LAST_LINE: u64 = 2_147_483_647;

/// `#line N` or `#line N "name"`, the line `text` of the directive at
/// `directive`, its operands from `after`, macro-replaced first (C17
/// 6.10.4 p5): the line after it is line `N` to `__LINE__`, and the file
/// is `name` to `__FILE__`, and to nothing else. A diagnostic points at the
/// line where it is in the file, the text a reader can open.
fn line(
    (text, directive): (&str, Span),
    after: u32,
    sources: &mut SourceMap,
    reading: &mut Reading,
    diagnostics: &mut DiagnosticSink,
) {
    let refuse = |diagnostics: &mut DiagnosticSink, why: String| {
        diagnostics.report(
            refused(
                text,
                directive,
                UNSUPPORTED_DIRECTIVE,
                "this `#line` is not read",
            )
            .with_note(why)
            .with_note(format!(
                "`#line` is read as a line from 1 to {LAST_LINE} in decimal digits, and a \
                 string literal with no prefix after it if it names the file"
            )),
        );
    };

    // Replaced as `#if`'s line is, as a use of its own.
    let tokens = condition::line(text, directive, (after, text.len() as u32));
    let errors = diagnostics.error_count();
    begin_use(reading);
    let tokens = replaced(tokens, false, sources, reading, diagnostics);
    if diagnostics.error_count() > errors {
        return;
    }

    let (number, name) = match tokens.as_slice() {
        [(number, _)] => (*number, None),
        [(number, _), (name, _)] => (*number, Some(*name)),
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
        .filter(|line| (1..=LAST_LINE).contains(line));
    let Some(line) = line else {
        return refuse(diagnostics, format!("`{digits}` is not such a line"));
    };
    let name = match name {
        None => None,
        Some(name) => {
            let written = sources.snippet(name.span);
            let Some(read) = destringized(written, Escapes::OnlyQuotesAndBackslashes) else {
                return refuse(
                    diagnostics,
                    format!(
                        "`{written}` is not a string literal with no prefix and no escape \
                         but `\\\\` and `\\\"`"
                    ),
                );
            };
            Some(read)
        }
    };

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

    /// Each class of ADR-0129 is answered for the pragmas it names, the
    /// namespaced ones by their second name, `GCC visibility` refused as
    /// ADR-0132 has it, and a name read only where it begins.
    ///
    /// Mutations: move any name to another class, read a namespace's
    /// pragma by its first name alone, or read a name past a quote; its
    /// assertion fails.
    #[test]
    fn a_pragma_is_classified_as_adr_0129_lists_it() {
        assert_eq!(classify(" once"), Class::Once);
        assert_eq!(classify(" once extra"), Class::Once);
        assert_eq!(classify("GCC error \"x\""), Class::Message { error: true });
        assert_eq!(
            classify("GCC warning(\"x\")"),
            Class::Message { error: false }
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
            assert_eq!(classify(ignored), Class::Ignored, "{ignored}");
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
            ("clang assume_nonnull begin", "clang assume_nonnull"),
        ] {
            assert_eq!(
                classify(refused),
                Class::Refused(what.to_owned()),
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
        ] {
            assert_eq!(classify(unknown), Class::Unknown, "{unknown}");
        }
    }

    /// `_Pragma`'s literal is read as C17 6.10.9 says, `\"` and `\\` and no
    /// other; a `#line` name with any other escape is refused, since `clang`
    /// reads it as something else; and a literal with a prefix is neither.
    ///
    /// Mutations: keep the quotes, read `\"` as written, or keep `\t` in a
    /// `#line` name; an assertion fails.
    #[test]
    fn a_literal_is_destringized_as_its_use_reads_it() {
        let pragma = Escapes::QuotesAndBackslashes;
        let line = Escapes::OnlyQuotesAndBackslashes;
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
        assert_eq!(destringized(r#""t\tz.c""#, line), None);
        assert_eq!(destringized("L\"once\"", pragma), None);
        assert_eq!(destringized("u8\"x.c\"", line), None);
    }

    /// A directive's name begins after any white space C allows there and
    /// ends where no identifier goes on: `$`, a `\` that begins a universal
    /// character name, or a letter outside ASCII would.
    ///
    /// Mutations: end it at the first character that is not a letter,
    /// digit or `_`, or at any one of those three; or skip only spaces and
    /// tabs after the `#`. An assertion fails.
    #[test]
    fn a_directive_name_is_the_whole_identifier() {
        assert_eq!(name("#  pragma once"), Some(("pragma", " once")));
        assert_eq!(name("#\u{b}\u{c}line 4"), Some(("line", " 4")));
        assert_eq!(name("%:line 4"), Some(("line", " 4")));
        assert_eq!(name("#pragmas"), Some(("pragmas", "")));
        assert_eq!(name("#line$ 4"), None);
        assert_eq!(name("#line\\u00e9 4"), None);
        assert_eq!(name("#lineé 4"), None);
    }
}
