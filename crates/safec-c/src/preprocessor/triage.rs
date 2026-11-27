//! What a directive's line is, decided in one place from the line alone.
//!
//! The frame loop in `read_includes` asked each question of a line in turn,
//! interleaved with what it did about the answers, and every fix to the
//! preprocessor under #1359 edited it there. Here the questions are asked in
//! the same order and answered as a [`Disposition`], which the frame loop
//! only acts on. A refusal is a [`Refusal`], whose fields are private to this
//! module and whose constructors read what it does to the conditional stack
//! from the line itself, so no refusal door can leave that out: two doors
//! kept it and two did not, and a conditional refused at one of the two was
//! left open, with a false report at its `#endif`. Found by architecture
//! review 13.

use super::line::{Line, spacing};
use super::{
    ConditionalForm, IncludeForm, MacroForm, UNSUPPORTED_DIRECTIVE, conditional_form, directive,
    include_form, macro_form, refused, spliced,
};
use crate::diagnostics::{Code, Diagnostic};
use crate::lexer::UNSUPPORTED_SPLICE;
use safec_ir::source::Span;

/// What the frame loop does with a directive's line.
#[derive(Debug, PartialEq)]
pub(super) enum Disposition<'l> {
    /// Refused, said by its reports.
    Refused(Refusal),
    /// A conditional, acted on, with a splice in it reported first.
    Conditional {
        form: ConditionalForm<'l>,
        splice: Option<Report>,
    },
    /// A line in a skipped group, passed over.
    PassedOver,
    /// A pragma, `#line`, a line marker, `#error` or `#warning`, read by
    /// [`directive::read`].
    Directive,
    /// `#include "name"`, with `quote` the offset of its opening quote.
    Include { name: &'l str, quote: u32 },
    /// A `#define` that is read, as [`MacroForm::Define`] has it.
    Define {
        name: &'l str,
        name_at: u32,
        body: (u32, u32),
        parameters: Option<Vec<&'l str>>,
    },
    /// An `#undef` that is read.
    Undef { name: &'l str },
}

/// One report a refusal makes, at the directive's `#`.
#[derive(Debug, PartialEq)]
pub(super) struct Report {
    code: Code,
    message: String,
    notes: Vec<&'static str>,
}

impl Report {
    fn new(code: Code, message: impl Into<String>, notes: &[&'static str]) -> Report {
        Report {
            code,
            message: message.into(),
            notes: notes.to_vec(),
        }
    }

    /// The diagnostic, for the directive at `directive` whose line is `text`.
    pub(super) fn diagnostic(&self, text: &str, directive: Span) -> Diagnostic {
        self.notes.iter().fold(
            refused(text, directive, self.code, &self.message),
            |report, note| report.with_note(*note),
        )
    }
}

/// What a refused conditional still does to the conditional stack: a
/// refused one, `#if`, `#elif` or an `#ifdef` this cannot read, still opens
/// or ends a level, so its `#endif` is matched rather than reported, and
/// none of its groups from there is compiled. Its own type, so it cannot be
/// handed to what acts on a conditional that is read: carried as a
/// conditional's form beside a flag, setting the flag wrongly evaluated the
/// expression of an `#if` refused for its comment. Found by review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    /// Opens a level, as the directive named.
    Opens(&'static str),
    /// Settles the open level: an `#elif` or an `#else`.
    Settles,
    /// Closes the open level.
    Closes,
}

impl Level {
    fn of(form: &ConditionalForm<'_>) -> Level {
        match form {
            ConditionalForm::If { .. } => Level::Opens("#if"),
            ConditionalForm::Ifdef { negated: false, .. } => Level::Opens("#ifdef"),
            ConditionalForm::Ifdef { negated: true, .. } => Level::Opens("#ifndef"),
            ConditionalForm::Elif { .. } | ConditionalForm::Else { .. } => Level::Settles,
            ConditionalForm::Endif { .. } => Level::Closes,
        }
    }
}

/// A refused line: its reports, and the level it still keeps.
#[derive(Debug, PartialEq)]
pub(super) struct Refusal {
    reports: Vec<Report>,
    level: Option<Level>,
}

impl Refusal {
    /// A refusal of `line`, keeping the level of the conditional the line
    /// reads as, if it reads as one.
    ///
    /// The level is what the line reads as and not what its directive's word
    /// is: `clang` reads `#endif$` and `#endif\N{...}` naming a letter as
    /// unknown directives that close nothing.
    fn new(line: &Line, reports: Vec<Report>) -> Refusal {
        Refusal {
            reports,
            level: conditional_form(line).as_ref().map(Level::of),
        }
    }

    /// A refusal of `line`, refused for white space outside ASCII after its
    /// `#`, keeping the level of the conditional it reads as with that white
    /// space made spaces of the same length.
    fn respaced(line: &Line, reports: Vec<Report>) -> Refusal {
        let spaced: String = line
            .text()
            .chars()
            .map(|c| {
                if spacing(c) {
                    " ".repeat(c.len_utf8())
                } else {
                    c.to_string()
                }
            })
            .collect();
        let level = Line::read(&spaced)
            .ok()
            .and_then(|plain| conditional_form(&plain).as_ref().map(Level::of));
        Refusal { reports, level }
    }

    /// A refusal of `line`, refused for characters before its `#`, keeping
    /// the level of the conditional it reads as with them made spaces, as
    /// `clang` reads a NUL there. Kept as no level, a NUL before an `#endif`
    /// in a correct file left its `#if` open, and every line after it that
    /// was right was reported. Found by review. Where the character is one
    /// `clang` reads as no directive, the level is one too many or too few;
    /// the line is refused either way.
    fn unprefixed(line: &Line, reports: Vec<Report>) -> Refusal {
        let level = line.hash().and_then(|(start, _)| {
            let plain = format!("{}{}", " ".repeat(start), &line.text()[start..]);
            Line::read(&plain)
                .ok()
                .and_then(|plain| conditional_form(&plain).as_ref().map(Level::of))
        });
        Refusal { reports, level }
    }

    /// What the refusal says.
    pub(super) fn reports(&self) -> &[Report] {
        &self.reports
    }

    /// The level the refused line still keeps.
    pub(super) fn level(&self) -> Option<Level> {
        self.level
    }
}

/// What `line`, a directive's line, is: `ended` where every comment in it
/// ends on it, as [`Line::read`] says, and `skipping` in a group that is
/// skipped. The questions are asked in this order, and the order is part of
/// the answer: a line the lexer joins to the next is refused before a
/// skipped group passes it over, and a name not read is refused only in a
/// group that is compiled.
pub(super) fn triage(line: &Line, ended: bool, skipping: bool) -> Disposition<'_> {
    let text = line.text();
    // A splice in a comment is a comment going on past the line: C splices
    // lines before it reads comments (C17 5.1.1.2 p1, phases 2 and 3), so
    // `// \` carries the comment, and not code, onto the line below.
    if !ended {
        return Disposition::Refused(Refusal::new(
            line,
            vec![Report::new(
                UNSUPPORTED_DIRECTIVE,
                "a comment that goes on past a directive's line is not read yet",
                &[
                    "C makes the comment one space, so the directive goes on after it ends, and \
                   the lines inside it are not code",
                ],
            )],
        ));
    }
    // Refused in every group, a skipped one included, since what `clang`
    // makes of it depends on the character it names or on the group it is
    // in. See `Name::Undecided`.
    if line.undecided() {
        return Disposition::Refused(Refusal::new(
            line,
            vec![Report::new(
                UNSUPPORTED_DIRECTIVE,
                "this directive's name goes on in an escape that is not read yet",
                &[
                    "`clang` reads a `\\N{...}` by the character it names, and a universal \
                   character name past U+10FFFF by the group it is in",
                ],
            )],
        ));
    }
    // Characters before the `#` that no token can begin: what `clang` makes
    // of the line depends on which they are, a NUL being white space to it,
    // so the line is refused in every group before it can be read as a
    // conditional. See ADR-0134.
    if line.prefixed() {
        return Disposition::Refused(Refusal::unprefixed(
            line,
            vec![Report::new(
                UNSUPPORTED_DIRECTIVE,
                "a character before this directive's `#` is not read",
                &[
                    "`clang` reads a NUL there as white space and the line as a directive, \
                     and other characters there as no directive",
                ],
            )],
        ));
    }
    let form = conditional_form(line);
    // A splice joins the line below to this one before any directive is read
    // (C17 5.1.1.2 p1, phase 2), and the lexer read that line on its own, as
    // a directive where it begins with `#`. So it is refused wherever a
    // directive is read at all, a skipped group's included: there `#define X
    // \` above `#else` ended a group C does not end, and the code after it was
    // compiled where C has none. Found by review.
    let splice = (spliced(text) && (skipping || form.is_some())).then(|| {
        Report::new(
            UNSUPPORTED_SPLICE,
            if form.is_some() {
                "a line splice or a trigraph in a conditional directive is not read yet"
            } else {
                "a line splice or a trigraph in a directive in a skipped group is not read yet"
            },
            &["C can join the line below to this one, and it would be read on its own instead"],
        )
    });
    if let Some(form) = form {
        return Disposition::Conditional { form, splice };
    }
    // A group that is skipped has its directives read only as far as their
    // names, to count the conditionals in it, and the rest ignored (C17
    // 6.10.1 p6): `#include <stdio.h>` or a function-like `#define` there is
    // not reported.
    if skipping {
        let mut reports: Vec<Report> = splice.into_iter().collect();
        // Passed over only where its start is proved to read as `clang`
        // reads it, so a spelling nobody listed is refused rather than
        // misread: `#` and then a NUL and `else` was passed over where
        // `clang` takes the group. White space outside ASCII after the `#`
        // is proved too: it makes no directive to `clang` in a skipped group,
        // which passes the line over, as this does. See ADR-0134.
        if !line.proved_start() && !line.spaced_oddly() {
            reports.push(Report::new(
                UNSUPPORTED_DIRECTIVE,
                "this line in a group that is skipped is not read",
                &["a character before its name may make it a directive `clang` acts on"],
            ));
        } else if directive::chooses_a_group(line) {
            reports.push(Report::new(
                UNSUPPORTED_DIRECTIVE,
                "`#elifdef` and `#elifndef` are not read yet",
                &[
                    "C23 reads them as choosing a group, and passed over, the group one chose \
                   would not be compiled",
                ],
            ));
        }
        return if reports.is_empty() {
            Disposition::PassedOver
        } else {
            Disposition::Refused(Refusal::new(line, reports))
        };
    }
    // Not read as a shorter name: `#pragma once$` is not `#pragma once` to
    // `clang`. Found by architecture review 12.
    if line.spaced_oddly() {
        return Disposition::Refused(Refusal::respaced(
            line,
            vec![Report::new(
                UNSUPPORTED_DIRECTIVE,
                "white space outside ASCII after `#` makes no directive",
                &[
                    "`clang -E` refuses such a line, and `clang` reads it as a directive; after a \
                   directive's name it is white space",
                ],
            )],
        ));
    }
    if line.directive().is_none() {
        return Disposition::Refused(Refusal::new(
            line,
            vec![Report::new(
                UNSUPPORTED_DIRECTIVE,
                "this directive's name goes on past what is read",
                &[directive::GLUED_NOTE],
            )],
        ));
    }
    // A null directive has no effect (C17 6.10.7), so it is passed over in
    // a compiled group as in a skipped one; it was refused as a directive
    // not read. Found by the differential corpus of directive lines.
    if line.null() {
        return Disposition::PassedOver;
    }
    if directive::owns(line) {
        return Disposition::Directive;
    }
    let refuse = |report: Report| Disposition::Refused(Refusal::new(line, vec![report]));
    match include_form(line) {
        IncludeForm::Quoted { name, quote } => return Disposition::Include { name, quote },
        IncludeForm::Spliced => {
            return refuse(Report::new(
                UNSUPPORTED_SPLICE,
                "a line splice or a trigraph in an `#include` is not read yet",
                &[
                    "C can join the line below to this one, and it would be compiled as code \
                   instead",
                ],
            ));
        }
        IncludeForm::Angled => {
            return refuse(Report::new(
                UNSUPPORTED_DIRECTIVE,
                "`#include <...>` is not read yet",
                &["only `#include \"...\"` is read yet"],
            ));
        }
        IncludeForm::Unread => {
            return refuse(Report::new(
                UNSUPPORTED_DIRECTIVE,
                "this `#include` is not read yet",
                &[
                    "only `#include \"...\"` is read yet, with nothing but a comment after it on \
                   its line",
                ],
            ));
        }
        IncludeForm::Other => {}
    }
    match macro_form(line) {
        MacroForm::Define {
            name,
            name_at,
            body,
            parameters,
        } => Disposition::Define {
            name,
            name_at,
            body,
            parameters,
        },
        MacroForm::Undef { name } => Disposition::Undef { name },
        MacroForm::Unread { directive } => refuse(Report::new(
            UNSUPPORTED_DIRECTIVE,
            format!("this `{directive}` is not read yet"),
            &[if directive == "#undef" {
                "an `#undef` is read when its name is ASCII letters, digits and `_`, with \
                 nothing after it"
            } else {
                "a `#define` is read when its name is ASCII letters, digits and `_`, followed \
                 by a space, `(` or the end of the line"
            }],
        )),
        MacroForm::Defined => refuse(Report::new(
            UNSUPPORTED_DIRECTIVE,
            "`defined` is not read as a macro's name",
            &["C17 6.10.8 p2 says it shall not be the subject of a `#define` or an `#undef`"],
        )),
        MacroForm::Parameters => refuse(Report::new(
            UNSUPPORTED_DIRECTIVE,
            "this parameter list is not read",
            &[
                "a function-like macro's parameters are read when each is a name of ASCII \
               letters, digits and `_`, none named twice, between `(` and `)`",
            ],
        )),
        MacroForm::Variadic => refuse(Report::new(
            UNSUPPORTED_DIRECTIVE,
            "this variadic parameter list is not read",
            &[
                "`...` is read as the last parameter; a named one such as `args...` is not read \
               yet",
            ],
        )),
        MacroForm::Spliced => refuse(Report::new(
            UNSUPPORTED_SPLICE,
            "a line splice or a trigraph in a `#define` is not read yet",
            &[
                "C can join the line below to this one, and it would be compiled as code \
               instead",
            ],
        )),
        // Which directives, rather than which stage. `docs/frontend.md` puts
        // the preprocessor in its Stage 3 and `docs/roadmap.md` pulls four of
        // them forward, so a stage number is both a number the reader does
        // not have and one that stopped being the whole answer.
        MacroForm::Other => refuse(Report::new(
            UNSUPPORTED_DIRECTIVE,
            "preprocessor directives are not supported yet",
            &[
                "`#include \"...\"`, `#define` and `#undef`, the conditional directives, \
                 `#pragma`, `#line` and line markers, `#error` and `#warning` are read",
                "the line is one token and is not compiled",
            ],
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> (Line, bool) {
        match Line::read(text) {
            Ok(line) => (line, true),
            Err(line) => (line, false),
        }
    }

    fn kind(text: &str, skipping: bool) -> &'static str {
        let (line, ended) = read(text);
        match triage(&line, ended, skipping) {
            Disposition::Refused(_) => "refused",
            Disposition::Conditional { splice: None, .. } => "conditional",
            Disposition::Conditional {
                splice: Some(_), ..
            } => "spliced conditional",
            Disposition::PassedOver => "passed over",
            Disposition::Directive => "directive",
            Disposition::Include { .. } => "include",
            Disposition::Define { .. } | Disposition::Undef { .. } => "macro",
        }
    }

    /// Each kind of line goes where the frame loop read it before the
    /// questions moved here, in a group that is compiled and in one that is
    /// skipped.
    ///
    /// Mutations: pass a skipped group's line over before its splice is
    /// asked, send a pragma to the macro readers, or read an `#include` in a
    /// skipped group; an assertion fails. Or read the carriage return that
    /// ends a CRLF line as a character after its `#`; the `#\r` rows fail.
    /// Or refuse a null directive in a compiled group; the `#` rows fail.
    /// Or let white space outside ASCII after a null directive's `#` pass;
    /// the `#\u{a0}` row fails.
    #[test]
    fn each_line_goes_where_its_kind_is_read() {
        for (text, compiled, skipped) in [
            ("#if 1", "conditional", "conditional"),
            ("#endif \\", "spliced conditional", "spliced conditional"),
            ("#define X 1", "macro", "passed over"),
            ("#define X \\", "refused", "refused"),
            ("#undef X", "macro", "passed over"),
            ("#include \"a.h\"", "include", "passed over"),
            ("#include <a.h>", "refused", "passed over"),
            ("#pragma once", "directive", "passed over"),
            ("#line 4", "directive", "passed over"),
            ("# 4 \"a.c\"", "directive", "passed over"),
            ("#error x", "directive", "passed over"),
            ("#elifdef X", "refused", "refused"),
            ("#warn x", "refused", "passed over"),
            ("#define X /* open", "refused", "refused"),
            ("#endif$", "refused", "refused"),
            ("#\u{a0}define X", "refused", "passed over"),
            ("#endif\\N{NO-BREAK SPACE}", "refused", "refused"),
            ("\0#endif", "refused", "refused"),
            ("#endif\r", "conditional", "conditional"),
            ("#", "passed over", "passed over"),
            ("%: ", "passed over", "passed over"),
            ("#\r", "passed over", "passed over"),
            ("# \t\r", "passed over", "passed over"),
            ("# /* c */", "passed over", "passed over"),
            ("#\u{a0}", "refused", "refused"),
            // A null directive only because the lexer leaves a `\` as a
            // character: a lexer that read the splice would join the line
            // below, which `clang` reads as a directive, and pass both over.
            ("#\\", "refused", "refused"),
        ] {
            assert_eq!(kind(text, false), compiled, "{text:?} compiled");
            assert_eq!(kind(text, true), skipped, "{text:?} skipped");
        }
    }

    /// A refusal keeps the level of the conditional its line reads as, at
    /// every door: a comment going on past it, white space outside ASCII
    /// after its `#`, read with that white space made spaces. And none where
    /// the line reads as no conditional, as `clang` reads `#endif$` and
    /// `#endif\N{...}` naming a letter as unknown directives.
    ///
    /// And with characters before its `#` made spaces, as `clang` reads a
    /// NUL there.
    ///
    /// Mutations: give no level in `Refusal::new`, `Refusal::respaced` or
    /// `Refusal::unprefixed`, or read the level from the directive's word;
    /// an assertion fails.
    #[test]
    fn a_refusal_keeps_the_level_its_line_reads_as() {
        let level = |text: &str| {
            let (line, ended) = read(text);
            match triage(&line, ended, false) {
                Disposition::Refused(refusal) => refusal.level(),
                other => panic!("{text:?} is not refused: {other:?}"),
            }
        };
        assert_eq!(level("#if 1 /* open"), Some(Level::Opens("#if")));
        assert_eq!(level("#\u{a0}endif"), Some(Level::Closes));
        assert_eq!(level("#\u{a0}ifndef X"), Some(Level::Opens("#ifndef")));
        assert_eq!(level("#\u{a0}else"), Some(Level::Settles));
        assert_eq!(level("#endif$"), None);
        assert_eq!(level("#endif\\N{LATIN SMALL LETTER E WITH ACUTE}"), None);
        assert_eq!(level("#define X\\u{"), None);
        assert_eq!(level("\0#endif"), Some(Level::Closes));
        assert_eq!(level("\0/* x */ #if 1"), Some(Level::Opens("#if")));
        assert_eq!(level("\u{1f600}#else"), Some(Level::Settles));
    }
}
