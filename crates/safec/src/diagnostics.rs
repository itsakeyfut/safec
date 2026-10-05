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
    /// The severity of a help diagnostic standing on its own.
    ///
    /// **Nothing can emit one, and a remedy is not this.** `Diagnostic::new`
    /// is private, so no diagnostic built outside this module can carry this
    /// severity. The variant itself is an ordinary public value and stays
    /// reachable; what is held is what a `Diagnostic` may be. What to change about a
    /// program is a [`Remedy`] on the diagnostic that rejected it, so that the
    /// two cannot be separated by a sort, a filter or a count. ADR-0034 records
    /// why, and what it rejected is exactly what this doc comment used to say:
    /// that a help is a suggestion reported alongside the thing it explains.
    ///
    /// The variant stays because it is a severity this renderer can spell, and
    /// because the two tests holding the two rendering paths to one spelling
    /// are better for having a fourth to iterate over.
    Help,
    /// Context the user asked for.
    ///
    /// **Nothing can emit one either**, for the reason above. What a check
    /// offers unprompted is [`Diagnostic::with_note`], a field on the
    /// diagnostic it belongs to, which `write_notes` spells `= note:`
    /// without asking any severity. This variant is a header this renderer
    /// can spell and nothing more, which is also why [`Severity::Help`]
    /// stays.
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
    /// same reason: the version spelled as a comparison alone answered one
    /// side of the question and left the other to nobody.
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
    ///
    /// **Also what a level nobody implemented amounts to**, since ADR-0035:
    /// `driver::undelivered` reaches this through [`Diagnostic::concluded`] for a
    /// run asking to be held to checks that do not exist, which is the limit case
    /// of unproven rather than a fourth certainty.
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

/// A change that would make the program compile, or what this check would need
/// in order to conclude.
///
/// **A type rather than a `String`, for two reasons.** It is the one part of a
/// diagnostic addressed to what the reader should do next rather than to what
/// happened, and that is worth holding in a type rather than in a convention
/// about which `Vec<String>` a sentence was put into. And
/// [`Diagnostic::concluded`] takes a message and a remedy in that order: two
/// `impl Into<String>` arguments in a row can be given the wrong way round with
/// nothing at all to say so, and these cannot.
///
/// **It carries no span, and that is a decision rather than an omission.**
/// Every place a remedy would point at today is already a label, `freed here`
/// and `allocated here` in `driver.rs`, or is not written down anywhere: the
/// place to test a pointer before reading through it is not a span this
/// compiler holds. A field set by nobody and read by nobody is breakable by no
/// mutation, which is a guard in name only. Whoever writes the first remedy
/// that wants a place adds one then. See ADR-0034.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remedy {
    message: String,
}

impl Remedy {
    /// What the reader should do next.
    ///
    /// Usually an imperative naming a change to the program. Not always: where
    /// a check gave up rather than found something, what it says instead is
    /// what it failed to establish, because an instruction there would claim
    /// something about a program nothing was worked out about. `LOST_REMEDY` in
    /// `driver.rs` is that case and is the reason this sentence does not say
    /// "in the imperative", which it used to and which was already false of a
    /// string in the tree.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// What this remedy asks for.
    pub fn message(&self) -> &str {
        &self.message
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
    /// What to change about the program, or what this check would have needed.
    ///
    /// A `Vec` rather than an `Option` for two reasons that pull the same way:
    /// a diagnostic about the invocation has nothing to suggest and carries
    /// none, and one rejection can ask for more than one thing.
    /// [`Self::concluded`] is what makes it non-empty for a safety finding.
    remedies: Vec<Remedy>,
}

impl Diagnostic {
    /// A diagnostic at the given severity.
    ///
    /// **Private, and that is the guard.** [`Severity::Help`] and
    /// [`Severity::Note`] mean nothing failed, and this is the only way to
    /// build a diagnostic at either. With it private, a diagnostic built
    /// outside this module can carry only a severity that means something:
    /// [`Severity::Error`] through [`Self::error`] and [`Self::concluded`], and
    /// [`Severity::Warning`] through [`Self::warning`] and the same. One at
    /// [`Severity::Help`] is `error[E0624]` where it used to be a sentence.
    /// See ADR-0034.
    ///
    /// **What this does not hold is a constructor added beside these.** A
    /// guard that is an absence degrades by addition rather than by edit, and
    /// nothing here notices: a `pub fn note` written next to [`Self::warning`]
    /// and reported into a sink leaves the whole suite green, which was
    /// measured rather than supposed. It is the same shape as the hole stated
    /// on [`Self::concluded`].
    ///
    /// Reachable inside this module **and its children**, which is why the two
    /// tests in `render.rs` can iterate all four severities: privacy holds
    /// against the rest of the crate and against a user of it, not against
    /// this module's own subtree.
    fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            certainty: Certainty::Proven,
            level: None,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            remedies: Vec::new(),
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
    /// What `E0004` cannot do is make these three right: it makes somebody
    /// look and nothing more. Three
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
    /// the policy of the run it is part of is. See ADR-0001.
    ///
    /// **The only way to build a [`Certainty::Unproven`] diagnostic from
    /// outside this module**, which is what a check is. Every field of a
    /// `Diagnostic` is private, so nothing else can make one; inside this file
    /// a second constructor could, and the only thing stopping that is
    /// somebody reading this sentence.
    ///
    /// **The remedy is required, because a rejection that does not say what to
    /// change is a refusal.** A check that cannot name one is not excused: what
    /// it says instead is what it failed to establish, which claims nothing
    /// about the program. Taking this parameter away is `error[E0061]` at every
    /// call site, which is the point of it being a parameter rather than a
    /// `with_` method: an omitted argument has to stop the build, and a `with_`
    /// method that was never called cannot. Making it an `Option` instead is
    /// `error[E0308]` at the same places, which is a different error and worth
    /// spelling correctly, because a reader checking this sentence runs the
    /// mutation it names. See ADR-0034.
    ///
    /// **The hole, stated rather than closed.** [`Self::error`] is public and
    /// this reaches it for [`Conclusion::Unsafe`], so a check can build a
    /// rejecting diagnostic through it and carry no remedy. Nothing here stops
    /// that. What it costs is a rejection reaching a reader without its remedy,
    /// which is visible in the output and in a corpus expectation rather than
    /// silent, so it is the cheaper of the two failures. It is the same shape
    /// as the sentence above about a second constructor.
    ///
    /// [`Conclusion::Safe`] builds no diagnostic, so the remedy given for one
    /// is dropped. A caller with nothing to suggest there is answering for an
    /// arm that reports nothing, which is what `nullability_finding` already
    /// does for the message and the label.
    pub fn concluded(what: Conclusion, message: impl Into<String>, remedy: Remedy) -> Option<Self> {
        match what {
            Conclusion::Safe => None,
            Conclusion::Unsafe => Some(Self::error(message).with_remedy(remedy)),
            Conclusion::Unknown => Some(
                Self {
                    certainty: Certainty::Unproven,
                    ..Self::new(Severity::Warning, message)
                }
                .with_remedy(remedy),
            ),
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

    /// Ask for a change that would make the program compile.
    ///
    /// **Private, because nothing attaches a second one.** [`Self::concluded`]
    /// is the only caller and it calls this once, so whether this appends or
    /// replaces is a difference no mutation can show: both were tried and the
    /// suite stayed green either way. Public, it would be an interface whose
    /// distinguishing behaviour has no caller. Whoever writes the first
    /// rejection with two ways out of it makes this `pub` and brings a test for
    /// the order they arrive in.
    ///
    /// Private to this module rather than to this `impl`, so
    /// [`crate::diagnostics::render`] can still build one to render.
    fn with_remedy(mut self, remedy: Remedy) -> Self {
        self.remedies.push(remedy);
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

    /// Every remedy, in the order they were attached.
    pub fn remedies(&self) -> &[Remedy] {
        &self.remedies
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
    /// Private, and called from exactly one place: [`DiagnosticSink::report`],
    /// under a [`Policy`] that denies unknown. Because [`Diagnostic::concluded`] is the only
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
    /// The policy a run at `safety`, having asked or not asked to be left
    /// `allow_unknown`, is entitled to.
    ///
    /// Three facts in one expression, which is total over the order the levels
    /// are declared in. Level 0 runs no check, so it has nothing to promote.
    /// The strictest level is defined as leaving nothing `Unknown`, so allowing
    /// is not on offer there. Everything between denies unless the run asked
    /// otherwise, because a conclusion this analysis could not prove does not
    /// build once a program has asked to be checked: see ADR-0033.
    ///
    /// Resolving it here is why this constructor exists: the two arguments are
    /// the whole question, and a caller holding both is a caller who can answer
    /// it wrong. See ADR-0004.
    ///
    /// `safety` is read and not kept. A policy says what to do with a result
    /// that arrives, and which checks ran to produce it is not that.
    pub fn new(safety: SafetyLevel, allow_unknown: bool) -> Self {
        Self {
            deny_unknown: safety > SafetyLevel::Off
                && (!allow_unknown || safety >= SafetyLevel::Strict),
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
    /// One line, because [`Policy::new`] is the only resolution there is.
    /// [`Options`] has public fields and no constructor, so anything it claimed
    /// to have resolved would be a claim a caller who did not use the parser
    /// could fail to deliver, and the Clang adapter is exactly that caller.
    /// `allow_unknown` claims nothing: it is what was asked for, and every
    /// value of it is a legitimate ask. See ADR-0004.
    fn from(options: &Options) -> Self {
        Self::new(options.safety, options.allow_unknown)
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
mod tests;
