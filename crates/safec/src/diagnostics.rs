//! Diagnostics: what the compiler reports, and how it is collected.
//!
//! Safety analysis is only useful if a developer can see why something was
//! rejected, so a diagnostic is a first-class value here rather than a string
//! written to stderr at the point of failure.
//!
//! A [`Diagnostic`] describes *what* is wrong. Nothing in this module decides
//! how it looks: rendering lives in [`crate::diagnostics::render`], which is
//! the only place that knows about a rendering library at all. That boundary is
//! what lets the same diagnostic reach a terminal, a `--error-format=json`
//! stream, and eventually a language server.

pub mod render;

use std::fmt;

use crate::options::Options;
use crate::safety::SafetyLevel;
use safec_ir::analysis::Conclusion;
use safec_ir::source::Span;

/// How serious a diagnostic is.
///
/// Declared from least to most severe, the same direction as
/// [`SafetyLevel`], so that `severity >= Severity::Warning` reads as "warning
/// or worse" and the worst severity in a set is its maximum. Two ordinal enums
/// in one crate that disagree about which way is up are a trap, and this is the
/// direction the reader expects.
///
/// A more serious variant than [`Severity::Error`], for a problem that has to
/// stop the compilation where an error would let it continue, belongs above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// A suggested way to resolve a diagnostic reported alongside it.
    Help,
    /// Context the user asked for, or that a check offers unprompted.
    Note,
    /// Something is suspect, but compilation continues.
    Warning,
    /// The compilation cannot produce an artifact.
    Error,
}

impl Severity {
    /// The word this severity is spelled with in a diagnostic header.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
            Self::Help => "help",
        }
    }

    /// Whether a diagnostic at this severity means compilation has failed.
    ///
    /// **The `match` is there to be read, and the comparison is the answer.**
    /// Neither half works alone, and the two failures they prevent are
    /// different ones.
    ///
    /// Without the `match`, `self >= Self::Error` is right and silent. A
    /// severity that stops a compilation where an error would let it continue
    /// is not something to arrive with nobody deciding what the exit code owes
    /// it, or whether the artifact should still be written. The `match` has no
    /// wildcard, so adding a variant is `error[E0004]` here until somebody
    /// looks. `EmitKind` in `driver.rs` is bounded the same way and for the
    /// same reason; RK-003 in the review knowledge bank records what the
    /// version spelled as a comparison alone cost.
    ///
    /// The obvious way to silence `E0004` is a wildcard, and that one is shut
    /// too: with the arms replaced by `_` this is a match over a single
    /// binding, `clippy::match_single_binding` fires, and the gate runs clippy
    /// with `-D warnings`.
    ///
    /// Without the comparison, looking is not enough. `matches!(self,
    /// Self::Error)` was what stood here, and an arm list would have replaced
    /// one silence with another: `Self::Fatal => false` compiles, and nothing
    /// downstream disagrees, because the tests walk a written-out roster and
    /// nothing makes anybody add a row to it. That was measured rather than
    /// imagined, on a branch where all 285 tests passed while `-o` printed a
    /// fatal diagnostic, wrote nothing, and exited zero. Answering by the
    /// ordering the variants are declared in leaves nothing to get wrong.
    pub fn is_error(self) -> bool {
        match self {
            Self::Help | Self::Note | Self::Warning | Self::Error => self >= Self::Error,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What an analysis concluded, as distinct from how it is reported.
///
/// Two, where the model has three: a safe result produces no diagnostic at all,
/// so only the other two reach a [`Diagnostic`], and what separates them is
/// whether the analysis could prove what it reports. Which conclusion becomes
/// which is [`Diagnostic::concluded`]'s to say and is not repeated here.
///
/// Severity is not this. A check decides what it concluded; whether an
/// unprovable conclusion is a warning or an error is [`DiagnosticSink`]'s
/// decision, taken once, from [`Policy`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Certainty {
    /// The problem is real. Everything that states a fact about the program is
    /// this, a syntax error and an unused variable included, which is why it is
    /// the default.
    #[default]
    Proven,
    /// The analysis could not prove the code safe, and could not prove it wrong
    /// either. The unknown of the three-valued model, and the thing an
    /// annotation exists to remove.
    Unproven,
}

/// A stable identifier for a class of diagnostic, such as `SC0601`.
///
/// Codes are how a user looks a diagnostic up and how a build script silences
/// one, so they are part of the interface. A code is assigned once and never
/// reused, even after the check that raised it is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code(&'static str);

impl Code {
    /// A diagnostic code.
    ///
    /// `const` so that a check can declare its codes as constants and have
    /// them cost nothing at runtime, and so that the shape `docs/diagnostics.md`
    /// allocates is held by the compiler rather than by whoever reviews the next
    /// one. Every code in this crate is declared as a `const`, and a `const`
    /// whose spelling is not `SC` and four digits stops the build with
    /// `error[E0080]` at its own declaration.
    ///
    /// The check cannot say which range a code belongs in, because only a human
    /// knows what a new diagnostic is about. It says that a code was not left
    /// spelled the way another compiler spells one, which is the half of
    /// [ADR-0009](https://github.com/itsakeyfut/safec/blob/main/docs/adr/0009-name-a-diagnostic-code-after-the-compiler-and-the-topic.md)
    /// a machine can hold.
    pub const fn new(code: &'static str) -> Self {
        let spelling = code.as_bytes();

        assert!(
            spelling.len() == 6 && spelling[0] == b'S' && spelling[1] == b'C',
            "a diagnostic code is `SC` and four digits"
        );

        let mut digit = 2;
        while digit < spelling.len() {
            assert!(
                spelling[digit].is_ascii_digit(),
                "a diagnostic code is `SC` and four digits"
            );
            digit += 1;
        }

        Self(code)
    }

    /// The code as it is spelled in a diagnostic header.
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A span with something to say about it.
///
/// A diagnostic points at several places at once: where a value was allocated,
/// where it was freed, and where it was used afterwards. Each of those is a
/// label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    span: Span,
    message: String,
    primary: bool,
}

impl Label {
    /// The label for the place a diagnostic is really about.
    ///
    /// This is where the caret goes when the diagnostic has one.
    pub fn primary(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            primary: true,
        }
    }

    /// A label for a place that explains the primary one.
    pub fn secondary(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            primary: false,
        }
    }

    /// Where this label points.
    pub fn span(&self) -> Span {
        self.span
    }

    /// What this label says.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether this is the place the diagnostic is really about.
    pub fn is_primary(&self) -> bool {
        self.primary
    }
}

/// Something the compiler has to say about the program.
///
/// Built by chaining: a message is required, everything else is optional.
///
/// ```
/// # use safec::diagnostics::{Code, Diagnostic, Label};
/// # use safec_ir::source::{SourceMap, Span};
/// # let mut map = SourceMap::new();
/// # let file = map.add_virtual("main.c", "int *p = 0;");
/// let diagnostic = Diagnostic::error("use of freed value `p`")
///     .with_code(Code::new("SC0601"))
///     .with_label(Label::primary(Span::new(file, 5, 6), "used here"))
///     .with_note("`p` was freed above");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    severity: Severity,
    certainty: Certainty,
    /// Which safety check this came from, for the reader. It does not take part
    /// in deciding the severity: that is [`Certainty`] and [`Policy`].
    level: Option<SafetyLevel>,
    code: Option<Code>,
    message: String,
    labels: Vec<Label>,
    notes: Vec<String>,
}

impl Diagnostic {
    /// A diagnostic at the given severity.
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            certainty: Certainty::Proven,
            level: None,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// A diagnostic that means compilation has failed.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// A diagnostic that reports something suspect without failing.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    /// What a check concluded, as what is reported about it.
    ///
    /// **The whole of the table in `docs/safety-model.md`, in one place.** A
    /// check answers a [`Conclusion`] and never decides a severity, so no two
    /// checks can remember the mapping differently: the arms are here, and a
    /// fourth conclusion is `error[E0004]` until somebody says what it reports.
    ///
    /// What `E0004` cannot do is make these three right, which is RK-015 in the
    /// review knowledge bank: it makes somebody look and nothing more. Three
    /// arms a reader can hold against the table in the document are what is
    /// left, and `every_conclusion_is_reported_the_way_the_model_says` writes
    /// them out again rather than asking this what it says.
    ///
    /// `None` for [`Conclusion::Safe`], because nothing is the report. That is
    /// also what makes reporting a safe result impossible rather than merely
    /// wrong: there is no diagnostic to hand the sink.
    ///
    /// An unproven result starts as a warning and may not stay one. Whether it
    /// fails the build is [`DiagnosticSink`]'s, taken once from [`Policy`], so
    /// a check that reaches [`Conclusion::Unknown`] does not need to know what
    /// `--deny-unknown` is. See ADR-0001.
    ///
    /// **The only way to build a [`Certainty::Unproven`] diagnostic from
    /// outside this module**, which is what a check is. Every field of a
    /// `Diagnostic` is private, so nothing else can make one; inside this file
    /// a second constructor could, and the only thing stopping that is
    /// somebody reading this sentence.
    pub fn concluded(what: Conclusion, message: impl Into<String>) -> Option<Self> {
        match what {
            Conclusion::Safe => None,
            Conclusion::Unsafe => Some(Self::error(message)),
            Conclusion::Unknown => Some(Self {
                certainty: Certainty::Unproven,
                ..Self::new(Severity::Warning, message)
            }),
        }
    }

    /// Attach a stable code, such as `SC0601`.
    pub fn with_code(mut self, code: Code) -> Self {
        self.code = Some(code);
        self
    }

    /// Point at a place in the source.
    pub fn with_label(mut self, label: Label) -> Self {
        self.labels.push(label);
        self
    }

    /// Add a remark that stands on its own, with no span to point at.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Record which safety check this came from.
    ///
    /// For the reader only. It does not affect the severity: a proven unsafe
    /// result is an error at every level, because there is no safety argument
    /// for knowing something is wrong and saying it quietly.
    pub fn with_safety_level(mut self, level: SafetyLevel) -> Self {
        self.level = Some(level);
        self
    }

    /// How serious this diagnostic is.
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// The code, if this diagnostic has one.
    pub fn code(&self) -> Option<Code> {
        self.code
    }

    /// The headline message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Every label, in the order they were attached.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Every note, in the order they were attached.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Whether the analysis could prove what this reports.
    pub fn certainty(&self) -> Certainty {
        self.certainty
    }

    /// Which safety check this came from, if it came from one.
    pub fn safety_level(&self) -> Option<SafetyLevel> {
        self.level
    }

    /// Raise an unproven warning to an error.
    ///
    /// Private, and called from exactly one place: [`DiagnosticSink::report`]
    /// under `--deny-unknown`. Because [`Diagnostic::concluded`] is the only
    /// source of [`Certainty::Unproven`] and fixes the severity at
    /// [`Severity::Warning`], this only ever has one direction to handle.
    fn promote_to_error(&mut self) {
        self.severity = Severity::Error;
    }

    /// The label the caret belongs on.
    ///
    /// The first primary label, or the first label of any kind when a diagnostic was built without one.
    pub fn primary_label(&self) -> Option<&Label> {
        self.labels
            .iter()
            .find(|label| label.is_primary())
            .or_else(|| self.labels.first())
    }
}

/// What a [`DiagnosticSink`] does with a result the analysis could not prove.
///
/// Held by the sink rather than passed to each check. The promotion then
/// happens in one function, and a check that never sees the policy cannot
/// forget to apply it. Missing one would make the compiler exit successfully on
/// code it never managed to check, which is the worst thing it can do.
///
/// The field is private, so [`Policy::new`] is the only way to arrive at one.
/// A public field would let a caller write the resolved answer itself, which is
/// how a guard placed here gets bypassed rather than applied. See ADR-0004.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Policy {
    /// Whether a [`Certainty::Unproven`] result is an error rather than a
    /// warning. The resolved answer: see [`Policy::new`].
    deny_unknown: bool,
}

impl Policy {
    /// The policy a run at `safety`, having asked or not asked for
    /// `deny_unknown`, is entitled to.
    ///
    /// The strictest safety level is defined as leaving nothing `Unknown`, so
    /// it denies unknown results whether or not the flag was given as well.
    /// Resolving that here is why this constructor exists: the two arguments
    /// are the whole question, and a caller holding both is a caller who can
    /// answer it wrong.
    ///
    /// `safety` is read and not kept. A policy says what to do with a result
    /// that arrives, and which checks ran to produce it is not that.
    pub fn new(safety: SafetyLevel, deny_unknown: bool) -> Self {
        Self {
            deny_unknown: deny_unknown || safety >= SafetyLevel::Strict,
        }
    }

    /// Whether a [`Certainty::Unproven`] result is an error rather than a
    /// warning.
    pub fn deny_unknown(self) -> bool {
        self.deny_unknown
    }
}

impl From<&Options> for Policy {
    /// The mapping from the options to what the sink acts on.
    ///
    /// `Cli::into_options` resolves the strictest level too, and this resolves
    /// it again rather than reading `options.deny_unknown` and believing it:
    /// [`Options`] has public fields and its `deny_unknown` doc claims to be
    /// the resolved answer, with nothing enforcing that for a caller who builds
    /// one without the parser, which is what the Clang adapter will do.
    /// Trusting the field alone means the strictest level reports an unprovable
    /// result as a warning and the compilation exits successfully. See ADR-0004.
    fn from(options: &Options) -> Self {
        Self::new(options.safety, options.deny_unknown)
    }
}

/// Where diagnostics are collected, and where policy is applied to them.
///
/// Passes report into a sink rather than writing to stderr, so that a test can
/// inspect what was reported and the driver can decide, once, what to do with
/// the lot. It is also the enforcement point: a check says what it concluded
/// and the sink decides, in one place, what that means for the build.
#[derive(Debug, Default)]
pub struct DiagnosticSink {
    diagnostics: Vec<Diagnostic>,
    errors: usize,
    policy: Policy,
}

impl DiagnosticSink {
    /// An empty sink that reports an unprovable result as a warning.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty sink that applies `policy` to everything reported into it.
    pub fn with_policy(policy: Policy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    /// What this sink does with a result the analysis could not prove.
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Record a diagnostic.
    pub fn report(&mut self, mut diagnostic: Diagnostic) {
        // The one place an unprovable result becomes an error. Counting after
        // the promotion rather than before is what keeps `error_count` equal to
        // the number of errors in `diagnostics`.
        if self.policy.deny_unknown && diagnostic.certainty() == Certainty::Unproven {
            diagnostic.promote_to_error();
        }
        if diagnostic.severity().is_error() {
            self.errors += 1;
        }
        self.diagnostics.push(diagnostic);
    }

    /// Everything reported so far, in the order it was reported.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The number of diagnostics that mean compilation has failed.
    pub fn error_count(&self) -> usize {
        self.errors
    }

    /// Whether compilation has failed.
    pub fn has_errors(&self) -> bool {
        self.errors > 0
    }

    /// Whether nothing has been reported.
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// The number of diagnostics reported, at any severity.
    pub fn len(&self) -> usize {
        self.diagnostics.len()
    }
}

#[cfg(test)]
mod tests {
    use safec_ir::analysis::Conclusion;

    /// What a check that could not prove anything reports, for a test that is
    /// about what happens to one rather than about how it is built.
    ///
    /// Through `concluded` rather than beside it, so that these tests exercise
    /// the path a check takes: an unproven diagnostic has one way to exist and
    /// this is it.
    fn unproven(message: &str) -> Diagnostic {
        Diagnostic::concluded(Conclusion::Unknown, message)
            .expect("an unknown conclusion is reported")
    }

    /// Each conclusion is reported the way `docs/safety-model.md` says.
    ///
    /// The three written out rather than walked, for RK-001's reason: a test
    /// that asks `concluded` what it answers agrees with it whatever it
    /// answers. These rows are the document's table copied by a reader, which
    /// is the only thing that can disagree with the code.
    ///
    /// What holds the *fourth* conclusion is `error[E0004]` in `concluded`
    /// rather than anything here. Nothing makes somebody add a row to this
    /// table, which is RK-015's limit: a compiler that makes you look does not
    /// make you right.
    ///
    /// Mutation: answer `Some` for `Safe`. The first row fails, and only it.
    /// Mutation: give `Unsafe` a warning's severity. The second row fails, and
    /// only it. Mutation: build `Unknown` with `Diagnostic::error`, so that its
    /// certainty is `Proven`. Eight tests fail, this among them: everything
    /// that watches the sink promote an unprovable result is downstream of this
    /// arm, which is what the third row is worth.
    #[test]
    fn every_conclusion_is_reported_the_way_the_model_says() {
        assert_eq!(
            Diagnostic::concluded(Conclusion::Safe, "nothing to say"),
            None
        );

        let unsafe_ = Diagnostic::concluded(Conclusion::Unsafe, "use of freed value `p`")
            .expect("an unsafe conclusion is reported");
        assert_eq!(unsafe_.severity(), Severity::Error);
        assert_eq!(unsafe_.certainty(), Certainty::Proven);
        assert_eq!(unsafe_.message(), "use of freed value `p`");

        let unknown = Diagnostic::concluded(Conclusion::Unknown, "`p` may escape")
            .expect("an unknown conclusion is reported");
        assert_eq!(unknown.severity(), Severity::Warning);
        assert_eq!(unknown.certainty(), Certainty::Unproven);
        assert_eq!(unknown.message(), "`p` may escape");
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
    /// The expected column is written out rather than computed, which is
    /// RK-001's rule and matters more here than usual: `is_error` answers by
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

    #[test]
    fn a_sink_leaves_an_unproven_warning_alone_by_default() {
        let mut sink = DiagnosticSink::new();
        sink.report(unproven("`p` may escape"));

        assert_eq!(sink.diagnostics()[0].severity(), Severity::Warning);
        assert_eq!(sink.error_count(), 0);
        assert!(!sink.has_errors());
    }

    /// The one place `--deny-unknown` is applied. A check never sees the policy,
    /// so it cannot forget it.
    #[test]
    fn a_sink_that_denies_unknown_raises_an_unproven_warning_to_an_error() {
        let mut sink = DiagnosticSink::with_policy(Policy::new(SafetyLevel::Off, true));
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
        let mut sink = DiagnosticSink::with_policy(Policy::new(SafetyLevel::Off, true));
        sink.report(Diagnostic::warning("unused variable `x`"));

        assert_eq!(sink.diagnostics()[0].severity(), Severity::Warning);
        assert_eq!(sink.error_count(), 0);
        assert!(!sink.has_errors());
    }

    /// The count is kept rather than recomputed, so it has to be taken after
    /// the promotion, not before.
    #[test]
    fn the_error_count_agrees_with_the_diagnostics_under_either_policy() {
        for deny_unknown in [false, true] {
            let mut sink = DiagnosticSink::with_policy(Policy::new(SafetyLevel::Off, deny_unknown));
            sink.report(Diagnostic::error("expected `;`"));
            sink.report(Diagnostic::warning("unused variable `x`"));
            sink.report(unproven("`p` may escape"));
            sink.report(unproven("`q` may escape"));

            let recount = sink
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.severity().is_error())
                .count();
            assert_eq!(sink.error_count(), recount, "deny_unknown = {deny_unknown}");
            assert_eq!(sink.error_count(), if deny_unknown { 3 } else { 1 });
        }
    }

    #[test]
    fn a_diagnostic_carries_the_safety_level_it_came_from() {
        let diagnostic = unproven("`p` may escape").with_safety_level(SafetyLevel::Lifetime);

        assert_eq!(diagnostic.safety_level(), Some(SafetyLevel::Lifetime));
        assert_eq!(Diagnostic::error("expected `;`").safety_level(), None);
    }

    /// The mapping from the resolved options to what the sink acts on, in one
    /// place so a driver cannot invent its own.
    #[test]
    fn a_policy_is_read_from_the_resolved_options() {
        let mut options = Options {
            inputs: Vec::new(),
            output: None,
            safety: SafetyLevel::Memory,
            emit: crate::options::EmitKind::Executable,
            target: Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
            deny_unknown: false,
            color: crate::options::ColorMode::Never,
        };
        assert!(!Policy::from(&options).deny_unknown());

        options.deny_unknown = true;
        assert!(Policy::from(&options).deny_unknown());
    }

    /// The level that is defined as leaving nothing `Unknown` has to deny
    /// unknown results even when the flag was not also set. `Cli::into_options`
    /// resolves this too, but `Options` has public fields and nothing enforces
    /// that its `deny_unknown` was resolved, so a caller that builds one
    /// without the parser would otherwise get warnings at the strictest level
    /// and a successful exit.
    #[test]
    fn the_strictest_safety_level_denies_unknown_even_unresolved() {
        let options = Options {
            inputs: Vec::new(),
            output: None,
            safety: SafetyLevel::Strict,
            emit: crate::options::EmitKind::Executable,
            target: Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
            deny_unknown: false,
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
    #[test]
    fn the_strictest_safety_level_denies_unknown_however_the_policy_is_built() {
        let policy = Policy::new(SafetyLevel::Strict, false);

        assert!(policy.deny_unknown());

        let mut sink = DiagnosticSink::with_policy(policy);
        sink.report(unproven("`p` may escape"));
        assert_eq!(sink.diagnostics()[0].severity(), Severity::Error);
        assert!(sink.has_errors());
    }
}
