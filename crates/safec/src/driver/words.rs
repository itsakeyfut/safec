//! What a safety finding reads as: the code, the message, the label and the
//! remedy each conclusion of the memory and nullability checks is reported with.
//!
//! One `match` per check, with every case written out rather than taken by `_`,
//! so that a kind or a reason added to a check is `error[E0004]` here until
//! somebody says what a reader is told about it.

use crate::diagnostics::{Code, Diagnostic, Label, Remedy};
use crate::safety::SafetyLevel;
use safec_ir::analysis::Conclusion;
use safec_ir::ir::Promise;
use safec_ir::memory::{self, Kind, Unproven};
use safec_ir::nullability::{self, Asked};
use safec_ir::source::Span;

/// A value freed where it may already have been freed.
///
/// `SC04xx` is the memory axis, reserved by `docs/diagnostics.md` before
/// anything could emit from it, and this is the first. It is built here rather
/// than where the check is because `safec_ir` cannot see a `Diagnostic` at all,
/// which is ADR-0011, and `BACKEND` above is the same arrangement for the same
/// reason.
const DOUBLE_FREE: Code = Code::new("SC0401");

/// A value read or written through a pointer after it was freed.
///
/// A code of its own rather than `DOUBLE_FREE`'s, because a reader filtering on
/// one wants the two apart: a double free is a mistake about ownership and this
/// is a mistake about lifetime, and the programs that produce them are different
/// programs. `docs/diagnostics.md`'s rule is that a code names a class of
/// program to search for, and these are two classes.
const USE_AFTER_FREE: Code = Code::new("SC0402");

/// A value read or written through a pointer that may be null.
///
/// A third code rather than a severity on one of the two above, because
/// `docs/diagnostics.md` makes a code a class of program to search for and this
/// is a third class: the other two are about a pointer that pointed somewhere
/// once, and this is about one that may never have.
const NULL_DEREFERENCE: Code = Code::new("SC0403");

/// A value freed through a pointer that is not the start of an allocation.
///
/// A code of its own rather than `DOUBLE_FREE`'s, because the fix is different:
/// a double free is a mistake about ownership, a use after free one about
/// lifetime, and this is a mistake about which value was handed to `free`. It
/// is also not `NULL_DEREFERENCE`'s, which is about a pointer that may point
/// nowhere, where this one points somewhere real that `free` does not take.
/// See ADR-0036.
const INTERIOR_FREE: Code = Code::new("SC0404");

/// A pointer that may be null handed back by a function that promised the
/// pointer it returns is not null.
///
/// Not `NULL_ARGUMENT`'s, though it is the same question asked of the other
/// direction of a call, because the fix is at a different place: at the
/// `return`, or at the promise the function made. See ADR-0050.
const NULL_RETURN: Code = Code::new("SC0408");

/// A pointer that may be null passed to a parameter declared `_Nonnull`.
///
/// Not `NULL_DEREFERENCE`'s, though the question is the same one, because the
/// fix is at a different place: that one is fixed where the pointer is read,
/// and this one where it is handed over, or at the promise the parameter made.
/// See ADR-0037.
const NULL_ARGUMENT: Code = Code::new("SC0405");

/// A pointer to an allocation that was freed, handed back by a `return`.
///
/// Not `USE_AFTER_FREE`'s, because that code is a dereference and this is a
/// read with none, and because the fix is at the return or at the free rather
/// than at a read. The caller that would have dereferenced it believes it is
/// live and cannot see why it is not. See ADR-0041.
const RETURN_AFTER_FREE: Code = Code::new("SC0406");

/// A pointer to an allocation that was freed, handed to a call.
///
/// Not `USE_AFTER_FREE`'s, for `RETURN_AFTER_FREE`'s reasons: this is a read
/// with no dereference, and the fix is at the call or at the free. The callee
/// believes its parameter is live and cannot see why it is not. See ADR-0042.
const ARGUMENT_AFTER_FREE: Code = Code::new("SC0407");

/// What to change where this check stopped following a pointer.
///
/// **It names no cause, and that is the whole of its design.**
/// `Unproven::Lost` has five producers, which its own doc comment lists, and a
/// `memory::Finding` does not say which of them answered. A remedy naming one
/// would be right for some and false for the others: a conservative
/// over-approximation given a confident word. So this says only what is true
/// of all five, and tells the reader the one thing that matters here, which is
/// not to go hunting for a defect.
///
/// Shared by the three codes that can lose a pointer, because the fact is the
/// same fact. #213 carries the reason into the `Finding` and replaces this
/// with five.
const LOST_REMEDY: &str = "nothing here says the program is wrong: this check could no longer \
                           say which allocation this pointer holds";

/// What to change where C has not ordered the free against the use.
///
/// The only remedy here about something C decided rather than about what the
/// program meant, which is why `Unsequenced` is a row of its own rather than
/// sharing one with `Disagreement`. See ADR-0022, and the note attached below
/// for what the check did and did not find.
const UNSEQUENCED_REMEDY: &str = "put the free and this in separate statements, so that C orders \
                                  one before the other";

/// What to change where this check could not tell whether the free reached.
///
/// **The free is conditional because this check does not know there is one.**
/// `Unproven::Disagreement` has two causes: paths that disagree about a free,
/// where one exists, and a call this check cannot read that was handed the
/// pointer, where there may be none at all. Nothing in a `Finding` separates
/// them, and its `freed` span does not: `memory.rs` carries a free span only
/// where it also answers `Unproven::Unsequenced`, so a `Disagreement` always
/// arrives with `None` and the report carries no `freed here` label.
///
/// What stood here said `free it on every path or on none` unconditionally. On
/// a program with no `free` in it that is an instruction to add one, and adding
/// it turns the warning into a proved use after free: the corpus case
/// `an_unsequenced_use_before_an_opaque_call_is_reported` allocates, calls two
/// functions, frees nothing, and carried exactly that advice at exit 0. A
/// remedy is a claim in the way a label is, and that one claimed a free
/// this check never found.
///
/// `if it is freed` is what makes one sentence true of both causes while
/// keeping the action a reader of the first can take. Naming no free at all
/// would be safe the way `LOST_REMEDY` is safe, and would cost the reader whose
/// paths really do disagree the one thing they could have done about it.
const DISAGREEMENT_REMEDY: &str = "if it is freed, free it on every path or on none, and keep it \
                                   out of a call this check cannot read in between";

/// What a memory check concluded, as what a user reads.
///
/// The check names a conclusion and never reads the policy, so this does not
/// either: `Diagnostic::concluded` is the one place a conclusion becomes a
/// severity and `DiagnosticSink` is the one place the policy is applied.
/// ADR-0001 is why there is one of each.
///
/// The value is not named. `docs/safety-model.md` writes "use of freed value
/// `p`" and the IR holds no `p`: a local is a type and an index, which is #136.
/// The caret goes on the call or the use, so the quoted line above it shows
/// `free(p)` or `*p` and the reader finds the name in their own text.
pub(super) fn memory_finding(finding: &memory::Finding) -> Option<Diagnostic> {
    // The label says only as much as the conclusion does, and says it in words
    // nothing else here uses. "freed again" asserts there was a first time,
    // which an unproven result does not know, and reusing the word the other
    // diagnostic spends on the *earlier, legitimate* free would have a reader
    // who learned that pair take the suspect for the safe one.
    //
    // **A reason names the analysis where the analysis is all there is.**
    // `Unproven::Lost` is this check having stopped following the
    // pointer, so nothing about the *program* was established and there is
    // nothing about the program to say. Giving those the words below put
    // `may free it again here` on a file with one `free` in it, and
    // `used here, perhaps after the free` on one with none. The other two
    // reasons each have a free behind them: a site the paths disagree about
    // was freed on one of them, and a site an opaque call was handed may have
    // been freed by it.
    //
    // The reason is not read beside `Unsafe`, where there is nothing
    // unproven, and each `Unknown` row names every reason rather than taking
    // `_`, so that a fourth cannot fall into a row written before it existed.
    // An arm that means two things at once reports the second as the first,
    // and this is that read forwards. Its limit is that `E0004` makes somebody
    // write an arm and does not make the arm right.
    // **A remedy is a claim like a label is**, so each of
    // these is written against what its row established and not against what
    // its words suggest. Two rows are worth saying out loud. `Lost` names no
    // cause because `Unproven::Lost` has five producers and a `Finding` does
    // not say which answered, so anything more specific would be right for some
    // of them and false for the rest; #213 carries the reason and replaces it
    // with five. `Disagreement` has two causes of its own, paths that disagree
    // and a call this check cannot read, so its remedy names what would let
    // this check conclude rather than which of the two happened.
    //
    // `Unsequenced` is split out of the group it shares its words with, and
    // only because its remedy differs: C decided that one, and what to change
    // is the statement boundary rather than the free. The message and the label
    // are written twice as a result, which is the same trade this function
    // already makes by naming every reason rather than taking `_`.
    let (code, message, label, remedy) = match (finding.kind, finding.conclusion, finding.unproven)
    {
        (Kind::DoubleFree, Conclusion::Unsafe, _) => (
            DOUBLE_FREE,
            "this frees a value that was freed already",
            "freed again here",
            "remove one of the two frees, or take this one off the path that reaches the first",
        ),
        (Kind::DoubleFree, Conclusion::Unknown, Some(Unproven::Lost)) => (
            DOUBLE_FREE,
            "this frees a pointer this check stopped following",
            "this check cannot say what this points at",
            LOST_REMEDY,
        ),
        // **`Unsequenced` is split out below and not here**, which is the one
        // place the two kinds are shaped differently. No corpus case reaches a
        // double free that is unproven for that reason, and two written to try
        // were reported as proved double frees instead:
        // `(free(p), 0) + (free(p), 0)` and `h((free(p), 0), (free(p), 0))`.
        // That is what was measured, rather than a claim that none exists. It
        // reads as the shape of the question: ADR-0023 carries a *read*
        // forwards to the free it is unordered against, and a second free is
        // the same defect whichever of the two runs first, so an open order
        // takes nothing away from it.
        //
        // A row here would therefore be one nothing can break, and the group
        // keeps the remedy it had. `DISAGREEMENT_REMEDY` asks for the free
        // conditionally, so it stays true of a double free that arrives this
        // way if one ever does.
        // `Unproven::Offset` is here and in the `UseAfterFree` row below
        // although neither kind can carry it: `memory::report::interior` is the
        // only producer and it builds only an `InteriorFree`. Named rather than
        // taken by `_`, for the reason above.
        (
            Kind::DoubleFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Unsequenced | Unproven::Offset) | None,
        ) => (
            DOUBLE_FREE,
            "this may free a value that was freed already",
            "may free it again here",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::UseAfterFree, Conclusion::Unsafe, _) => (
            USE_AFTER_FREE,
            "this uses a value after it was freed",
            "used here",
            "move the free after this use, or do not free here",
        ),
        (Kind::UseAfterFree, Conclusion::Unknown, Some(Unproven::Lost)) => (
            USE_AFTER_FREE,
            "this uses a pointer this check stopped following",
            "this check cannot say what this points at",
            LOST_REMEDY,
        ),
        (Kind::UseAfterFree, Conclusion::Unknown, Some(Unproven::Unsequenced)) => (
            USE_AFTER_FREE,
            "this may use a value after it was freed",
            "used here, perhaps after the free",
            UNSEQUENCED_REMEDY,
        ),
        (
            Kind::UseAfterFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Offset) | None,
        ) => (
            USE_AFTER_FREE,
            "this may use a value after it was freed",
            "used here, perhaps after the free",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::ReturnAfterFree, Conclusion::Unsafe, _) => (
            RETURN_AFTER_FREE,
            "this returns a pointer to an allocation that was freed",
            "returned here",
            "return a pointer that is still allocated, or do not free this one before returning it",
        ),
        (Kind::ReturnAfterFree, Conclusion::Unknown, Some(Unproven::Lost)) => (
            RETURN_AFTER_FREE,
            "this returns a pointer this check stopped following",
            "this check cannot say what this points at",
            LOST_REMEDY,
        ),
        // `Unsequenced` cannot arrive, because `memory::report::verdict`
        // answers that a `return` is ordered after every free in its
        // expression, and `Offset` cannot because only
        // `memory::report::interior` builds it. Named rather than taken by `_`,
        // for the reason the double-free rows give.
        (
            Kind::ReturnAfterFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Unsequenced | Unproven::Offset) | None,
        ) => (
            RETURN_AFTER_FREE,
            "this may return a pointer to an allocation that was freed",
            "returned here, perhaps after the free",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::ArgumentAfterFree, Conclusion::Unsafe, _) => (
            ARGUMENT_AFTER_FREE,
            "this passes a pointer to an allocation that was freed",
            "passed here",
            "pass a pointer that is still allocated, or do not free this one before passing it",
        ),
        (Kind::ArgumentAfterFree, Conclusion::Unknown, Some(Unproven::Lost)) => (
            ARGUMENT_AFTER_FREE,
            "this passes a pointer this check stopped following",
            "this check cannot say what this points at",
            LOST_REMEDY,
        ),
        (Kind::ArgumentAfterFree, Conclusion::Unknown, Some(Unproven::Unsequenced)) => (
            ARGUMENT_AFTER_FREE,
            "this may pass a pointer to an allocation that was freed",
            "passed here, perhaps after the free",
            UNSEQUENCED_REMEDY,
        ),
        // `Offset` cannot arrive, because only `memory::report::interior`
        // builds it. Named rather than taken by `_`, for the reason the
        // double-free rows give.
        (
            Kind::ArgumentAfterFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Offset) | None,
        ) => (
            ARGUMENT_AFTER_FREE,
            "this may pass a pointer to an allocation that was freed",
            "passed here, perhaps after the free",
            DISAGREEMENT_REMEDY,
        ),
        // **Words of its own, under the same code**: what is passed is live,
        // and `a pointer to an allocation that was freed` would be false about
        // `use2(&a)`. The fix is the same, at the call or at the free. Only
        // `Disagreement` arrives, because `memory::known::Known::handed_below`
        // always answers `Reached::Partial` beside a site freed or unproven;
        // every other reason is named rather than taken by `_`, for the reason
        // the double-free rows give. The label says `perhaps` and the remedy
        // has `DISAGREEMENT_REMEDY`'s two halves because the site may be
        // unproven rather than freed: a free on one path, or a call this check
        // cannot read. See ADR-0042.
        (
            Kind::FreedBehindArgument,
            Conclusion::Unsafe | Conclusion::Unknown,
            Some(
                Unproven::Disagreement | Unproven::Lost | Unproven::Unsequenced | Unproven::Offset,
            )
            | None,
        ) => (
            ARGUMENT_AFTER_FREE,
            "this may pass a pointer to where a freed pointer is stored",
            "passed here, perhaps holding a freed pointer behind it",
            "set a pointer stored there to null once it is freed, free it on every path or on \
             none, and keep it out of a call this check cannot read before this one",
        ),
        (Kind::InteriorFree, Conclusion::Unsafe, _) => (
            INTERIOR_FREE,
            "this frees a pointer that is not the start of an allocation",
            "not the start of the allocation",
            "free the pointer the allocation was made with, and keep the offset in a variable \
             of its own",
        ),
        // **The remedy is conditional because the offset may really be
        // zero.** What this check could not evaluate is the offset, not the
        // program, and an instruction that holds only where the offset is not
        // zero has to say so: a remedy is a claim like a label is, which is
        // `DISAGREEMENT_REMEDY`'s own reason.
        //
        // Every reason named, although only `Offset` arrives: the other three
        // come out of `verdict`, which `memory::report::interior` does not
        // call.
        (
            Kind::InteriorFree,
            Conclusion::Unknown,
            Some(
                Unproven::Offset | Unproven::Lost | Unproven::Disagreement | Unproven::Unsequenced,
            )
            | None,
        ) => (
            INTERIOR_FREE,
            "this may free a pointer that is not the start of an allocation",
            "this offset may not be zero",
            "free the pointer the allocation was made with if this offset can be non-zero",
        ),
        // Neither check answers this and `Diagnostic::concluded` gives `None`
        // for it, so none of the four is read. Written out rather than `_` so
        // that a fourth conclusion has to be answered for here.
        (_, Conclusion::Safe, _) => (DOUBLE_FREE, "nothing", "nothing", "nothing"),
    };

    let mut diagnostic = Diagnostic::concluded(finding.conclusion, message, Remedy::new(remedy))?
        .with_code(code)
        .with_safety_level(SafetyLevel::Memory)
        .with_label(Label::primary(finding.at, label));

    // These two mean the same thing under either conclusion and under either
    // check, so they share their words where the primary does not. Each is
    // attached only where the check knows it: what makes a finding `Unknown` is
    // that the paths or the sites reaching it disagree, and there is then no
    // single place to point at.
    if let Some(freed) = finding.freed {
        diagnostic = diagnostic.with_label(Label::secondary(freed, "freed here"));
    }
    // **Why this one is unproven, because it is not the usual why.** Every
    // other `Unknown` here is the check having lost something; this is the
    // check having found nothing that orders the free first. Without the note
    // a reader sees two carets and a warning and has no way to tell which of
    // the two it is, and neither is something more of the same analysis would
    // fix. See ADR-0022.
    //
    // **It says what this check found and not what C decided**, because the
    // two are not the same: the order really is open in `*p + (free(p), 0)`,
    // and in `x = (free(p), *p)` C17 6.5.17 p2 settles it while this check
    // still records nothing, which is #178. A note claiming C left those
    // unsequenced would be false about the second.
    //
    // 6.5.2.2 p10 rather than 6.5 p3, which the first draft of this cited: p3
    // leaves subexpressions *unsequenced*, and unsequenced evaluations may
    // interleave. What makes a call one order or the other is p10's
    // indeterminate sequencing, and a free is a call.
    if finding.unproven == Some(Unproven::Unsequenced) {
        diagnostic = diagnostic.with_note(
            "C17 6.5.2.2 p10 leaves a call indeterminately sequenced with the rest of its \
             expression, and this check found nothing here that orders the free before this",
        );
    }
    if let Some(made) = finding.made {
        diagnostic = diagnostic.with_label(Label::secondary(made, "allocated here"));
    }

    Some(diagnostic)
}

/// What the nullability check concluded, as what a user reads.
///
/// The check names a conclusion and never reads the policy, so this does not
/// either, for `memory_finding`'s reason and ADR-0001's.
///
/// **Two rows per question.** The memory check's words differ by which of its
/// two questions was asked and by why an answer was unproven; there are two
/// questions here, a dereference and an argument to a `_Nonnull` parameter, and
/// one reason, which is that nothing established what the pointer holds. A
/// reason enum with one variant is a field nobody reads.
///
/// The value is not named, for `memory_finding`'s reason: the IR holds no `p`,
/// which is #136, and the caret's quoted line shows the reader their own text.
pub(super) fn nullability_finding(finding: &nullability::Finding) -> Option<Diagnostic> {
    // The unproven row does not say "may be null" of the *pointer* and then
    // blame the dereference: what this check failed to establish is that the
    // pointer is not null, and a reader who is told the pointer may be null is
    // being told something was worked out about it. Nothing was.
    //
    // Every conclusion written out rather than `_`, so that a fourth has to be
    // answered for here. `Safe` is unreachable through `Nullness::concluded`
    // and `Diagnostic::concluded` gives `None` for it either way.
    // The unproven row asks for a test rather than for a value, because what
    // this check failed to establish is that the pointer is not null and a test
    // is what would establish it. Telling a reader to give it a value there
    // would assert that it has none, which is the row above's sentence and not
    // this one's, and a remedy is a claim like a label is.
    //
    // The argument rows follow the same rule, and their unsafe remedy offers
    // the promise as the other way out, because a parameter declared
    // `_Nonnull` that a caller has a reason to pass null to is a promise that
    // was wrong rather than a call that was.
    let (code, message, label, remedy) = match (finding.asked, finding.conclusion) {
        (Asked::Dereference { through_memory: _ }, Conclusion::Unsafe) => (
            NULL_DEREFERENCE,
            "this dereferences a null pointer",
            "this is null when it is read through",
            "give this pointer a value before reading through it, or do not read through it here",
        ),
        (
            Asked::Dereference {
                through_memory: false,
            },
            Conclusion::Unknown,
        ) => (
            NULL_DEREFERENCE,
            "this may dereference a null pointer",
            "this check cannot say this is not null",
            "test this pointer against null before reading through it",
        ),
        // A pointer read out of memory and read through in place, `**pp`,
        // which a test of `*pp` does not settle: this check keeps nothing
        // about `*pp`, so the row above would be advice to do what the
        // program may already do (#333). A test of the local itself settles
        // it, `q != 0` or `if (q)` since #334. A test of an assignment,
        // `if ((q = *pp))`, does not yet, because the branch reads a temporary
        // the assigned value was copied into (#336).
        (
            Asked::Dereference {
                through_memory: true,
            },
            Conclusion::Unknown,
        ) => (
            NULL_DEREFERENCE,
            "this may dereference a null pointer",
            "this check cannot say this is not null",
            "read each pointer this goes through into a local, and test that local against null before reading through it",
        ),
        (Asked::Argument { promise }, conclusion) => {
            return passed(finding.at, conclusion, promise);
        }
        (
            Asked::Return {
                promise,
                reached_end,
            },
            conclusion,
        ) => return returned(finding.at, conclusion, promise, reached_end),
        (Asked::Dereference { through_memory: _ }, Conclusion::Safe) => {
            (NULL_DEREFERENCE, "nothing", "nothing", "nothing")
        }
    };

    Some(
        Diagnostic::concluded(finding.conclusion, message, Remedy::new(remedy))?
            .with_code(code)
            .with_safety_level(SafetyLevel::Memory)
            .with_label(Label::primary(finding.at, label)),
    )
}

/// What the nullability check concluded about an argument, as what a user
/// reads.
///
/// Its own function for the reason [`returned`] is: what the parameter
/// promised decides what it is called, how the promise is taken back, and
/// which level made it.
///
/// **Nothing makes a parameter's promise by default yet.** Every function here
/// has external linkage, and level 5 makes a parameter of one `_Nullable`
/// where it says neither (ADR-0050). The `Defaulted` words are written so that
/// the day a function with internal linkage is read, a parameter level 5 made
/// non-null is not reported as declared `_Nonnull`.
///
/// The unsafe remedy offers the promise as the other way out, because a
/// parameter promised not null that a caller has a reason to pass null to is a
/// promise that was wrong rather than a call that was.
fn passed(at: Span, conclusion: Conclusion, promise: Promise) -> Option<Diagnostic> {
    let (promised_at, described, promise_label, take_back, level) = match promise {
        Promise::Written(written) => (
            written,
            "a parameter declared `_Nonnull`",
            "the parameter is declared `_Nonnull` here",
            "remove `_Nonnull` from the parameter",
            SafetyLevel::Memory,
        ),
        Promise::Defaulted(name) => (
            name,
            "a parameter that is not `_Nullable`",
            "at level 5 this parameter is not null, because it is not written `_Nullable`",
            "write `_Nullable` after the `*` of the parameter in every declaration of the function",
            SafetyLevel::Strict,
        ),
    };
    let (message, label, remedy) = match conclusion {
        Conclusion::Unsafe => (
            format!("this passes a null pointer to {described}"),
            "this is null when it is passed",
            format!("pass a pointer to an object here, or {take_back}"),
        ),
        Conclusion::Unknown => (
            format!("this may pass a null pointer to {described}"),
            "this check cannot say this is not null",
            "test this pointer against null before passing it".to_owned(),
        ),
        Conclusion::Safe => return None,
    };

    Some(
        Diagnostic::concluded(conclusion, message, Remedy::new(remedy))?
            .with_code(NULL_ARGUMENT)
            .with_safety_level(level)
            .with_label(Label::primary(at, label))
            .with_label(Label::secondary(promised_at, promise_label)),
    )
}

/// What the nullability check concluded about a `return`, as what a user
/// reads.
///
/// Its own function because its remedy depends on why the return was
/// promised, which is a value rather than a row: a written `_Nonnull` is taken
/// back by removing it, and level 5's default by writing `_Nullable`. The
/// level named is the one that made the promise, so a default says level 5.
///
/// **The end of a body is told to end with a `return`**, whatever was
/// concluded, because there is no pointer to test and `_Nullable` would not
/// make the value the caller reads any less indeterminate (C17 6.9.1 p12).
fn returned(
    at: Span,
    conclusion: Conclusion,
    promise: Promise,
    reached_end: bool,
) -> Option<Diagnostic> {
    let (promised_at, promise_label, take_back, level) = match promise {
        Promise::Written(written) => (
            written,
            "the return is declared `_Nonnull` here",
            "remove `_Nonnull` from the return type",
            SafetyLevel::Memory,
        ),
        Promise::Defaulted(name) => (
            name,
            "at level 5 this returns a pointer that is not null, because it is not written `_Nullable`",
            "write `_Nullable` after the `*` of the return type",
            SafetyLevel::Strict,
        ),
    };
    let (message, label, remedy) = match (reached_end, conclusion) {
        (true, Conclusion::Unsafe | Conclusion::Unknown | Conclusion::Safe) => (
            "this function may reach the end of its body without returning a pointer",
            "the end of this function's body can be reached without a `return`",
            "end every path through the body with a `return`".to_owned(),
        ),
        (false, Conclusion::Unsafe) => (
            "this returns a null pointer from a function that promised not to",
            "this is null when it is returned",
            format!(
                "return a pointer to an object here, or {take_back} in every declaration of the function"
            ),
        ),
        (false, Conclusion::Unknown) => (
            "this may return a null pointer from a function that promised not to",
            "this check cannot say this is not null",
            format!(
                "return only a pointer this check can see is not null, testing it first where it came from a call, or {take_back} in every declaration of the function"
            ),
        ),
        (false, Conclusion::Safe) => return None,
    };

    let mut diagnostic = Diagnostic::concluded(conclusion, message, Remedy::new(remedy))?
        .with_code(NULL_RETURN)
        .with_safety_level(level)
        .with_label(Label::primary(at, label));
    // A default promise at the end of a body points at the function's name
    // twice, and one label there is enough.
    if promised_at != at {
        diagnostic = diagnostic.with_label(Label::secondary(promised_at, promise_label));
    }
    Some(diagnostic)
}
