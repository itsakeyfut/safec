use clap::ValueEnum;
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

/// Each conclusion is reported the way `docs/safety-model.md` says.
///
/// The three written out rather than walked, because a test
/// that asks `concluded` what it answers agrees with it whatever it
/// answers. These rows are the document's table copied by a reader, which
/// is the only thing that can disagree with the code.
///
/// What holds the *fourth* conclusion is `error[E0004]` in `concluded`
/// rather than anything here. Nothing makes somebody add a row to this
/// table: a compiler that makes you look does not make you right.
///
/// Mutation: answer `Some` for `Safe`. The first row fails, and only it.
/// Mutation: give `Unsafe` a warning's severity. The second row fails, and
/// only it. Mutation: build `Unknown` with `Diagnostic::error`, so that its
/// certainty is `Proven`. Eight tests fail, this among them: everything
/// that watches the sink promote an unprovable result is downstream of this
/// arm, which is what the third row is worth.
///
/// **The remedy is checked on both reported rows and not only on one.** It
/// is attached by a different expression in each arm, so an arm that
/// dropped it would be a silence in exactly one of the two. Mutation: drop
/// the `with_remedy` call from the `Unsafe` arm. The second row fails and
/// the third does not.
#[test]
fn every_conclusion_is_reported_the_way_the_model_says() {
    assert_eq!(
        Diagnostic::concluded(Conclusion::Safe, "nothing to say", Remedy::new("nothing")),
        None
    );

    let unsafe_ = Diagnostic::concluded(
        Conclusion::Unsafe,
        "use of freed value `p`",
        Remedy::new("move the free after this use"),
    )
    .expect("an unsafe conclusion is reported");
    assert_eq!(unsafe_.severity(), Severity::Error);
    assert_eq!(unsafe_.certainty(), Certainty::Proven);
    assert_eq!(unsafe_.message(), "use of freed value `p`");
    assert_eq!(
        unsafe_
            .remedies()
            .iter()
            .map(Remedy::message)
            .collect::<Vec<_>>(),
        ["move the free after this use"]
    );

    let unknown = Diagnostic::concluded(
        Conclusion::Unknown,
        "`p` may escape",
        Remedy::new("keep `p` in one local"),
    )
    .expect("an unknown conclusion is reported");
    assert_eq!(unknown.severity(), Severity::Warning);
    assert_eq!(unknown.certainty(), Certainty::Unproven);
    assert_eq!(unknown.message(), "`p` may escape");
    assert_eq!(
        unknown
            .remedies()
            .iter()
            .map(Remedy::message)
            .collect::<Vec<_>>(),
        ["keep `p` in one local"]
    );
}
use super::*;
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

fn span(start: u32, end: u32) -> Span {
    // A handle out of a real map, because `FileId::from_index` belongs to
    // `safec_ir` and is not `pub`: a handle is only meaningful against the
    // map it came from, and ADR-0011 made that boundary a crate boundary.
    let mut sources = SourceMap::new();
    Span::new(sources.add_virtual("t.c", ""), start, end)
}

#[test]
fn severities_are_ordered_from_least_to_most_serious() {
    assert!(Severity::Help < Severity::Note);
    assert!(Severity::Note < Severity::Warning);
    assert!(Severity::Warning < Severity::Error);
}

/// The chain above pins the order; this pins what the order is *for*. A
/// filter asks `severity >= Severity::Warning` for "warning or worse", and
/// the worst thing in a set is its maximum. Both read backwards if the
/// variants are declared the other way round.
#[test]
fn the_worst_severity_in_a_set_is_its_maximum() {
    let reported = [Severity::Warning, Severity::Error, Severity::Note];

    assert_eq!(reported.iter().max(), Some(&Severity::Error));
    assert_eq!(reported.iter().min(), Some(&Severity::Note));

    let warning_or_worse: Vec<_> = reported
        .iter()
        .filter(|severity| **severity >= Severity::Warning)
        .collect();
    assert_eq!(warning_or_worse, [&Severity::Warning, &Severity::Error]);
}

/// Named for the rule rather than for today's variants: "only an error"
/// would stop being true the day the enum gains the more serious variant
/// its own doc comment invites, which is the day this has to keep holding.
///
/// The expected column is written out rather than computed, because a
/// table built the way the code builds one compares the code with itself,
/// and that matters more here than usual: `is_error` answers by
/// comparing against `Self::Error`, so a column that did the same would be
/// the function checked against itself and would hold whatever the function
/// said. These four answers are what the compiler is supposed to do, stated
/// independently of how it works it out.
///
/// What this cannot do is cover a variant that does not exist yet. Nothing
/// makes anybody add a row, and `error[E0004]` points at `is_error` and at
/// `render.rs`, never here. That is why the answer in `is_error` is the
/// ordering rather than an arm: the row a new variant never gets is a row
/// nothing needed, because there is no arm to write wrongly.
///
/// Mutation: change `self >= Self::Error` to `self > Self::Error`, or to
/// `>= Self::Warning`. The row for `Error` fails on the first and the row
/// for `Warning` on the second, by name.
#[test]
fn a_severity_at_or_above_error_fails_the_compilation() {
    for (severity, fails_the_build) in [
        (Severity::Help, false),
        (Severity::Note, false),
        (Severity::Warning, false),
        (Severity::Error, true),
    ] {
        assert_eq!(severity.is_error(), fails_the_build, "{severity}");
    }
}

/// These words appear in every diagnostic header, so editors and test
/// harnesses read them.
#[test]
fn severities_are_spelled_the_way_a_header_spells_them() {
    assert_eq!(Severity::Error.to_string(), "error");
    assert_eq!(Severity::Warning.to_string(), "warning");
    assert_eq!(Severity::Note.to_string(), "note");
    assert_eq!(Severity::Help.to_string(), "help");
}

/// The shape `docs/diagnostics.md` allocates, held where a code is made.
///
/// Every code in the crate is a `const`, so this fires during compilation
/// and the build stops at the declaration: `const C: Code =
/// Code::new("E0301");` is `error[E0080]`, which is the whole reason the
/// check is inside a `const fn`. That is what a test cannot demonstrate, so
/// this one reaches the same assertion the other way, through a call the
/// compiler cannot fold.
///
/// Mutation: delete either `assert!`. The matching case below stops
/// panicking and the test fails.
#[test]
fn a_code_is_sc_and_four_digits_or_it_is_not_a_code() {
    for refused in ["E0301", "SC030", "SC03011", "SC030a", "sc0301", "", "SC"] {
        assert!(
            std::panic::catch_unwind(|| Code::new(refused)).is_err(),
            "{refused:?} was accepted"
        );
    }

    assert_eq!(Code::new("SC0301").as_str(), "SC0301");
}

#[test]
fn a_diagnostic_carries_what_it_was_built_with() {
    let diagnostic = Diagnostic::error("use of freed value `p`")
        .with_code(Code::new("SC0601"))
        .with_label(Label::secondary(span(0, 4), "freed here"))
        .with_label(Label::primary(span(10, 12), "used here"))
        .with_note("`p` was moved into `consume`");

    assert_eq!(diagnostic.severity(), Severity::Error);
    assert_eq!(diagnostic.code().unwrap().as_str(), "SC0601");
    assert_eq!(diagnostic.message(), "use of freed value `p`");
    assert_eq!(diagnostic.labels().len(), 2);
    assert_eq!(diagnostic.notes(), ["`p` was moved into `consume`"]);
}

/// Labels are kept in the order they were attached, because a use-after-free
/// reads as allocation, then free, then use. The caret still belongs on the
/// primary label wherever it sits in that order.
#[test]
fn the_caret_goes_on_the_primary_label_not_the_first_one() {
    let diagnostic = Diagnostic::error("use of freed value `p`")
        .with_label(Label::secondary(span(0, 4), "allocated here"))
        .with_label(Label::secondary(span(6, 10), "freed here"))
        .with_label(Label::primary(span(12, 14), "used here"));

    let primary = diagnostic.primary_label().unwrap();
    assert_eq!(primary.message(), "used here");
    assert_eq!(primary.span(), span(12, 14));
}

#[test]
fn a_diagnostic_with_no_primary_label_falls_back_to_the_first() {
    let diagnostic =
        Diagnostic::warning("suspect").with_label(Label::secondary(span(0, 1), "here"));

    assert_eq!(diagnostic.primary_label().unwrap().message(), "here");
}

#[test]
fn a_diagnostic_without_labels_has_no_caret() {
    assert!(
        Diagnostic::error("no input files")
            .primary_label()
            .is_none()
    );
}

/// A use-after-free reads as allocation, then free, then use, and a check
/// attaches its labels and notes in that order. Losing the order, or
/// keeping only one of them, destroys the explanation.
#[test]
fn labels_and_notes_keep_the_order_they_were_attached_in() {
    let diagnostic = Diagnostic::error("use of freed value `p`")
        .with_label(Label::secondary(span(0, 4), "allocated here"))
        .with_label(Label::secondary(span(6, 10), "freed here"))
        .with_label(Label::primary(span(12, 14), "used here"))
        .with_note("first")
        .with_note("second");

    assert_eq!(
        diagnostic
            .labels()
            .iter()
            .map(Label::message)
            .collect::<Vec<_>>(),
        ["allocated here", "freed here", "used here"]
    );
    assert_eq!(diagnostic.notes(), ["first", "second"]);
}

#[test]
fn a_sink_counts_only_the_diagnostics_that_fail_the_build() {
    let mut sink = DiagnosticSink::new();
    assert!(sink.is_empty());
    assert!(!sink.has_errors());

    sink.report(Diagnostic::warning("unused variable `x`"));
    assert_eq!(sink.len(), 1);
    assert_eq!(sink.error_count(), 0);
    assert!(!sink.has_errors());
    // `is_empty` asks whether anything was reported, not whether anything
    // failed: a driver that skips rendering on an empty sink must still
    // render a run that produced only warnings.
    assert!(!sink.is_empty());

    sink.report(Diagnostic::error("use of freed value `p`"));
    assert_eq!(sink.len(), 2);
    assert_eq!(sink.error_count(), 1);
    assert!(sink.has_errors());
}

#[test]
fn a_sink_keeps_diagnostics_in_the_order_they_were_reported() {
    let mut sink = DiagnosticSink::new();
    sink.report(Diagnostic::error("first"));
    sink.report(Diagnostic::warning("second"));

    let messages: Vec<_> = sink.diagnostics().iter().map(Diagnostic::message).collect();
    assert_eq!(messages, ["first", "second"]);
}

/// Everything that states a fact about the program is proven, including a
/// diagnostic that has nothing to do with safety analysis.
#[test]
fn a_diagnostic_states_a_fact_unless_it_says_otherwise() {
    assert_eq!(
        Diagnostic::error("expected `;`").certainty(),
        Certainty::Proven
    );
    assert_eq!(
        Diagnostic::warning("unused variable `x`").certainty(),
        Certainty::Proven
    );
    assert_eq!(
        Diagnostic::new(Severity::Note, "for reference").certainty(),
        Certainty::Proven
    );
}

/// An unprovable diagnostic is a warning until the sink says otherwise.
///
/// The name said `unproven` until that constructor was replaced by
/// `concluded`, and what it holds is the severity a conclusion starts at
/// rather than which function made it: the promotion below is about a
/// warning becoming an error, and this is where the warning comes from.
///
/// Mutation: start an unknown conclusion at any other severity. This fails,
/// and so does `a_sink_leaves_an_unproven_warning_alone_by_default`.
///
/// **Not the promotion test beside that one**, which is worth knowing:
/// `promote_to_error` sets the severity to `Error` whatever it was, so
/// where an unproven result *started* is invisible to the one test that
/// promotes it. Only the test that declines to promote can see it, which is
/// why these two are not interchangeable.
#[test]
fn a_result_that_could_not_be_proven_starts_as_a_warning() {
    let diagnostic = unproven("`p` may escape the lifetime of `buf`");

    assert_eq!(diagnostic.certainty(), Certainty::Unproven);
    assert_eq!(diagnostic.severity(), Severity::Warning);
}

/// The default sink, which names no safety level and so has asked for
/// nothing. It is what the frontend's own tests report into; a run that
/// asked to be checked gets a [`Policy`] built from its level instead, and
/// `every_level_that_runs_a_check_denies_unknown_unless_it_was_allowed` is
/// where that is held.
#[test]
fn a_sink_leaves_an_unproven_warning_alone_by_default() {
    let mut sink = DiagnosticSink::new();
    sink.report(unproven("`p` may escape"));

    assert_eq!(sink.diagnostics()[0].severity(), Severity::Warning);
    assert_eq!(sink.error_count(), 0);
    assert!(!sink.has_errors());
}

/// The one place the policy is applied. A check never sees it, so it cannot
/// forget it.
///
/// Built at `Memory` with nothing asked, which is what a plain `safec
/// main.c` resolves to since ADR-0033. `Off` cannot stand in: a level that
/// runs no check has nothing to promote and now says so.
#[test]
fn a_sink_that_denies_unknown_raises_an_unproven_warning_to_an_error() {
    let mut sink = DiagnosticSink::with_policy(Policy::new(SafetyLevel::Memory, false));
    sink.report(unproven("`p` may escape"));

    assert_eq!(sink.diagnostics()[0].severity(), Severity::Error);
    assert_eq!(sink.error_count(), 1);
    assert!(sink.has_errors());
}

/// The negative half. Without this the promotion could be correct for the
/// wrong reason: raising every warning would pass the test above and turn
/// `unused variable` into a build failure.
#[test]
fn a_sink_that_denies_unknown_leaves_a_proven_warning_alone() {
    let mut sink = DiagnosticSink::with_policy(Policy::new(SafetyLevel::Memory, false));
    sink.report(Diagnostic::warning("unused variable `x`"));

    assert_eq!(sink.diagnostics()[0].severity(), Severity::Warning);
    assert_eq!(sink.error_count(), 0);
    assert!(!sink.has_errors());
}

/// The count is kept rather than recomputed, so it has to be taken after
/// the promotion, not before.
#[test]
fn the_error_count_agrees_with_the_diagnostics_under_either_policy() {
    for allow_unknown in [false, true] {
        let mut sink = DiagnosticSink::with_policy(Policy::new(SafetyLevel::Memory, allow_unknown));
        sink.report(Diagnostic::error("expected `;`"));
        sink.report(Diagnostic::warning("unused variable `x`"));
        sink.report(unproven("`p` may escape"));
        sink.report(unproven("`q` may escape"));

        let recount = sink
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.severity().is_error())
            .count();
        assert_eq!(
            sink.error_count(),
            recount,
            "allow_unknown = {allow_unknown}"
        );
        assert_eq!(sink.error_count(), if allow_unknown { 1 } else { 3 });
    }
}

#[test]
fn a_diagnostic_carries_the_safety_level_it_came_from() {
    let diagnostic = unproven("`p` may escape").with_safety_level(SafetyLevel::Lifetime);

    assert_eq!(diagnostic.safety_level(), Some(SafetyLevel::Lifetime));
    assert_eq!(Diagnostic::error("expected `;`").safety_level(), None);
}

/// The mapping from the options to what the sink acts on, in one place so a
/// driver cannot invent its own.
#[test]
fn a_policy_is_read_from_the_options_the_run_was_given() {
    let mut options = Options {
        inputs: Vec::new(),
        output: None,
        safety: SafetyLevel::Memory,
        emit: crate::options::EmitKind::Executable,
        target: Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        allow_unknown: false,
        color: crate::options::ColorMode::Never,
    };
    assert!(Policy::from(&options).deny_unknown());

    options.allow_unknown = true;
    assert!(!Policy::from(&options).deny_unknown());
}

/// Asking to be checked is being held to it: see ADR-0033.
///
/// The rows are written out rather than computed from the levels. A test
/// that restates the implementation's comparison holds for whatever that
/// comparison happens to say, and this table is a definition instead. The
/// length check is what covers a level nobody has written yet: adding one
/// without answering for it here
/// fails this test by name rather than passing quietly, which is as far as
/// `value_variants` can be taken, since it is not a `const fn` and the
/// count cannot be a compile error.
///
/// Mutation: resolve the way this resolved before #209,
/// `deny_unknown: allow_unknown || safety >= SafetyLevel::Strict`. The four
/// middle rows fail on their first assertion, and so does most of the
/// corpus. Both halves have to go red: if only one does, the default is
/// being decided in two places.
#[test]
fn every_level_that_runs_a_check_denies_unknown_unless_it_was_allowed() {
    // The level, what it does when nothing was asked, and what it does when
    // the run asked to be left its unproven conclusions.
    const ROWS: [(SafetyLevel, bool, bool); 6] = [
        (SafetyLevel::Off, false, false),
        (SafetyLevel::Memory, true, false),
        (SafetyLevel::Lifetime, true, false),
        (SafetyLevel::Ownership, true, false),
        (SafetyLevel::Thread, true, false),
        (SafetyLevel::Strict, true, true),
    ];

    assert_eq!(
        ROWS.len(),
        SafetyLevel::value_variants().len(),
        "a safety level was added and this table did not answer for it"
    );

    for (level, by_default, when_allowed) in ROWS {
        assert_eq!(
            Policy::new(level, false).deny_unknown(),
            by_default,
            "{level:?}, nothing asked"
        );
        assert_eq!(
            Policy::new(level, true).deny_unknown(),
            when_allowed,
            "{level:?}, asked to allow unknown"
        );
    }
}

/// The level that is defined as leaving nothing `Unknown` has to deny
/// unknown results even when the run asked to be left them. `Options` has
/// public fields and no constructor, so a caller that builds one without
/// the parser, which is the Clang adapter, would otherwise get warnings at
/// the strictest level and a successful exit.
#[test]
fn the_strictest_safety_level_denies_unknown_even_unresolved() {
    let options = Options {
        inputs: Vec::new(),
        output: None,
        safety: SafetyLevel::Strict,
        emit: crate::options::EmitKind::Executable,
        target: Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        allow_unknown: true,
        color: crate::options::ColorMode::Never,
    };

    assert!(Policy::from(&options).deny_unknown());

    let mut sink = DiagnosticSink::with_policy(Policy::from(&options));
    sink.report(unproven("`p` may escape"));
    assert_eq!(sink.diagnostics()[0].severity(), Severity::Error);
    assert!(sink.has_errors());
}

/// The guard is on the constructor rather than on the conversion, so a
/// caller that never builds an [`Options`] at all gets it too. That caller
/// is the Clang adapter. The other half of this is a compile error rather
/// than an assertion: `Policy { deny_unknown: false }` no longer builds,
/// here or anywhere else, because the field is private.
///
/// Mutation: `Self { deny_unknown: !allow_unknown }`, which honours the
/// request at every level. This fails, and so do the two above it.
#[test]
fn the_strictest_safety_level_denies_unknown_however_the_policy_is_built() {
    let policy = Policy::new(SafetyLevel::Strict, true);

    assert!(policy.deny_unknown());

    let mut sink = DiagnosticSink::with_policy(policy);
    sink.report(unproven("`p` may escape"));
    assert_eq!(sink.diagnostics()[0].severity(), Severity::Error);
    assert!(sink.has_errors());
}
