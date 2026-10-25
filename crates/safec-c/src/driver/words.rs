//! What a safety finding reads as: the code, the message, the label and the
//! remedy each conclusion of the memory and nullability checks is reported with.
//!
//! One `match` per check, with every case written out rather than taken by `_`,
//! so that a kind or a reason added to a check is `error[E0004]` here until
//! somebody says what a reader is told about it.

use crate::diagnostics::{Code, Diagnostic, Label, Remedy};
use crate::safety::SafetyLevel;
use safec_ir::analysis::Conclusion;
use safec_ir::ir::{Between, Promise};
use safec_ir::memory::{self, Departure, Kind, LostReason, Unproven};
use safec_ir::nullability::{self, Asked};
use safec_ir::source::{SourceMap, Span};

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

/// A pointer local one operand writes and another reads or writes: two
/// arguments of a call, or two operands of an operator that does not sequence.
///
/// Not about what the pointer holds, so not any of the codes above: C17 6.5
/// p2 makes it undefined whatever that is, and the fix is to put the write in
/// a statement of its own rather than to change the pointer. See ADR-0052.
const UNSEQUENCED_ACCESS: Code = Code::new("SC0409");

/// A function the memory check did not read because it is too large.
///
/// Not any of the codes above, which are each a class of program to fix: this
/// is about this check, not the program, and the fix is a smaller function.
const TOO_LARGE: Code = Code::new("SC0410");
const UNASSIGNED: Code = Code::new("SC0411");

/// An access that may not fit the object its pointer reaches.
///
/// A class of its own rather than any code above, because it is about where
/// in an object an access lands and not whether the object is live: a use
/// after free is about the allocation, and this is about its size. An
/// allocation too small and a pointer off its start share it, being one
/// question. See ADR-0060.
const OUTSIDE_ITS_OBJECT: Code = Code::new("SC0412");

/// A pointer handed on that may not point at a whole object.
///
/// Not `SC0412`, because nothing has been read or written through it yet:
/// it is where another function or a later load will believe it whole, and
/// the fix is in what is handed on rather than in an access. See ADR-0060.
const HANDED_ON_NARROW: Code = Code::new("SC0413");

/// What to change where this check stopped following a pointer and cannot
/// say why: `LostReason::Other`.
///
/// **It names no cause, and that is the whole of its design.** `Other`
/// covers every cause the lattice keeps in one bit, and more than one reason
/// at once, so a remedy naming one would be right for some and false for the
/// others: a conservative over-approximation given a confident word. So this
/// says only what is true of all of them, and tells the reader the one thing
/// that matters here, which is not to go hunting for a defect. The reasons a
/// producer can tell apart get their own words in [`lost_words`] (#213).
const LOST_REMEDY: &str = "nothing here says the program is wrong: this check could no longer \
                           say which allocation this pointer holds";

/// The label and the remedy for a pointer this check stopped following,
/// by why, as far as the producer could tell.
///
/// **The message is the kind's and these are the reason's**, so a reason
/// added later is `E0004` here and nowhere in the table above it. Each says
/// what is true of its producer and no more: none names a cause the
/// producer did not establish, and `MayBeALocal`, which may be a defect C
/// forbids, says it could not be ruled out rather than that it is there.
///
/// **Only a free reaches a reason other than `Other`.** The one producer
/// that tells reasons apart, `Allocations::touching`, answers a free or a
/// `realloc`; a use, a return and an argument are asked through producers
/// that always say `Other`. So these words are written for a free, and a
/// producer added for another kind has to come back here. See #213.
fn lost_words(reason: LostReason) -> (&'static str, &'static str) {
    match reason {
        LostReason::ReadOutOfMemory => (
            "this was read out of memory, which this check does not follow",
            "nothing here says the program is wrong: this check follows the local an \
             allocation was made into, not what memory holds",
        ),
        LostReason::NoSiteKnown => (
            "this check knows of no allocation this points at",
            "nothing here says the program is wrong: this check knows of no \
             allocation this pointer holds here",
        ),
        LostReason::MayBeALocal => (
            "this may be the address of a local",
            "this check could not rule out that this holds a local's address, \
             which free must never be handed; if it is always what an allocation \
             function returned, nothing here says the program is wrong",
        ),
        LostReason::Other => ("this check cannot say what this points at", LOST_REMEDY),
    }
}

/// What to change where C has not ordered the free against the use.
///
/// The only remedy here about something C decided rather than about what the
/// program meant, which is why `Unsequenced` is a row of its own rather than
/// sharing one with `Disagreement`. See ADR-0022, and the note attached below
/// for what the check did and did not find.
const UNSEQUENCED_REMEDY: &str = "put the free and this in separate statements, so that C orders \
                                  one before the other";

/// What to change where the free C has not ordered is in a parameter's
/// length, where no statement can be written: every length is evaluated on
/// entry (C17 6.9.1 p10), and the body is executed after that (p11), so a
/// free there is ordered.
const LENGTH_REMEDY: &str =
    "free it in the body instead, where every parameter's length has already been evaluated";

/// What to change where a pointer is written in a conflict inside a
/// parameter's length, or between two lengths: no statement can be written
/// in a declarator, and the body runs after every length (#529, #531).
const WRITE_IN_BODY_REMEDY: &str =
    "write it in the body instead, where every parameter's length has already been evaluated";

/// Where the free and the use an [`Unproven::Unsequenced`] finding names were
/// written, which decides what C left unordered between them (#444).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unordered {
    /// In the body, in one expression: the call is indeterminately sequenced
    /// with the rest of it (C17 6.5.2.2 p10).
    InTheBody,
    /// Both in one parameter's length, which is one expression too, so the
    /// same clause holds, but no statement can be written there.
    InOneLength,
    /// In two parameters' lengths, which nothing orders against each other:
    /// a parameter's declarator is not a full declarator, so its end is no
    /// sequence point (C17 6.7.6 p3).
    AcrossLengths,
}

/// Which of the three a finding is, from the free it names, `freed`, the
/// caret, `at`, and the span of every parameter length a definition in this
/// unit evaluates on entry.
///
/// **The free's span alone says whether it is about the lengths.** A free in a
/// length is ordered before the whole body by the marker after the lengths,
/// so a use in the body is proved rather than left open, and a free in the
/// body can never be open against a length, which ran first. Lengths of two
/// definitions never overlap, so no function has to be looked up. A free
/// outside every length is [`Unordered::InTheBody`], which is also the answer
/// for a finding that names no free, where none of the three is read.
///
/// Spans rather than the finding, so the rule can be asked about two files.
pub(super) fn unordered(
    sources: &SourceMap,
    freed: Option<Span>,
    at: Span,
    lengths: &[Span],
) -> Unordered {
    // Located, not compared as offsets: a span names its own file, and a free
    // in an included file is inside the length that holds its `#include`, as
    // it would be written out. Two spans with no file in common are not inside
    // each other. See ADR-0054.
    let within = |outer: Span, inner: Span| sources.within(outer, inner);
    let Some(freed) = freed else {
        return Unordered::InTheBody;
    };
    match lengths.iter().find(|length| within(**length, freed)) {
        None => Unordered::InTheBody,
        Some(length) if within(*length, at) => Unordered::InOneLength,
        Some(_) => Unordered::AcrossLengths,
    }
}

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

/// A message that names the value it is about, where there is a name, and
/// the one that names nothing where there is not.
///
/// Both written at each row rather than one built from the other, because
/// the named one is not the other with a word replaced: `` this returns `p`,
/// which points to an allocation that was freed `` has a clause the unnamed
/// one does not need.
fn said(name: Option<&str>, unnamed: &str, named: impl FnOnce(&str) -> String) -> String {
    name.map_or_else(|| unnamed.to_owned(), named)
}

/// What a memory check concluded, as what a user reads.
///
/// The check names a conclusion and never reads the policy, so this does not
/// either: `Diagnostic::concluded` is the one place a conclusion becomes a
/// severity and `DiagnosticSink` is the one place the policy is applied.
/// ADR-0001 is why there is one of each.
///
/// **The message names the value where the source wrote it**, `name`, which
/// the driver looks up from [`memory::Finding::local`]: `docs/safety-model.md`
/// writes "use of freed value `p`". Where the conclusion is about a temporary
/// or about a pointer read out of memory there is no name, and the message is
/// the one that names nothing, since `p[2]` is not a use of `p` and `free(*pp)`
/// is not a free of `pp`. Only the message: the labels are about other reads,
/// which the finding does not say the local of. A read out of fresh memory is
/// the exception, named as `*name` after the pointer it is read through.
///
/// `lengths` is every parameter length a definition evaluates on entry, for
/// [`unordered`].
pub(super) fn memory_finding(
    sources: &SourceMap,
    finding: &memory::Finding,
    name: Option<&str>,
    lengths: &[Span],
) -> Option<Diagnostic> {
    let unordered = unordered(sources, finding.freed, finding.at, lengths);
    // Nowhere in a declarator can a statement be written, whichever clause
    // left the two unordered there.
    let unsequenced_remedy = match unordered {
        Unordered::InTheBody => UNSEQUENCED_REMEDY,
        Unordered::InOneLength | Unordered::AcrossLengths => LENGTH_REMEDY,
    };
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
    // `used here, perhaps after the free` on one with none.
    //
    // **And a `Disagreement` finding cannot say which free, or whether
    // there was one**, so its words do not assert one. A site the paths
    // disagree about was freed on one of them, and a site a call this check
    // cannot read was handed may have been freed by it or by nothing, and
    // `SiteState::Unknown` is one variant for both, so nothing here can say
    // which cause it was. Nor does such a finding carry a `freed` span:
    // "perhaps after the free" named a free the report never showed, and in
    // a program with no `free` in it, one there was not.
    // `a_doubt_with_no_free_in_the_program_asserts_none` is that program.
    // What certainly happened is the use, the pass, the return or the free
    // at the caret, so the `may` is on whether the pointer had been freed.
    // `Unsequenced` keeps its words, because its `freed here` label is the
    // free they mean.
    //
    // The reason is not read beside `Unsafe`, where there is nothing
    // unproven, and each `Unknown` row names every reason rather than taking
    // `_`, so that a fourth cannot fall into a row written before it existed.
    // An arm that means two things at once reports the second as the first,
    // and this is that read forwards. Its limit is that `E0004` makes somebody
    // write an arm and does not make the arm right.
    // **A remedy is a claim like a label is**, so each of
    // these is written against what its row established and not against what
    // its words suggest. Two rows are worth saying out loud. `Lost` carries
    // why, as far as its producer can tell, and `lost_words` gives each reason
    // what is true of it; `Other` keeps the remedy that names no cause, for
    // the causes one bit holds together (#213). `Disagreement` has two causes
    // of its own, paths that disagree
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
            said(name, "this frees a value that was freed already", |name| {
                format!("this frees `{name}`, which was freed already")
            }),
            "freed again here",
            "remove one of the two frees, or take this one off the path that reaches the first",
        ),
        (Kind::DoubleFree, Conclusion::Unknown, Some(Unproven::Lost(reason))) => {
            let (label, remedy) = lost_words(reason);
            (
                DOUBLE_FREE,
                said(
                    name,
                    "this frees a pointer this check stopped following",
                    |name| format!("this frees `{name}`, which this check stopped following"),
                ),
                label,
                remedy,
            )
        }
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
            Some(
                Unproven::Disagreement
                | Unproven::Unsequenced
                | Unproven::Offset
                | Unproven::Extent,
            )
            | None,
        ) => (
            DOUBLE_FREE,
            said(
                name,
                "this frees a value that may have been freed already",
                |name| format!("this frees `{name}`, which may have been freed already"),
            ),
            "this may be its second free",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::UseAfterFree, Conclusion::Unsafe, _) => (
            USE_AFTER_FREE,
            said(name, "this uses a value after it was freed", |name| {
                format!("this uses `{name}` after it was freed")
            }),
            "used here",
            "move the free after this use, or do not free here",
        ),
        (Kind::UseAfterFree, Conclusion::Unknown, Some(Unproven::Lost(reason))) => {
            let (label, remedy) = lost_words(reason);
            (
                USE_AFTER_FREE,
                said(
                    name,
                    "this uses a pointer this check stopped following",
                    |name| format!("this uses `{name}`, which this check stopped following"),
                ),
                label,
                remedy,
            )
        }
        (Kind::UseAfterFree, Conclusion::Unknown, Some(Unproven::Unsequenced)) => (
            USE_AFTER_FREE,
            said(name, "this may use a value after it was freed", |name| {
                format!("this may use `{name}` after it was freed")
            }),
            "used here, perhaps after the free",
            unsequenced_remedy,
        ),
        (
            Kind::UseAfterFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Offset | Unproven::Extent) | None,
        ) => (
            USE_AFTER_FREE,
            said(name, "this uses a value that may have been freed", |name| {
                format!("this uses `{name}`, which may have been freed")
            }),
            "used here, where it may have been freed before",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::ReturnAfterFree, Conclusion::Unsafe, _) => (
            RETURN_AFTER_FREE,
            said(
                name,
                "this returns a pointer to an allocation that was freed",
                |name| {
                    format!("this returns `{name}`, which points to an allocation that was freed")
                },
            ),
            "returned here",
            "return a pointer that is still allocated, or do not free this one before returning it",
        ),
        (Kind::ReturnAfterFree, Conclusion::Unknown, Some(Unproven::Lost(reason))) => {
            let (label, remedy) = lost_words(reason);
            (
                RETURN_AFTER_FREE,
                said(
                    name,
                    "this returns a pointer this check stopped following",
                    |name| format!("this returns `{name}`, which this check stopped following"),
                ),
                label,
                remedy,
            )
        }
        // `Unsequenced` cannot arrive, because `memory::report::verdict`
        // answers that a `return` is ordered after every free in its
        // expression, and `Offset` cannot because only
        // `memory::report::interior` builds it. Named rather than taken by `_`,
        // for the reason the double-free rows give.
        (
            Kind::ReturnAfterFree,
            Conclusion::Unknown,
            Some(
                Unproven::Disagreement
                | Unproven::Unsequenced
                | Unproven::Offset
                | Unproven::Extent,
            )
            | None,
        ) => (
            RETURN_AFTER_FREE,
            said(
                name,
                "this returns a pointer that may point to a freed allocation",
                |name| format!("this returns `{name}`, which may point to a freed allocation"),
            ),
            "returned here, where it may have been freed before",
            DISAGREEMENT_REMEDY,
        ),
        (Kind::ArgumentAfterFree, Conclusion::Unsafe, _) => (
            ARGUMENT_AFTER_FREE,
            said(
                name,
                "this passes a pointer to an allocation that was freed",
                |name| {
                    format!("this passes `{name}`, which points to an allocation that was freed")
                },
            ),
            "passed here",
            "pass a pointer that is still allocated, or do not free this one before passing it",
        ),
        (Kind::ArgumentAfterFree, Conclusion::Unknown, Some(Unproven::Lost(reason))) => {
            let (label, remedy) = lost_words(reason);
            (
                ARGUMENT_AFTER_FREE,
                said(
                    name,
                    "this passes a pointer this check stopped following",
                    |name| format!("this passes `{name}`, which this check stopped following"),
                ),
                label,
                remedy,
            )
        }
        (Kind::ArgumentAfterFree, Conclusion::Unknown, Some(Unproven::Unsequenced)) => (
            ARGUMENT_AFTER_FREE,
            said(
                name,
                "this may pass a pointer to an allocation that was freed",
                |name| {
                    format!("this may pass `{name}`, which points to an allocation that was freed")
                },
            ),
            "passed here, perhaps after the free",
            unsequenced_remedy,
        ),
        // `Offset` cannot arrive, because only `memory::report::interior`
        // builds it. Named rather than taken by `_`, for the reason the
        // double-free rows give.
        (
            Kind::ArgumentAfterFree,
            Conclusion::Unknown,
            Some(Unproven::Disagreement | Unproven::Offset | Unproven::Extent) | None,
        ) => (
            ARGUMENT_AFTER_FREE,
            said(
                name,
                "this passes a pointer that may point to a freed allocation",
                |name| format!("this passes `{name}`, which may point to a freed allocation"),
            ),
            "passed here, where it may have been freed before",
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
                Unproven::Disagreement
                | Unproven::Lost(_)
                | Unproven::Unsequenced
                | Unproven::Offset
                | Unproven::Extent,
            )
            | None,
        ) => (
            ARGUMENT_AFTER_FREE,
            "this passes a pointer to where a pointer that may have been freed is stored"
                .to_owned(),
            "passed here, perhaps holding a freed pointer behind it",
            "set a pointer stored there to null once it is freed, free it on every path or on \
             none, and keep it out of a call this check cannot read before this one",
        ),
        (Kind::InteriorFree, Conclusion::Unsafe, _) => (
            INTERIOR_FREE,
            said(
                name,
                "this frees a pointer that is not the start of an allocation",
                |name| format!("this frees `{name}`, which is not the start of an allocation"),
            ),
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
        //
        // **The named message moves the `may` onto the offset**, the one row
        // whose named form is not the unnamed one's shape, for the same
        // reason: the free certainly happens, and what is open is whether
        // `q` is at the start, which is what the label says too. Kept where
        // the unnamed form puts it, `` this may free `q`, which is not the
        // start ``, it would say flatly that `q` is not the start, which is
        // false wherever the offset is zero.
        (
            Kind::InteriorFree,
            Conclusion::Unknown,
            Some(
                Unproven::Offset
                | Unproven::Extent
                | Unproven::Lost(_)
                | Unproven::Disagreement
                | Unproven::Unsequenced,
            )
            | None,
        ) => (
            INTERIOR_FREE,
            said(
                name,
                "this may free a pointer that is not the start of an allocation",
                |name| format!("this frees `{name}`, which may not be the start of an allocation"),
            ),
            "this offset may not be zero",
            "free the pointer the allocation was made with if this offset can be non-zero",
        ),
        (
            Kind::UnsequencedAccess {
                written,
                both_write,
                between,
            },
            Conclusion::Unsafe,
            _,
        ) => {
            // What the two accesses are operands of is the only difference in
            // the words, so it is chosen once. See ADR-0052.
            let (other, in_a_statement) = match between {
                Between::Arguments => (
                    "another argument of the same call",
                    "assign it in a statement of its own before the call",
                ),
                Between::Operands => (
                    "another operand of the same expression",
                    "assign it in a statement of its own before this expression",
                ),
                Between::Lengths => ("another parameter's length", WRITE_IN_BODY_REMEDY),
            };
            // **Where the two are decides the remedy, not what they are
            // operands of.** A call's arguments or an operator's operands
            // inside a parameter's length cannot be split into a statement
            // either, since a declarator holds none (#531). The two spans
            // are inside one length exactly then; across two lengths is
            // `Between::Lengths`, whose remedy this already is. `self::`
            // because the local `unordered` above answers about the free,
            // which this finding has none of.
            let remedy = match self::unordered(sources, Some(written), finding.at, lengths) {
                Unordered::InTheBody => in_a_statement,
                Unordered::InOneLength | Unordered::AcrossLengths => WRITE_IN_BODY_REMEDY,
            };
            (
                UNSEQUENCED_ACCESS,
                if both_write {
                    said(
                        name,
                        &format!("this writes a pointer {other} writes too"),
                        |name| format!("this writes `{name}`, which {other} writes too"),
                    )
                } else {
                    said(
                        name,
                        &format!("this reads a pointer {other} writes"),
                        |name| format!("this reads `{name}`, which {other} writes"),
                    )
                },
                if both_write {
                    "written here as well"
                } else {
                    "read here"
                },
                remedy,
            )
        }
        // Never built: the memory check concludes this one `Unsafe` from its
        // element and asks no lattice. Written out rather than `_`, for the
        // reason the double-free rows give.
        (Kind::UnsequencedAccess { .. }, Conclusion::Unknown, _) => (
            UNSEQUENCED_ACCESS,
            "nothing".to_owned(),
            "nothing",
            "nothing",
        ),
        // Built only `Unknown`, at the function's name, before anything about
        // the function was read; `Unsafe` is written out beside it so that the
        // row has nothing to fall into.
        (
            Kind::TooLarge { .. } | Kind::TooLong { .. },
            Conclusion::Unknown | Conclusion::Unsafe,
            _,
        ) => (
            TOO_LARGE,
            format!(
                "`{}` is too large for the memory check to read yet",
                sources.snippet(finding.at)
            ),
            "nothing about this function's memory is proved",
            "split the function into smaller ones",
        ),
        // Only ever a proof: a read some paths assigned is the next arm's.
        // `Unknown` takes the same words so the match stays exhaustive
        // without a second wording nothing produces.
        (Kind::Unassigned, Conclusion::Unsafe | Conclusion::Unknown, _) => (
            UNASSIGNED,
            said(
                name,
                "this reads a pointer before anything was assigned to it",
                |name| format!("`{name}` is read before anything was assigned to it"),
            ),
            "read here",
            "assign it where it is declared, `= 0` if nothing else",
        ),
        // Only ever a proof, as `Unassigned` is, and under its code: both are
        // a pointer read before anything was assigned to it.
        // Reported where it is read, so the name is the local it is read
        // through, and the slot is `*name`. See ADR-0057.
        (Kind::IndeterminateLoad, Conclusion::Unsafe | Conclusion::Unknown, _) => (
            UNASSIGNED,
            said(
                name,
                "this reads a pointer out of memory before anything was assigned to it",
                |name| format!("`*{name}` is read before anything was assigned to it"),
            ),
            "read here",
            "assign to it before reading it, or allocate with `calloc`",
        ),
        // Only ever `Unknown`, since the edge may be one no execution takes.
        // `Unsafe` takes the same words so the match stays exhaustive.
        (Kind::SometimesUnassigned { .. }, Conclusion::Unknown | Conclusion::Unsafe, _) => (
            UNASSIGNED,
            said(
                name,
                "this may read a pointer before anything was assigned to it",
                |name| format!("`{name}` may be read before anything was assigned to it"),
            ),
            "read here",
            "assign it on every path, `= 0` where it is declared if nothing else",
        ),
        // **The numbers are in the message and not the label**, because the
        // label says what is wrong at the caret and the numbers are why it
        // is proved. Only a proof carries them, and the object's size only
        // where every object it may reach was read as that size; where they
        // differ, every one is narrower than the access and none is named.
        // See ADR-0060.
        (Kind::OutsideItsObject { width, extent }, Conclusion::Unsafe, _) => (
            OUTSIDE_ITS_OBJECT,
            match extent {
                Some(extent) => format!(
                    "this reaches {width} {} into an object of {extent} {}",
                    bytes(width),
                    bytes(extent)
                ),
                None => format!(
                    "this reaches {width} {} into every object it may point at, and each is \
                     narrower",
                    bytes(width)
                ),
            },
            "wider than the object it reaches",
            "make the object at least as wide as this access, or reach less of it",
        ),
        // Every reason named, although only `Extent` arrives, for the reason
        // the double-free rows give. **The remedy names what this check can
        // read**, since what made it unknown may be the pointer, the size or
        // a length, and saying which would be a claim the finding does not
        // carry.
        (
            Kind::OutsideItsObject { .. },
            Conclusion::Unknown,
            Some(
                Unproven::Extent
                | Unproven::Offset
                | Unproven::Lost(_)
                | Unproven::Disagreement
                | Unproven::Unsequenced,
            )
            | None,
        ) => (
            OUTSIDE_ITS_OBJECT,
            said(
                name,
                "this may reach outside the object it points at",
                |name| format!("this may reach outside the object `{name}` points at"),
            ),
            "this check cannot say this is inside its object",
            "this check shows an access fits only through a pointer to the start of a local or \
             of an allocation of constant size, with a constant length where a call takes one",
        ),
        // **Where it left is the label**, since the three doors are one
        // question and the reader needs only to know which one this is.
        // **Only ever `Unknown`**, for the reason `Kind::HandedOnNarrow`
        // gives, and `Unsafe` takes the same words so the match stays
        // exhaustive; what was shown narrower is said in the message. The
        // reason is not read, since only `Extent` arrives.
        (
            Kind::HandedOnNarrow {
                how,
                width,
                narrower,
                extent,
            },
            Conclusion::Unknown | Conclusion::Unsafe,
            _,
        ) => (
            HANDED_ON_NARROW,
            match (narrower, extent) {
                (true, Some(extent)) => format!(
                    "this lets a pointer to {extent} {} leave where {width} {} are believed",
                    bytes(extent),
                    bytes(width)
                ),
                (true, None) => format!(
                    "this lets a pointer leave to objects each narrower than the {width} {} \
                     believed",
                    bytes(width)
                ),
                (false, _) => said(
                    name,
                    "this lets a pointer leave that may not point at an object as wide as its type",
                    |name| {
                        format!(
                            "this lets `{name}` leave, which may not point at an object as wide \
                             as its type"
                        )
                    },
                ),
            },
            departure(how),
            "let a pointer leave only at the start of a local or of an allocation of constant \
             size, as wide as the type it leaves as",
        ),
        // Neither check answers this and `Diagnostic::concluded` gives `None`
        // for it, so none of the four is read. Written out rather than `_` so
        // that a fourth conclusion has to be answered for here.
        (_, Conclusion::Safe, _) => (DOUBLE_FREE, "nothing".to_owned(), "nothing", "nothing"),
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
    // and in `*r + ((free(p), 1) && *p)` C17 6.5.13 p4 settles it while this
    // check still records nothing, because `*r` beside it may act (ADR-0022).
    // A note claiming C left those unsequenced would be false about the
    // second.
    //
    // 6.5.2.2 p10 rather than 6.5 p3, which the first draft of this cited: p3
    // leaves subexpressions *unsequenced*, and unsequenced evaluations may
    // interleave. What makes a call one order or the other is p10's
    // indeterminate sequencing, and a free is a call.
    //
    // **Not across two parameters' lengths**, where the use is not in the
    // rest of the free's expression at all and what leaves the two unordered
    // is 6.7.6 p3 (#444).
    if finding.unproven == Some(Unproven::Unsequenced) {
        let note = match unordered {
            Unordered::InTheBody | Unordered::InOneLength => {
                "C17 6.5.2.2 p10 leaves a call indeterminately sequenced with the rest of its \
                 expression, and this check found nothing here that orders the free before this"
            }
            // Not "only at the end of a full declarator": a comma inside a
            // length is a sequence point too, and p3 says nothing about
            // where else there are any. Found by review.
            Unordered::AcrossLengths => {
                "C17 6.7.6 p3 puts a sequence point at the end of a full declarator, and a \
                 parameter's declarator is not one, so nothing orders one parameter's length \
                 against another's, and this check found nothing here that orders the free \
                 before this"
            }
        };
        diagnostic = diagnostic.with_note(note);
    }
    if let Some(made) = finding.made {
        diagnostic = diagnostic.with_label(Label::secondary(made, "allocated here"));
    }
    if let Kind::TooLarge { bytes, max } = finding.kind {
        diagnostic = diagnostic.with_note(format!(
            "the check would hold about {bytes} bytes for this function, and holds at most \
             {max}"
        ));
    }
    if let Kind::SometimesUnassigned { edge, taken } = finding.kind {
        let when = if taken {
            "when this is true"
        } else {
            "when this is false"
        };
        diagnostic = diagnostic.with_label(Label::secondary(edge, when));
    }
    if matches!(
        finding.kind,
        Kind::Unassigned | Kind::SometimesUnassigned { .. }
    ) {
        diagnostic = diagnostic.with_note(
            "C17 6.3.2.1 p2: reading a local whose address is never taken before it is \
             assigned is undefined, whatever the value is read for",
        );
    }
    if finding.kind == Kind::IndeterminateLoad {
        diagnostic = diagnostic.with_note(
            "C17 7.22.3.1 p2 and 7.22.3.4 p2 leave what `aligned_alloc` and `malloc` allocate \
             indeterminate until something writes it, and Annex J.2 lists using that value as \
             undefined",
        );
    }
    if let Kind::TooLong { walks } = finding.kind {
        diagnostic = diagnostic.with_note(format!(
            "the check walked this function's blocks {walks} times without settling, the \
             most it walks one of this many locals"
        ));
    }
    // **What C says is wrong, because the caret alone does not.** The read
    // and the write are both in plain sight and look ordered left to right,
    // which C17 does not make them. See ADR-0052.
    if let Kind::UnsequencedAccess {
        written,
        both_write: _,
        between,
    } = finding.kind
    {
        let note = match between {
            Between::Arguments => {
                "C17 6.5 p2 makes this call undefined wherever both are evaluated: nothing orders \
                 a call's arguments against each other (6.5 p3), only before the call (6.5.2.2 p10)"
            }
            Between::Operands => {
                "C17 6.5 p2 makes this undefined wherever both are evaluated: nothing orders an \
                 operator's operands against each other (6.5 p3)"
            }
            Between::Lengths => {
                "C17 6.5 p2 makes this undefined wherever both are evaluated: a parameter's \
                 declarator is not a full declarator (6.7.6 p3), so nothing orders one \
                 parameter's length against another's"
            }
        };
        diagnostic = diagnostic
            .with_label(Label::secondary(written, "written here"))
            .with_note(note);
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
/// **The message names the pointer where the source wrote it**, `name`, for
/// `memory_finding`'s reason and by its rule: `p[2]`'s temporary and the
/// pointer `**pp` loads out of memory have none, and keep the words that name
/// nothing.
pub(super) fn nullability_finding(
    finding: &nullability::Finding,
    name: Option<&str>,
) -> Option<Diagnostic> {
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
            said(name, "this dereferences a null pointer", |name| {
                format!("this dereferences `{name}`, which is null")
            }),
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
            said(name, "this may dereference a null pointer", |name| {
                format!("this may dereference `{name}` while it is null")
            }),
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
            said(name, "this may dereference a null pointer", |name| {
                format!("this may dereference `{name}` while it is null")
            }),
            "this check cannot say this is not null",
            "read each pointer this goes through into a local, and test that local against null before reading through it",
        ),
        (Asked::Argument { promise }, conclusion) => {
            return passed(finding.at, conclusion, promise, name);
        }
        (
            Asked::Return {
                promise,
                reached_end,
            },
            conclusion,
        ) => return returned(finding.at, conclusion, promise, reached_end, name),
        (Asked::Dereference { through_memory: _ }, Conclusion::Safe) => {
            (NULL_DEREFERENCE, "nothing".to_owned(), "nothing", "nothing")
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
fn passed(
    at: Span,
    conclusion: Conclusion,
    promise: Promise,
    name: Option<&str>,
) -> Option<Diagnostic> {
    let (promised_at, described, promise_label, take_back, level) = match promise {
        Promise::Declared(written) => (
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
            said(
                name,
                &format!("this passes a null pointer to {described}"),
                |name| format!("this passes `{name}`, which is null, to {described}"),
            ),
            "this is null when it is passed",
            format!("pass a pointer to an object here, or {take_back}"),
        ),
        Conclusion::Unknown => (
            said(
                name,
                &format!("this may pass a null pointer to {described}"),
                |name| format!("this may pass `{name}` while it is null to {described}"),
            ),
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
/// concluded, because there is no pointer to test, and writing `_Nullable`
/// would not help: C17 6.9.1 p12 makes the caller's use of the value
/// undefined whatever the declaration says.
fn returned(
    at: Span,
    conclusion: Conclusion,
    promise: Promise,
    reached_end: bool,
    name: Option<&str>,
) -> Option<Diagnostic> {
    let (promised_at, promise_label, take_back, level) = match promise {
        Promise::Declared(written) => (
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
            // Nothing was returned, so nothing is named.
            "this function may reach the end of its body without returning a pointer".to_owned(),
            "the end of this function's body can be reached without a `return`",
            "end every path through the body with a `return`".to_owned(),
        ),
        (false, Conclusion::Unsafe) => (
            said(
                name,
                "this returns a null pointer from a function that promised not to",
                |name| {
                    format!(
                        "this returns `{name}`, which is null, from a function that promised not to"
                    )
                },
            ),
            "this is null when it is returned",
            format!(
                "return a pointer to an object here, or {take_back} in every declaration of the function"
            ),
        ),
        (false, Conclusion::Unknown) => (
            said(
                name,
                "this may return a null pointer from a function that promised not to",
                |name| {
                    format!(
                        "this may return `{name}` while it is null from a function that promised not to"
                    )
                },
            ),
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

/// `byte` or `bytes`, so that one access of one byte does not read as several.
fn bytes(count: u64) -> &'static str {
    if count == 1 { "byte" } else { "bytes" }
}

/// Where a pointer left, as the label at the caret says it.
fn departure(how: Departure) -> &'static str {
    match how {
        Departure::Argument => "passed here",
        Departure::Return => "returned here",
        Departure::Store => "stored here",
    }
}
