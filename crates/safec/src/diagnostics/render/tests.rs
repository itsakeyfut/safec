use safec_ir::analysis::Conclusion;

/// What a check that could not prove anything reports, for a test that is
/// about what happens to one rather than about how it is built.
///
/// Through `concluded` rather than beside it, so that these tests exercise
/// the path a check takes: an unproven diagnostic has one way to exist and
/// this is it.
fn unproven(message: &str) -> Diagnostic {
    Diagnostic::concluded(
        Conclusion::Unknown,
        message,
        Remedy::new("say more about it"),
    )
    .expect("an unknown conclusion is reported")
}
use super::*;
use clap::Parser as _;

use crate::diagnostics::Code;

fn render(sources: &SourceMap, diagnostic: &Diagnostic) -> String {
    render_with(sources, diagnostic, ColorMode::Never)
}

fn render_with(sources: &SourceMap, diagnostic: &Diagnostic, color: ColorMode) -> String {
    let mut out = Vec::new();
    Renderer::new(color)
        .render(sources, diagnostic, &mut out)
        .expect("writing to a vector cannot fail");
    String::from_utf8(out).expect("the renderer writes text")
}

fn first_line(rendered: &str) -> String {
    rendered.lines().next().unwrap_or_default().to_owned()
}

fn moved_value() -> (SourceMap, Diagnostic) {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int *p = alloc();\nconsume(p);\n*p = 42;\n");
    let diagnostic = Diagnostic::error("use of moved value `p`")
        .with_code(Code::new("SC0601"))
        .with_label(Label::secondary(Span::new(file, 5, 6), "move occurs here"))
        .with_label(Label::primary(Span::new(file, 30, 32), "value used here"))
        .with_note("`p` was moved into `consume`");
    (sources, diagnostic)
}

#[test]
fn a_diagnostic_without_a_label_still_reports_its_message() {
    let sources = SourceMap::new();
    let rendered = render(
        &sources,
        &Diagnostic::error("no input files")
            .with_code(Code::new("SC0001"))
            .with_note("pass at least one `.c` file"),
    );

    assert_eq!(
        rendered,
        "error[SC0001]: no input files\n  = note: pass at least one `.c` file\n"
    );
}

/// The common shape for a diagnostic about the invocation rather than the
/// source.
#[test]
fn a_diagnostic_without_a_code_says_only_the_severity() {
    let sources = SourceMap::new();

    assert_eq!(
        render(&sources, &Diagnostic::error("no input files")),
        "error: no input files\n"
    );
}

/// The header is what an editor's problem matcher reads, so the two
/// rendering paths have to spell it identically. Left to itself `ariadne`
/// writes `Error:` and collapses a note and a help into `Advice:`.
#[test]
fn both_rendering_paths_spell_the_header_the_same_way() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int x;\n");

    for severity in [
        Severity::Error,
        Severity::Warning,
        Severity::Note,
        Severity::Help,
    ] {
        let bare = Diagnostic::new(severity, "m").with_code(Code::new("SC0001"));
        let anchored = bare
            .clone()
            .with_label(Label::primary(Span::new(file, 0, 3), "here"));

        let expected = format!("{}[SC0001]: m", severity.as_str());
        assert_eq!(first_line(&render(&sources, &bare)), expected);
        assert_eq!(first_line(&render(&sources, &anchored)), expected);
    }
}

/// A remedy reaches the reader on both rendering paths, after the notes.
///
/// **Two calls rather than one.** `write_one` writes after `ariadne`'s
/// block and `render_header_only` writes after its own line, so dropping
/// either one is a silence on half the diagnostics rather than on all of
/// them, and a test that rendered only an anchored diagnostic would not
/// see it.
///
/// Mutation: drop the `write_remedies` call from `write_one`. The anchored
/// half fails. Mutation: drop it from `render_header_only`. The bare half
/// fails. Mutation: call `write_remedies` before `write_notes`. The
/// ordering assertion fails and nothing else does.
#[test]
fn a_remedy_is_written_after_the_notes_on_both_rendering_paths() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int x;\n");

    let bare = unproven("m");
    let anchored = bare
        .clone()
        .with_label(Label::primary(Span::new(file, 0, 3), "here"));

    for rendered in [render(&sources, &bare), render(&sources, &anchored)] {
        let help = rendered
            .find("  = help: say more about it")
            .unwrap_or_else(|| panic!("no remedy in {rendered:?}"));
        // An unproven conclusion always carries one, so there is a note to
        // be after. It is what says the result could not be proven.
        let note = rendered
            .find("  = note: ")
            .unwrap_or_else(|| panic!("no note in {rendered:?}"));
        assert!(note < help, "the remedy came before the note: {rendered:?}");
    }
}

/// The check that byte offsets are read as byte offsets. `ariadne` reads a
/// span as character offsets by default, so without `IndexType::Byte` the
/// span below lands a line or more past where it belongs.
#[test]
fn a_span_after_non_ascii_text_points_at_the_right_line() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "/* 日本語日本語日本語 */\nint x;\nint y;\n");
    let start = sources.file(file).contents().find("int x").unwrap() as u32;

    let rendered = render(
        &sources,
        &Diagnostic::error("something about `x`")
            .with_label(Label::primary(Span::new(file, start, start + 5), "here")),
    );

    assert!(rendered.contains("int x"), "{rendered}");
    assert!(!rendered.contains("int y"), "{rendered}");
}

#[test]
fn a_rendered_diagnostic_names_its_file_and_says_what_is_wrong() {
    let (sources, diagnostic) = moved_value();
    let rendered = render(&sources, &diagnostic);

    assert!(rendered.contains("error[SC0601]"), "{rendered}");
    assert!(rendered.contains("use of moved value `p`"), "{rendered}");
    assert!(rendered.contains("<main.c>"), "{rendered}");
    assert!(rendered.contains("move occurs here"), "{rendered}");
    assert!(rendered.contains("value used here"), "{rendered}");
    assert!(
        rendered.contains("  = note: `p` was moved into `consume`"),
        "{rendered}"
    );
}

/// `ariadne` opens a second source block whenever a label sits earlier than
/// the one before it, and heads every block with the anchor's position, so
/// handing over the attachment order produces a block whose header names a
/// line it does not show. The order a check happens to attach its labels in
/// must not change the picture.
#[test]
fn rendering_does_not_depend_on_the_order_labels_were_attached() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int *p = alloc();\nconsume(p);\n*p = 42;\n");
    let moved = Label::secondary(Span::new(file, 5, 6), "move occurs here");
    let used = Label::primary(Span::new(file, 30, 32), "value used here");

    let source_order = Diagnostic::error("m")
        .with_label(moved.clone())
        .with_label(used.clone());
    let primary_first = Diagnostic::error("m").with_label(used).with_label(moved);

    let rendered = render(&sources, &source_order);
    assert_eq!(rendered, render(&sources, &primary_first));
    // One source block, so exactly one header naming the file.
    assert_eq!(rendered.matches("<main.c>").count(), 1, "{rendered}");
}

/// `ariadne` panics on an end one past the file and drops the label without
/// a word on an end far past it. The layer whose job is to report problems
/// has to be total, so the span is moved and the move is disclosed.
#[test]
fn a_span_past_the_end_of_its_file_is_moved_and_noted() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int x;\n");

    for end in [8, 600] {
        let rendered = render(
            &sources,
            &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 0, end), "here")),
        );

        assert!(rendered.contains("int x"), "{rendered}");
        assert!(rendered.contains("here"), "{rendered}");
        assert!(
            rendered.contains("reaches past the end of <main.c> (7 bytes)"),
            "{rendered}"
        );
    }
}

#[test]
fn a_span_inside_a_character_is_moved_and_noted() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "/* 日本 */\n");

    let rendered = render(
        &sources,
        &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 3, 4), "here")),
    );

    assert!(rendered.contains("ends inside a character"), "{rendered}");
    assert!(rendered.contains("here"), "{rendered}");
}

/// The note names both offsets, so a reader can check which end it means.
/// Blaming the end when the start was the one that split invites them to
/// count the bytes and find the compiler wrong about its own diagnostic.
/// In `/* 日本 */`, byte 4 splits the first character and byte 9 is the
/// space, a real boundary.
#[test]
fn a_span_that_starts_inside_a_character_says_so_rather_than_blaming_the_end() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "/* 日本 */\n");

    let rendered = render(
        &sources,
        &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 4, 9), "here")),
    );

    assert!(rendered.contains("starts inside a character"), "{rendered}");
    assert!(!rendered.contains("ends inside a character"), "{rendered}");
}

/// Byte 4 splits the first character and byte 7 splits the second, so both
/// ends moved and the note has to say both.
#[test]
fn a_span_split_at_both_ends_says_both() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "/* 日本 */\n");

    let rendered = render(
        &sources,
        &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 4, 7), "here")),
    );

    assert!(
        rendered.contains("starts and ends inside a character"),
        "{rendered}"
    );
}

/// The two reasons are independent, so a span can have both. Reporting only
/// the one that reached past the end hides that the caret also moved.
#[test]
fn a_span_that_is_both_past_the_end_and_split_says_both() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "/* 日本 */\n");

    let rendered = render(
        &sources,
        &Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 4, 600), "here")),
    );

    assert!(rendered.contains("reaches past the end of"), "{rendered}");
    assert!(rendered.contains("starts inside a character"), "{rendered}");
}

/// A file name, and later an identifier out of the source, is content. A
/// terminal obeys an escape sequence in it, so a name can colour the rest
/// of the report or clear the line above it. Neither rendering path may
/// pass one through, and the colour mode does not get a say: `--color
/// always` means the renderer adds colour, not that content may.
///
/// **A remedy is the fourth place that echoes, and it is here for the same
/// reason the note is.** Every remedy in the tree today is a static string
/// this compiler wrote, so nothing a user controls reaches one yet; the day
/// a remedy names an identifier it will, and that shape has already cost
/// this repository a terminal.
///
/// Mutation: drop `shown` from `write_remedies`. This fails and nothing
/// else does, because every other remedy is plain ASCII.
#[test]
fn content_never_reaches_the_terminal_as_an_instruction() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int x;\n");

    let bare = Diagnostic::error("cannot read `evil\u{1b}[31m.c`");
    let anchored = bare
        .clone()
        .with_label(Label::primary(Span::new(file, 0, 3), "here\u{1b}[32m"))
        .with_note("note\u{1b}[33m");
    // Through `concluded` rather than beside it: that is the one way a
    // remedy is attached, now that `with_remedy` is private.
    let remedied = Diagnostic::concluded(
        Conclusion::Unknown,
        "m",
        Remedy::new("rename `evil\u{1b}[34m.c`"),
    )
    .expect("an unknown conclusion is reported");

    // The remedy goes through both paths, not just the one a diagnostic
    // with nothing to point at takes. They share `write_remedies`, so a
    // mutation to the escaping breaks either; what this fourth case holds
    // is that they go on sharing it.
    let remedied_and_anchored = remedied
        .clone()
        .with_label(Label::primary(Span::new(file, 0, 3), "here"));

    for mode in [ColorMode::Never, ColorMode::Always] {
        for diagnostic in [&bare, &anchored, &remedied, &remedied_and_anchored] {
            let rendered = render_with(&sources, diagnostic, mode);

            assert!(!rendered.contains("evil\u{1b}"), "{mode:?}: {rendered:?}");
            assert!(rendered.contains("evil\\u{1b}"), "{mode:?}: {rendered:?}");
        }
    }
}

/// A file's name reaches the terminal on the header line too.
///
/// `content_never_reaches_the_terminal_as_an_instruction` puts the escape
/// in a message, a label and a note, which are the three things this module
/// writes. It never gives the source map a hostile *name*, so it never
/// exercises the fourth: `ariadne` asks the cache what to call a file and
/// prints the answer in `╭─[ name:line:col ]`, the first structural line of
/// every anchored report.
///
/// A name is not this compiler's text. It comes from a command line today
/// and from a `#include` later, and a `.c` file here has already cleared
/// somebody's terminal when its bytes were echoed verbatim.
///
/// Mutation: drop the `shown` from `SourceMapCache::display`. The escape
/// reaches the header under every colour mode and this fails; before it was
/// written, nothing in either crate did.
#[test]
fn a_file_name_is_escaped_on_the_line_that_locates_a_diagnostic() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("evil\u{1b}[31m.c", "int x;\n");

    let anchored =
        Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 0, 3), "here"));

    for mode in [ColorMode::Never, ColorMode::Always] {
        let rendered = render_with(&sources, &anchored, mode);

        assert!(rendered.contains("evil\\u{1b}"), "{mode:?}: {rendered:?}");
        assert!(!rendered.contains("evil\u{1b}"), "{mode:?}: {rendered:?}");
    }
}

/// The other half of `content_never_reaches_the_terminal_as_an_instruction`,
/// and it needs its own test because the text of a file is echoed by
/// `ariadne` rather than written by this module. Nothing reached this path
/// until a diagnostic had a label to point with, so a `.c` file containing
/// `ESC [ 2 J` could clear the terminal of anyone who compiled it.
#[test]
fn source_text_is_echoed_without_the_control_characters_in_it() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int x = 1; /* \u{1b}[2J */\n");
    let diagnostic =
        Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 4, 5), "here"));

    for mode in [ColorMode::Never, ColorMode::Always] {
        let rendered = render_with(&sources, &diagnostic, mode);

        assert!(!rendered.contains("\u{1b}[2J"), "{mode:?}: {rendered:?}");
    }

    // What replaced it is only contiguous without colour: `ariadne` colours
    // a source line one character at a time, so an escape of its own sits
    // between every pair of them.
    let plain = render_with(&sources, &diagnostic, ColorMode::Never);
    assert!(plain.contains("?[2J"), "{plain:?}");
}

/// One byte out for each byte in. A label span is a byte offset into the
/// text `ariadne` is given, so a replacement of a different length would
/// move every caret after it. Rendering against a file that already has the
/// replacement at that byte is the same picture.
#[test]
fn replacing_a_control_character_does_not_move_the_caret() {
    let mut dirty = SourceMap::new();
    let dirty_file = dirty.add_virtual("main.c", "/* \u{1b} */ int y;\n");
    let mut clean = SourceMap::new();
    let clean_file = clean.add_virtual("main.c", "/* ? */ int y;\n");

    let at = |file| {
        Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 13, 14), "here"))
    };

    assert_eq!(
        render(&dirty, &at(dirty_file)),
        render(&clean, &at(clean_file))
    );
}

/// `\r` is a line separator to `ariadne`, which breaks on it and never
/// emits it, so it does not reach a terminal and must not be replaced.
///
/// Two things would break if it were. A lone `\r` would stop separating,
/// putting both statements onto one line, and every file written on Windows
/// would carry a stray replacement at the end of every line.
#[test]
fn a_carriage_return_still_separates_lines() {
    let mut sources = SourceMap::new();
    let lone = sources.add_virtual("lone.c", "int a;\rint b;\n");
    let crlf = sources.add_virtual("crlf.c", "int a;\r\nint b;\r\n");

    let at = |file| {
        Diagnostic::error("boom").with_label(Label::primary(Span::new(file, 10, 11), "here"))
    };

    let rendered = render(&sources, &at(lone));
    assert!(rendered.contains("<lone.c>:2:"), "{rendered:?}");
    assert!(!rendered.contains(REPLACEMENT), "{rendered:?}");

    let rendered = render(&sources, &at(crlf));
    assert!(!rendered.contains(REPLACEMENT), "{rendered:?}");
}

/// A newline is laid out rather than acted on, and a message that runs to
/// two lines is ordinary, so it is left as it was written.
#[test]
fn a_newline_in_a_message_is_left_alone() {
    let sources = SourceMap::new();

    let rendered = render(&sources, &Diagnostic::error("first\nsecond"));

    assert!(rendered.contains("first\nsecond"), "{rendered:?}");
}

/// A span built against a different source map. The renderer cannot quote
/// what it does not have, but it must not drop the label in silence.
#[test]
fn a_label_naming_a_file_this_renderer_does_not_have_is_noted() {
    let mut sources = SourceMap::new();
    sources.add_virtual("main.c", "int x;\n");
    // Minted by another map, which is the hazard `FileId`'s doc describes
    // and, since ADR-0011, the only way a caller outside `safec_ir` can
    // build one. Its index is past anything this renderer holds.
    let mut elsewhere = SourceMap::new();
    elsewhere.add_virtual("a.c", "");
    let foreign = elsewhere.add_virtual("b.c", "");

    let rendered = render(
        &sources,
        &Diagnostic::error("boom").with_label(Label::primary(Span::new(foreign, 0, 3), "here")),
    );

    assert_eq!(
        rendered,
        "error: boom\n  = note: `here` points into a file this renderer does not have (index 1)\n"
    );
}

#[test]
fn colour_is_emitted_only_when_it_was_asked_for() {
    let (sources, diagnostic) = moved_value();

    let always = render_with(&sources, &diagnostic, ColorMode::Always);
    let never = render_with(&sources, &diagnostic, ColorMode::Never);

    assert!(!never.contains('\u{1b}'), "{never:?}");
    assert!(always.contains('\u{1b}'), "{always:?}");
    // A primary label is coloured by severity and a secondary one is not,
    // so the two must not come out the same.
    assert!(always.contains("\u{1b}[31m"), "{always:?}");
    assert!(always.contains("\u{1b}[34m"), "{always:?}");
}

#[test]
fn every_label_and_note_reaches_the_output() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int *p = alloc();\nconsume(p);\n*p = 42;\n");

    let rendered = render(
        &sources,
        &Diagnostic::error("m")
            .with_label(Label::secondary(Span::new(file, 5, 6), "allocated here"))
            .with_label(Label::secondary(Span::new(file, 18, 25), "moved here"))
            .with_label(Label::primary(Span::new(file, 30, 32), "used here"))
            .with_note("first note")
            .with_note("second note"),
    );

    for expected in [
        "allocated here",
        "moved here",
        "used here",
        "  = note: first note",
        "  = note: second note",
    ] {
        assert!(
            rendered.contains(expected),
            "{expected:?} missing from {rendered}"
        );
    }
    assert!(
        rendered.find("first note") < rendered.find("second note"),
        "{rendered}"
    );
}

/// An unprovable result has to be distinguishable from an ordinary warning,
/// or the reader cannot tell which warnings an annotation would remove.
#[test]
fn an_unproven_diagnostic_says_that_it_could_not_be_proven() {
    let sources = SourceMap::new();

    let unproven = render(&sources, &unproven("`p` may escape"));
    assert!(
        unproven.contains(
            "  = note: this could not be proven; it is an error without `--allow-unknown`"
        ),
        "{unproven}"
    );

    let proven = render(&sources, &Diagnostic::warning("unused variable `x`"));
    assert!(!proven.contains("could not be proven"), "{proven}");
}

/// Once the sink has promoted it the note explains why it is an error, and
/// it cannot name what is responsible: denying is the default wherever a
/// check runs and `--safety strict` sets the same policy, so naming a flag
/// would tell most users that one they never gave is to blame.
///
/// Driven from the command line rather than from a hand-built `Policy`, so
/// that the whole path from an argument to a rendered note is pinned. The
/// bare invocation is one of the rows because since ADR-0033 it is a
/// checked one: `--safety` defaults to `memory`.
///
/// Mutation: name `--allow-unknown` in the promoted arm of
/// `certainty_note`. The last assertion fails on both rows.
#[test]
fn a_promoted_diagnostic_does_not_name_a_flag_the_user_may_not_have_given() {
    for args in [
        &["safec", "--safety", "strict", "a.c"][..],
        &["safec", "a.c"][..],
    ] {
        let options = crate::cli::Cli::try_parse_from(args)
            .unwrap()
            .into_options();
        let mut sink = DiagnosticSink::with_policy(crate::diagnostics::Policy::from(&options));
        sink.report(unproven("`p` may escape"));

        let rendered = render(&SourceMap::new(), &sink.diagnostics()[0]);

        assert!(rendered.starts_with("error: "), "{args:?}: {rendered}");
        assert!(
                rendered.contains(
                    "  = note: this could not be proven, and unproven results are errors in this compilation"
                ),
                "{args:?}: {rendered}"
            );
        assert!(
            !rendered.contains("--allow-unknown"),
            "{args:?} blamed a flag: {rendered}"
        );
    }
}

#[test]
fn a_diagnostic_says_which_safety_check_it_came_from() {
    let sources = SourceMap::new();

    let rendered = render(
        &sources,
        &unproven("`p` may escape").with_safety_level(crate::safety::SafetyLevel::Lifetime),
    );

    assert!(
        rendered.contains("  = note: this check belongs to safety level 2"),
        "{rendered}"
    );
}

/// The reason the renderer takes the map per call instead of holding it. A
/// lexer that meets an `#include` has to add a file while diagnostics are
/// already being reported, and it cannot do that while something borrows
/// the map. This does not compile against a renderer that keeps the borrow,
/// and a renderer that sized its cache once would report the second file as
/// one it does not have.
#[test]
fn a_renderer_does_not_hold_the_source_map() {
    let renderer = Renderer::new(ColorMode::Never);
    let mut sources = SourceMap::new();
    let main = sources.add_virtual("main.c", "int x;\n");

    let mut out = Vec::new();
    renderer
        .render(
            &sources,
            &Diagnostic::error("first").with_label(Label::primary(Span::new(main, 0, 3), "here")),
            &mut out,
        )
        .unwrap();

    // The renderer is still alive, and the map still grows.
    let header = sources.add_virtual("header.h", "int y;\n");
    renderer
        .render(
            &sources,
            &Diagnostic::error("second")
                .with_label(Label::primary(Span::new(header, 0, 3), "there")),
            &mut out,
        )
        .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("<main.c>"), "{rendered}");
    assert!(rendered.contains("<header.h>"), "{rendered}");
    assert!(rendered.contains("there"), "{rendered}");
    assert!(
        !rendered.contains("does not have"),
        "the second file was not found: {rendered}"
    );
}

#[test]
fn render_all_writes_every_diagnostic_in_the_order_they_were_reported() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int x;\n");
    let mut sink = DiagnosticSink::new();
    sink.report(Diagnostic::error("first").with_label(Label::primary(Span::new(file, 0, 3), "a")));
    sink.report(Diagnostic::warning("second"));
    sink.report(Diagnostic::error("third").with_label(Label::primary(Span::new(file, 4, 5), "b")));

    let mut out = Vec::new();
    Renderer::new(ColorMode::Never)
        .render_all(&sources, &sink, &mut out)
        .unwrap();
    let rendered = String::from_utf8(out).unwrap();

    let first = rendered.find("first").expect("first is missing");
    let second = rendered.find("second").expect("second is missing");
    let third = rendered.find("third").expect("third is missing");
    assert!(first < second && second < third, "{rendered}");
}

/// The two paths are one interface, so the header has to match in colour as
/// well as in words. `ariadne` colours `error[SC0001]:` including the colon,
/// and the hand-written path has to put the escapes in the same places.
#[test]
fn both_rendering_paths_colour_the_header_the_same_way() {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("main.c", "int x;\n");

    for severity in [
        Severity::Error,
        Severity::Warning,
        Severity::Note,
        Severity::Help,
    ] {
        let bare = Diagnostic::new(severity, "m").with_code(Code::new("SC0001"));
        let anchored = bare
            .clone()
            .with_label(Label::primary(Span::new(file, 0, 3), "here"));

        let bare = render_with(&sources, &bare, ColorMode::Always);
        let anchored = render_with(&sources, &anchored, ColorMode::Always);

        assert!(first_line(&bare).contains('\u{1b}'), "{bare:?}");
        assert_eq!(first_line(&bare), first_line(&anchored), "{severity:?}");
    }
}
