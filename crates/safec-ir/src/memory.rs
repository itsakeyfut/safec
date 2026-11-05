//! Whether a program frees one allocation twice, uses one after it was freed,
//! or frees a pointer that is not the start of one.
//!
//! The first checks on [the safety model]'s memory axis, and the first thing
//! this compiler says about what a C program *does* rather than about how it is
//! written.
//!
//! **Five answers out of one walk.** They read one lattice: what a free does to
//! a site is what makes a later use of it a defect, so computing the states
//! twice would be the same computation twice and a second chance for the two
//! copies to disagree. The third asks a free where in its allocation the
//! pointer is, which is ADR-0036, the fourth asks a `return` whether what it
//! hands back was freed, which is ADR-0041, and the fifth asks the same of what
//! a call is handed, which is ADR-0042. [`Kind`] is how the caller tells them
//! apart.
//!
//! **And one about size, out of the same walk.** Whether an access fits the
//! object it reaches reads what each site was made with, `Known::extent`, and
//! what a pointer read out of memory is believed to point at. It is not about
//! a free, so it is asked beside the dereference question rather than through
//! it. See ADR-0060.
//!
//! **This answers a [`Finding`] rather than a diagnostic.** ADR-0011 keeps this
//! crate from seeing one, and what that buys is a check testable against IR
//! built by hand instead of only through the frontend. `safec_llvm`'s `Refusal`
//! is the same shape, and `safec-c` turns one of those into a diagnostic too.
//!
//! **An allocation is named by the local it first landed in.** A call's
//! destination and a parameter are the two places one can arrive from, and both
//! are locals, so a site is numbered as the local is and there is no second
//! numbering to keep in step; it is a type of its own all the same, `Site`
//! beside `LocalIdx` in `memory/index.rs`, so that one cannot be passed for
//! the other. Two calls are two locals because the lowering gives each call
//! its own temporary. A parameter has to be a site as well: without one,
//! `void f(int *p) { free(p); free(p); }` has nothing to mark and the second
//! free is missed in silence, which is the worst thing this compiler can do.
//!
//! **Everything here is about one function.** Nothing reads a callee's body and
//! nothing reads a caller, so `void g(int *a, int *b) { free(a); free(b); }` is
//! silent: `a` and `b` are two sites as far as this can see, and a caller that
//! hands it one pointer twice is a double free this does not find. That is the
//! boundary rather than a defect in it, and what moves the boundary is a
//! summary per function, which nothing here has.
//!
//! **One concern per file.** `memory/parts.rs` holds what the lattice value is
//! made of, `known.rs` the value and the walks over it, `index.rs` the two
//! kinds of index its tables are kept by, and `bits.rs` the sets of them;
//! `built.rs` what a value built out of operands holds; `allocations.rs` the
//! analysis and the facts about a function the walk and the report both read;
//! `transfer.rs` what each element, terminator and edge does to the value,
//! `store.rs` where a write lands, and `init.rs` what a write leaves
//! unwritten; `fit.rs` whether an access fits its object,
//! which the walk and the report both ask; `report.rs` what is asked once the
//! walk has settled; and `assigned.rs` the definite assignment check. This
//! file keeps what a caller names: [`findings`], [`Finding`], [`Kind`] and
//! [`Unproven`]. What an element dereferences and loads is read in
//! `crate::reads`, whose readers of a dereference the nullability check asks
//! too; what a library function's name says is `crate::library`, and which
//! locals are live is `crate::liveness`.
//!
//! [the safety model]: https://github.com/itsakeyfut/safec/blob/main/docs/safety-model.md
//! [`LocalId`]: crate::ir::LocalId

mod allocations;
mod assigned;
mod bits;
mod built;
mod fit;
mod index;
mod init;
mod known;
mod parts;
mod report;
mod runs;
mod store;
mod transfer;

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve_first_turn_within, solve_within};
use crate::ir::{
    Between, BlockId, Element, FuncId, Function, LocalId, Place, Terminator, TranslationUnit, Ty,
};
use crate::nullability::{self};
use crate::reads::{
    dereferenced_in_element, dereferenced_in_terminator, read_out_of_memory_after,
    read_out_of_memory_in_terminator_after,
};
use crate::source::{SourceMap, Span};

use allocations::Allocations;
use built::named;
use index::Table;
use report::{
    after_a_call, copied_away, departed_at_call, departed_in_element,
    freed_after_its_read_was_reported, handed, handed_stored_out, loaded_indeterminate,
    may_not_fit, reached_by_library, reported, returned, stored_away, unwritten_read, used,
    used_before,
};

/// Which of the five things this check answers about a finding is.
///
/// One walk over one lattice, so this is not five checks and
/// `docs/diagnostics.md` says so where it hands them their codes. What differs
/// is the question: the codes and the words are different, and the thing a
/// caret lands on is a call for a double free, an interior free and an
/// argument after free, a dereference for a use after free, and a `return` for
/// a return after free.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `free(p); free(p);`
    DoubleFree,
    /// `free(p); *p = 42;`
    UseAfterFree,
    /// `free(p + 1);`
    ///
    /// Named for what it catches and not for every invalid free: this is a
    /// pointer into an allocation this check followed, and `free(17)` or a
    /// free of a local's address is not reported under it. See ADR-0036.
    InteriorFree,
    /// `free(p); return p;`
    ///
    /// One of the two reads of a pointer without a dereference this check asks
    /// about, because a caller believes what it is handed is live and nothing
    /// reads this function's body from there. See ADR-0041.
    ReturnAfterFree,
    /// `free(p); *out = p;`, where code outside this function may read
    /// `*out`.
    ///
    /// The other door a pointer leaves by: a store into memory a caller,
    /// or code this check cannot read, may read it from, believed live
    /// there as a call's result is. A store into memory only this function
    /// reads is asked where it is read back, not where it is stored, and as
    /// [`Kind::FreedBehindReturn`] or [`Kind::FreedBehindStore`] where that
    /// memory leaves later. See ADR-0041.
    StoreAfterFree,
    /// A free of an allocation whose address may have been stored where code
    /// outside this function may read it: `*out = p; free(p);` (#650).
    ///
    /// [`Kind::StoreAfterFree`] in the other order, asked at the free since
    /// the store was of a live pointer. Always `Unknown`, since whoever reads
    /// that memory may only write it, as for what a door leaves holding
    /// behind it. See `Known::stored_out` and ADR-0041.
    FreedAfterStore,
    /// A call this check cannot read handed an allocation whose address may
    /// have been stored where code outside this function may read it:
    /// `*out = p; release(p);` (#672).
    ///
    /// [`Kind::FreedAfterStore`] where the call, not `free`, may be what frees
    /// it. Always `Unknown`, since the call may keep what it is handed. See
    /// `Known::stored_out` and ADR-0041.
    HandedAfterStore,
    /// `free(x); *b = x; return b;`
    ///
    /// Not [`Kind::ReturnAfterFree`], because what is returned is live: it
    /// is what it points at that holds a freed pointer, which the caller may
    /// read and use. Never a proof, since the caller may only write there, as
    /// [`Kind::FreedBehindArgument`] is not. See ADR-0041 and ADR-0042.
    FreedBehindReturn,
    /// `free(x); *b = x; *out = b;`, where code outside this function may
    /// read `*out`.
    ///
    /// [`Kind::FreedBehindReturn`] at the other door a pointer leaves by,
    /// and never a proof for the same reason. See ADR-0041 and ADR-0042.
    FreedBehindStore,
    /// `free(p); g(p);`
    ///
    /// The other, the same belief arriving by the other door: a function's body
    /// believes its pointer parameters live where it starts, and nothing reads
    /// its callers from there. See ADR-0042.
    ArgumentAfterFree,
    /// `free(a); g(&a);`
    ///
    /// Not [`Kind::ArgumentAfterFree`], because what is handed over is live:
    /// it is what it points at that holds a freed pointer, which the callee
    /// may read and use. Never a proof, since it may only write there. See
    /// ADR-0042.
    FreedBehindArgument,
    /// `g(q, q = &x)`, `*q = (free(q), q = r, 0)`
    ///
    /// Not about what `q` holds, which is why it is always a proof: C17 6.5
    /// p2 makes it undefined whatever that is, because one operand writes `q`
    /// and another reads it with nothing ordering them. The lowering finds it
    /// and [`Element::UnsequencedAccess`] carries it, so this check concludes
    /// it without asking its lattice. See ADR-0052.
    UnsequencedAccess {
        /// Where one operand writes the local, which a label points at.
        written: Span,
        /// Whether the access at the caret is a write too.
        both_write: bool,
        /// Whether the two are a call's arguments or an operator's operands.
        between: Between,
    },
    /// A function past the bound this check reads, `MAX_BYTES`, so it is not
    /// read and nothing about its memory is proved. Always `Unknown`, at the
    /// function's name.
    TooLarge {
        /// About how many bytes its value would take, `blocks * locals *
        /// (locals + LINEAR)`.
        bytes: usize,
        /// The most this check holds.
        max: usize,
    },
    /// A pointer local read where no path to the read assigned it, which C17
    /// 6.3.2.1 p2 leaves undefined whatever the read is for. Always `Unsafe`;
    /// a read some paths assigned is [`Kind::SometimesUnassigned`] or
    /// nothing, and a local whose address is taken is never one. See
    /// `memory::assigned`.
    Unassigned,
    /// A pointer local read where some path assigned it, after a branch edge
    /// that leads to the read on every path and assigns it on none. Always
    /// `Unknown`, since no execution may take the edge. See
    /// `memory::assigned`.
    SometimesUnassigned {
        /// The branch's controlling expression.
        edge: Span,
        /// Whether it is the edge a non-zero condition takes.
        taken: bool,
    },
    /// A pointer read out of an allocation `malloc` or `aligned_alloc` made
    /// and nothing wrote, on every path. Its value is indeterminate (C17
    /// 7.22.3.1 p2, 7.22.3.4 p2), and Annex J.2 lists using it as undefined.
    /// Always `Unsafe`, at the read, whatever it is read for, and not again
    /// at a later use of what it read. `local` is the pointer it is read
    /// through. See `Known::init` and ADR-0057.
    ///
    /// Also out of a local that pointer may be the address of, which nothing
    /// wrote since it was made (C17 6.2.4 p6, 6.7.9 p10, and the same line of
    /// Annex J.2), so each says which storage it reached and the words cite
    /// that storage's clause (#640). See `Known::init`. **`Unknown`
    /// where what it reads may be unwritten**: an element of a local array
    /// after a write that may have been of any element (#639, #667), or a
    /// local written on one path into a join and not the other (#668). See
    /// `Known::init`.
    IndeterminateLoad {
        /// Whether it may reach an allocation nothing wrote.
        reaches_allocation: bool,
        /// Whether it may reach a local nothing wrote.
        reaches_local: bool,
    },
    /// A pointer local whose address is taken, read by name before anything
    /// wrote it, on every path: its value is indeterminate (C17 6.2.4 p6,
    /// 6.7.9 p10), and Annex J.2 lists using it as undefined, the reading
    /// ADR-0057 takes. `Unsafe` at the read, and `Unknown` where the local
    /// was written on one path into a join and not the other (#668). `local`
    /// is the local read. A local whose address is never taken is `Unassigned`'s,
    /// which `memory::assigned` builds, and the two never meet (#640). See
    /// `Known::init` and ADR-0057.
    IndeterminateLocal,
    /// A function this check gave up on part of the way, past `MAX_WORK`, so
    /// nothing about its memory is proved. Always `Unknown`, at the
    /// function's name.
    TooLong {
        /// The walks it was given before it gave up.
        walks: usize,
    },
    /// `int *p = malloc(1); *p = 5;`, `*(p + 5) = 1;`
    ///
    /// An access this check could not show lands inside the object it
    /// reaches. `Unsafe` only where every object it may reach has a size this
    /// check read exactly, it starts at the start of each, and it is wider
    /// than every one of them; `Unknown` wherever less is known. A pointer
    /// moved off its start is never proved outside, since no distance is
    /// carried (ADR-0036). See ADR-0060.
    OutsideItsObject {
        /// How many bytes the access reads or writes, where that was read,
        /// and `0` where it was not: a string's length, or a length that is
        /// not a constant. It is printed only beside a proof, and a proof
        /// needs one.
        width: u64,
        /// How many bytes the object takes, where it is proved outside and
        /// every object it may reach agrees.
        extent: Option<u64>,
    },
    /// `g(p + 5)`, `return p + 5;`, `*box = p + 5;`
    ///
    /// A pointer that leaves this function, which whoever receives it
    /// believes to point at the start of an object as wide as the type it
    /// leaves as, and which this check could not show does. The other half
    /// of what [`Kind::OutsideItsObject`] believes where a pointer arrives.
    ///
    /// **Always `Unknown`, even where the object is shown narrower.** C
    /// leaves nothing undefined where a pointer to one `char` is passed as an
    /// `int *`; it is undefined where the receiver reads or writes through
    /// it, and `Unsafe` says some execution of this function is undefined.
    /// What is shown here is that the receiver's belief is false, so the
    /// words say narrower and the conclusion stays unproven. See ADR-0060.
    HandedOnNarrow {
        /// Where it leaves.
        how: Departure,
        /// How many bytes it is believed to point at where it arrives.
        width: u64,
        /// Whether every object it may reach was read as exactly narrower
        /// than `width`, which the words say.
        narrower: bool,
        /// How many bytes the object takes, where it is shown narrower and
        /// every object it may reach agrees.
        extent: Option<u64>,
        /// Whether the doubt is about what the memory it points at holds,
        /// below its own level, rather than about what it points at itself,
        /// which the words then say (#636).
        below: bool,
    },
}

/// Where a pointer leaves the function it was followed in.
///
/// Three doors, each one where the other side believes what it is handed:
/// a callee believes its parameter, a caller the result, and a later load
/// what was stored. See ADR-0060.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Departure {
    /// Handed to a call as an argument.
    Argument,
    /// Handed back by a `return`.
    Return,
    /// Written through a projection, into memory through a pointer and into
    /// an element of a local array (#621); written by name into a local code
    /// this check cannot read may read (#690); or copied into
    /// memory by `memcpy` or `memmove`. A later read believes what it finds
    /// there, as it believes a load.
    Store,
}

/// About how many bytes one block's value takes for each local, beside the
/// byte or so each pair of locals takes.
///
/// **The value is square in the locals and linear in them too**, and the
/// linear part is the larger until there are a thousand locals: every local
/// has a [`Held`](parts::Held) and a row in each per-local table, each with
/// its own allocation. Fitted to the peak memory of seven functions, from 12
/// locals in 60,011 blocks to 1,402 locals in 801, release build: about one
/// byte a pair and 1,000 a local, within a tenth on all but the smallest.
/// Counting the pairs alone left a function of many blocks and few locals
/// unbounded: one of 102 locals in 12,101 blocks took 1.3 GB at a quarter of
/// that bound, and the bound let the same shape grow four times larger. Found
/// by review.
const LINEAR: usize = 1024;

/// How many bytes each level of a [`Levels`](parts::Levels) a local may
/// carry adds to one block's value: three lists a local, what its value
/// believes of its memory, what was stored in it as a site, and what a call
/// it was handed to may have written there (#636), of one `Option<u64>` a
/// level, which takes 16 bytes. Read off the layout rather
/// than fitted, since [`LINEAR`] was fitted before the lists existed: a
/// function at the parser's 256 `*`s takes about 8 KB a local more than
/// [`LINEAR`] counts, and ran to about twice the memory that bound allows
/// with nothing printed. Found by review (#634).
const PER_LEVEL: usize = 48;

/// How many bytes [`Held::named_at_read`](parts::Held::named_at_read) adds to
/// each local beside what it adds a pair, which `per_block` counts as three
/// tenths of a byte. Fitted, since [`LINEAR`] and the byte a pair were fitted
/// before the field existed: on a loop per write over 100, 140, 150 and 160
/// allocating locals, 404 to 644 locals, release build, peak memory grew by
/// 14 to 17%, from 282 to 322 MB at the smallest and 841 to 978 MB at the
/// largest, and these two terms count 1 to 7% over each of those (#648).
/// Its bit a pair alone, an eighth of a byte, counted under half of it.
const NAMED_AT_READ: usize = 96;

/// How many bytes the per-element record of a local array adds to each
/// local (#667): [`Held::element`](parts::Held::element) and its two flags
/// beside it, and the proved half of a row of `Known::element_init`, one
/// set of bits, with the two flags beside that. Measured as [`NAMED_AT_READ`] was, on the same four
/// functions, release build: peak memory grew by 6.5, 6.1, 5.9 and 5.6%,
/// from 323 to 344 MB at the smallest and 980 to 1,035 MB at the largest,
/// which is about 110 bytes a local at each size. The bits an array keeps
/// are at most one word, beside the row's own 32 bytes. And the doubted
/// half of that row (#668), a second set of bits, measured the same way against
/// `main` after #667: 4.1, 3.4 and 2.9% on the three smaller functions, from
/// 345 to 359 MB at the smallest and 912 to 938 MB at the largest, which
/// the 110 bytes above put at 54 to 69 bytes a local; 72 are counted. The
/// largest of the four is now past the bound on both.
const ELEMENTS: usize = 184;

/// How many bytes a block's cell adds to each local now that it is a
/// [`Held`](parts::Held) rather than the six fields it replaced (#780):
/// `named_at_read` a pair, which [`CELL_PAIRS`] counts, and the rest of a
/// `Held` beside a local. Measured on functions of 60, 80, 100 and 120
/// allocating locals with a loop per write, the shape [`NAMED_AT_READ`] was
/// measured on, release build, against #779's tree: peak memory grew by 20,
/// 20, 21 and 22%, from 270 to 324 MB at the smallest and 1,392 to 1,705 MB
/// at the largest. Fitted as a term square in the locals and one linear in
/// them, the growth is about 31% of the first and 16% of the second, which
/// is these two.
const CELL: usize = 216;

/// The tenths of a byte a pair the cell adds. See [`CELL`].
const CELL_PAIRS: usize = 4;

/// About how many bytes one block's value takes in a function of this many
/// locals, whose deepest pointer type is `depth` `*`s deep.
///
/// Mutation: leave out [`LINEAR`]. Both tests of a function past a bound fail.
/// Mutation: leave out [`PER_LEVEL`].
/// `a_function_whose_lists_are_too_large_is_unknown_and_not_read` fails.
fn per_block(locals: usize, depth: usize) -> usize {
    let per_local = locals
        .saturating_add(locals.saturating_mul(3 + CELL_PAIRS) / 10)
        .saturating_add(LINEAR)
        .saturating_add(CELL)
        .saturating_add(NAMED_AT_READ)
        .saturating_add(ELEMENTS)
        .saturating_add(PER_LEVEL.saturating_mul(depth));
    locals.saturating_mul(per_local)
}

/// How many `*`s deep the deepest pointer type among `function`'s locals
/// is, which is as long as a [`Levels`](parts::Levels) in it gets. One
/// answer for the analysis and for the bound on its size.
fn deepest_pointer(unit: &TranslationUnit, function: &Function) -> usize {
    function
        .locals()
        .map(|local| transfer::pointer_depth(unit, function.local(local)))
        .max()
        .unwrap_or(0)
}

/// How many walks times [`per_block`] this check does in one function.
///
/// A walk copies and compares one block's value, so the work is that product.
/// The value's size is bounded by [`MAX_BYTES`], and the walks are not: a
/// chain of copies in one loop takes a walk per link, so seven blocks of 804
/// locals took 2,800 walks and a minute and a quarter with nothing printed,
/// under a third of the size bound. Between 540 million and a billion a
/// second were measured on four functions once the tables were packed (#479), so
/// this is four to eight seconds.
///
/// A `u64` because the bound is past what a 32-bit `usize` holds.
const MAX_WORK: u64 = 1 << 32;

/// About how many bytes this check holds for one function, about 1.1 GB.
///
/// The value holds one [`per_block`] for every block and every edge, so its
/// memory grows as `blocks` times that, a temporary counting as a local. A
/// loop per write over 150 allocating locals is 1.31 billion and 1.2 GB
/// measured; over 200 it is 2.7 billion and 2.7 GB, with nothing printed
/// while it ran. Past this the function is concluded `Unknown` rather than
/// read, a report rather than a hang.
const MAX_BYTES: usize = 1 << 30;

/// One thing this check concluded, and where.
///
/// Not a diagnostic: this crate cannot see one. What each conclusion costs a
/// build is `Diagnostic::concluded`'s in `safec-c`, which is the one place that
/// answers it, and ADR-0001 is why there is only one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The function it was concluded in.
    ///
    /// What lets a driver tell a conclusion about a hatch from one about the
    /// program. The span alone cannot say, because nothing here maps a span
    /// back to the function whose body holds it. See ADR-0038.
    pub function: FuncId,
    /// Which of the two this is.
    pub kind: Kind,
    /// What that check concluded.
    pub conclusion: Conclusion,
    /// Where a caret goes: the call that frees or is handed a freed pointer, or
    /// the element that reads or writes through one, or returns one.
    ///
    /// Not the place's own span, which a [`Place`] does not have: the element's
    /// or the terminator's.
    pub at: Span,
    /// The local whose own value this conclusion is about, temporary or not.
    ///
    /// `None` where it is about something no local holds: a pointer read out
    /// of memory, at a deeper level of a dereference or as `free(*pp)`'s
    /// argument, and what [`Kind::FreedBehindArgument`] finds behind the
    /// pointer it was handed. Whether the source wrote the local is
    /// [`Function::name_of`]'s to answer and not this check's, so a temporary
    /// is `Some` here and unnamed there.
    ///
    /// **About the caret and nothing else.** `int *q = p; free(q); return *p;`
    /// is about `p`, read through at the caret, while its `freed here` is a
    /// free through `q`: the labels beside it are about other reads, and
    /// nothing here says which local each of those went through.
    ///
    /// [`Function::name_of`]: crate::ir::Function::name_of
    pub local: Option<LocalId>,
    /// The earliest free reaching here, where the check knows which one it was.
    ///
    /// `None` for most `Unknown`s: what usually makes one unknown is that the
    /// paths or the sites reaching here disagree, so there is no single free to
    /// point at. [`Unproven::Unsequenced`] is the exception, where there is one
    /// and what is open is the order.
    pub freed: Option<Span>,
    /// Where the allocation was made, where this check saw it happen.
    ///
    /// `None` for a parameter, whose allocation is a caller's, and for an
    /// `Unknown` for the reason above. `docs/safety-model.md` asks the
    /// diagnostic for this line and it is honest to leave it off rather than
    /// point at an allocation that may not be the one.
    pub made: Option<Span>,
    /// Why this could not be proven, and `None` where it was.
    ///
    /// One reason rather than a flag per reason. The words a reader is given
    /// differ by reason, so this is what chooses them, and two flags beside
    /// each other would have combinations that mean nothing with only a doc
    /// comment to say so. `Some` exactly where [`Self::conclusion`] is
    /// [`Conclusion::Unknown`].
    ///
    /// **That last sentence is held by nothing.** Every `Verdict` is built
    /// in one function and every [`Finding`] out of a `Verdict`, so the two
    /// fields agree by being written together rather than by a rule anything
    /// checks. A mutation cannot show it either: the arm that would answer for
    /// a reason beside a proof is unreachable.
    pub unproven: Option<Unproven>,
}

/// Why this check stopped following a pointer, as far as the producer of
/// [`Unproven::Lost`] can tell, so that a reader is told what to do about
/// it rather than only that something was lost.
///
/// **As far as the producer can tell, and no further.** The one bit a
/// local carries for having lost what it held, `Held::lost`, is set by
/// several causes the lattice does not keep apart, so a pointer lost that
/// way is [`Self::Other`]. Telling them apart would be a set in the
/// lattice, with its own join and height (#213).
///
/// **No span.** None of these producers has one in the lattice, and a
/// variant with a field it could not fill would be a claim nothing
/// established, so the type has nowhere to put one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LostReason {
    /// Read out of memory: an argument read through a projection,
    /// `free(*pp)`, or a pointer a load gave. This check follows locals,
    /// not what memory holds.
    ReadOutOfMemory,
    /// A local reaching no site: this check knows of no allocation it
    /// holds here. That is all the producer establishes, so the words say
    /// that and no cause: the local may never have been given a pointer,
    /// may have been given a constant after holding an allocation, or may
    /// have been written where this check cannot see.
    NoSiteKnown,
    /// What this check could not rule out being a local's address, which
    /// `free` must never be handed (C17 7.22.3.3 p2). Could not rule out
    /// and nothing more: a call this check cannot read is believed able to
    /// return any escaped local's address, so `read_int(&n); p = make();
    /// free(p);` gives it too, and the words say only that.
    MayBeALocal,
    /// Any other cause, `Held::lost` among them, or two reasons at once
    /// that do not include [`Self::MayBeALocal`], which survives any meeting.
    Other,
}

impl LostReason {
    /// One reason for two: itself where they agree, and [`Self::Other`] where
    /// they do not, since naming either would be wrong about the other.
    ///
    /// **Except [`Self::MayBeALocal`], which survives any meeting.** It is the
    /// one reason whose doubt, where it is real, is what C forbids rather than
    /// what this check could not follow, and a path that may hold a local's
    /// address is still a path whatever the pointer may be on another: folded
    /// into `Other`, `r = &x; if (c) r = *t; free(r);` was told nothing says
    /// the program is wrong. Found while implementing #213.
    pub(crate) fn joined(self, other: Self) -> Self {
        match (self, other) {
            (Self::MayBeALocal, _) | (_, Self::MayBeALocal) => Self::MayBeALocal,
            _ if self == other => self,
            _ => Self::Other,
        }
    }
}

/// Why a finding could not be proven.
///
/// **Four reasons and not two, because one of them is about this check and
/// the others are about the program.** A reader told a value may have been
/// freed already is being told something was established somewhere; where this
/// check lost the pointer, nothing was, and saying it anyway is a claim about
/// a program that nobody worked out. Which words each reason gets is
/// `memory_finding`'s in `safec-c`, for the reason [`Finding`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unproven {
    /// The paths or the sites reaching here disagree about a site that really
    /// was freed, or a call this check cannot read was handed one, or could
    /// reach one because it was exposed, and may have freed it. There is a free
    /// to suspect, and no single one to point at. See ADR-0039.
    Disagreement,
    /// This check stopped following the pointer, so nothing here established a
    /// free at all.
    ///
    /// `Reached::Lost` with nothing else contributing, and it takes the same
    /// word as that variant because it is the same fact reaching the reader.
    /// Every producer sits in one of two functions, which is worth writing out
    /// because the ones a reader meets first are not all of them.
    /// `Known::reached_by` answers it for a local that held a site and lost
    /// the name for it, which is ADR-0018 and which a call this check cannot
    /// read also produces, ADR-0029; and for one whose address escaped, which
    /// is ADR-0017. `Allocations::touching` answers it for an argument
    /// written through a projection, `free(*pp)`, and for one whose local
    /// reaches no site at all. None of them says a free happened; each says
    /// this check can no longer say what the pointer points at.
    ///
    /// Those are private, so they are named here rather than linked: a link
    /// out of a public item to one of them is
    /// `rustdoc::private_intra_doc_links`, which this crate denies.
    ///
    /// Each producer says why as far as it can tell, [`LostReason`]: those
    /// on `Known` always say `Other`, and `Allocations::touching`'s are the
    /// reasons it tells apart.
    Lost(LostReason),
    /// C has not said which order runs. See ADR-0022.
    Unsequenced,
    /// A pointer this check followed was moved by something it cannot
    /// evaluate, so whether it is still at the start of its allocation is not
    /// known. Only [`Kind::InteriorFree`] carries it. See ADR-0036.
    Offset,
    /// This check could not show that an access fits the object it reaches:
    /// the pointer may be off its start, or what it reaches has a size this
    /// check did not read or read as smaller than the access. Only
    /// [`Kind::OutsideItsObject`] carries it. See ADR-0060.
    Extent,
}

/// Every double free this unit contains, and every one it cannot rule out.
///
/// One walk per function: the fixpoint answers what holds where each block
/// starts, and this replays each block from there to find the calls to report.
/// The replay rather than a second lattice, because the transfer is what
/// decides which sites a call touches and having two answers to that is having
/// one of them be wrong.
pub fn findings(sources: &SourceMap, unit: &TranslationUnit) -> Vec<Finding> {
    findings_within(sources, unit, MAX_BYTES, MAX_WORK)
}

/// The analysis of one function, as [`findings`] solves it.
///
/// A function of its own so that a test can solve a function it built and
/// read the value where a block starts, which is the one way to see a fact no
/// report shows.
fn analysis<'a>(
    sources: &'a SourceMap,
    unit: &'a TranslationUnit,
    func: FuncId,
) -> Allocations<'a> {
    let function = unit.function(func);
    let mut analysis = Allocations {
        sources,
        unit,
        function: func,
        locals: function.locals().len(),
        deepest_pointer: deepest_pointer(unit, function),
        parameters: function.parameters().collect(),
        exposed_parameters: function
            .parameters()
            .filter(|_| sources.snippet(function.name) != "main")
            // A `match` for `Allocations::is_pointer`'s reason: a kind of
            // type added later is asked whether it is exposed.
            .filter(|&local| match unit.ty(function.local(local)) {
                Ty::Pointer(_) => true,
                // No parameter is an array: C17 6.7.6.3 p7 adjusts it to a
                // pointer.
                Ty::Int | Ty::Char | Ty::Void | Ty::Array { .. } => false,
            })
            .collect(),
        live_in: crate::liveness::live_in(function),
        cfg: Cfg::of(function),
        opaque_results: Table::from_vec(Vec::new()),
        runs_once: Vec::new(),
    };
    analysis.runs_once = vec![false; function.blocks().len()];
    let taken = assigned::address_taken(function);
    for each in analysis.cfg.loops() {
        analysis.runs_once[each.head().index()] =
            runs::shown_to_run(unit, function, &analysis.cfg, &each, &taken);
    }
    analysis.opaque_results = analysis.opaque_results_of(function);
    analysis
}

/// [`findings`], with the bounds passed in, so that a test can ask for them on
/// a function small enough to build.
fn findings_within(
    sources: &SourceMap,
    unit: &TranslationUnit,
    max_bytes: usize,
    max_work: u64,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    for func in unit.functions() {
        let function = unit.function(func);
        // A declaration has no blocks, and `Function::blocks` panics rather
        // than answering for one.
        if !function.is_defined() {
            continue;
        }

        let locals = function.locals().len();
        let depth = deepest_pointer(unit, function);
        let bytes = function
            .blocks()
            .len()
            .saturating_mul(per_block(locals, depth));
        if bytes > max_bytes {
            findings.push(Finding {
                function: func,
                kind: Kind::TooLarge {
                    bytes,
                    max: max_bytes,
                },
                conclusion: Conclusion::Unknown,
                at: function.name,
                local: None,
                freed: None,
                made: None,
                unproven: None,
            });
            continue;
        }

        let analysis = analysis(sources, unit, func);
        let cfg = Cfg::of(function);
        // Never zero, so no clamp: `Function::with_parameters`, which every
        // constructor goes through, makes the local for what the function
        // returns, and locals are only ever added, so `per_block` is at least
        // 1,025. A function with no locals would be a division by zero here,
        // which panics and names itself.
        let walk = u64::try_from(per_block(locals, depth)).unwrap_or(u64::MAX);
        let max_walks = usize::try_from(max_work / walk).unwrap_or(usize::MAX);
        let Some(solution) = solve_within(&analysis, function, &cfg, max_walks) else {
            findings.push(Finding {
                function: func,
                kind: Kind::TooLong { walks: max_walks },
                conclusion: Conclusion::Unknown,
                at: function.name,
                local: None,
                freed: None,
                made: None,
                unproven: None,
            });
            continue;
        };
        // A pointer local read before anything was assigned to it, only in a
        // function the memory check read, since one past its bounds is not
        // read at all. Its own walk is not bounded here: one bit per local
        // only rises, so it settles within `locals + 1` visits a block.
        assigned::findings(unit, sources, func, function, &mut findings);
        // The other check's answer, asked at each block's terminator, which is
        // where a free is reached. A free of a pointer that check established
        // is null frees nothing, and `asked` is the one place that is applied.
        // See ADR-0027.
        let null = nullability::null_at_terminators(sources, unit, function, &cfg);

        // Per function, because a span belongs to one of them. The third
        // field is where in `findings` the report standing at that caret is,
        // so that a proof arriving later can replace a suspicion: `findings`
        // is appended to and assigned into, never removed from or reordered,
        // until the sort at the end of this function.
        let mut said: Vec<(Span, Place, usize)> = Vec::new();
        // The pointers already asked whether an access fits, per caret, kept
        // apart from `said` for the reason `may_not_fit` gives, with where
        // the report standing for each is, as `said` keeps it.
        let mut asked: Vec<(Span, LocalId, usize)> = Vec::new();
        // **And the pointers a library call touched, apart from those.** The
        // same pointer at the same call is two accesses, a dereference in an
        // argument and the call's own reach, and the second may be the
        // wider: keyed together, a doubt about `*p` in `memset(p, *p, 8)`
        // stood where the proof that `memset` writes past `p`'s object
        // should have, and `--allow-unknown` built the overflow. Found by
        // review. See ADR-0060. With where the report standing for each is,
        // as `asked` keeps it, so that a proof at a later argument replaces
        // a doubt at an earlier one over the same pointer.
        let mut touched: Vec<(Span, LocalId, usize)> = Vec::new();
        // And the pointers that left, per caret and door, apart again: a
        // pointer read through and handed on at one call is two questions.
        // With where the report standing for each is, as `said` keeps it.
        let mut departed: Vec<(Span, Departure, Option<LocalId>, usize)> = Vec::new();
        // The locals whose address is taken, which a read by name of asks
        // `Known::init` and `memory::assigned` does not. See #640.
        let taken = assigned::address_taken(function);
        for &id in cfg.order() {
            // `Cfg::order` holds exactly the reachable blocks and `solve` gives
            // every reachable block a value, so nothing skips here today. It is
            // a `continue` rather than a panic because what the `None` says is
            // that no execution reaches this block, and saying nothing about
            // code nothing runs is the right answer to that however it arose.
            let Some(mut known) = solution.value(id).cloned() else {
                continue;
            };

            let block = function.block(id);
            let mut stored = None;
            for element in &block.elements {
                // Before `used`, so that the proof stands at the caret and a
                // doubt about the same place there does not replace it.
                loaded_indeterminate(
                    sources,
                    &mut findings,
                    &mut said,
                    &analysis,
                    function,
                    read_out_of_memory_after(&mut stored, element),
                    &known,
                    false,
                );
                unwritten_read(
                    sources,
                    &mut findings,
                    &mut said,
                    &analysis,
                    assigned::element_reads(element),
                    &taken,
                    &known,
                    false,
                );
                // Before the transfer, which is what the element does: the
                // question is what was true where it runs.
                used(
                    sources,
                    &mut findings,
                    &mut said,
                    dereferenced_in_element(element),
                    &known,
                    func,
                );
                // The same accesses, asked whether they fit. See ADR-0060.
                may_not_fit(
                    &mut findings,
                    &mut asked,
                    &analysis,
                    function,
                    dereferenced_in_element(element),
                    &known,
                );
                // After the dereferences in the element, as `handed` is after
                // the ones at a call, so that `return *tab;` keeps a doubt
                // about reading `*tab` over one about what it returns, as
                // `release(*tab)` does. Before the transfer, for the reason
                // above: what is returned is what was held as the write ran.
                returned(
                    &mut findings,
                    &mut said,
                    &analysis,
                    function,
                    element,
                    &known,
                );
                // And what it stores where code outside this function may
                // read it, the return's other door, asked the same way and at
                // the same point. See ADR-0041.
                stored_away(
                    &mut findings,
                    &mut said,
                    &analysis,
                    function,
                    element,
                    &known,
                );
                // And whether what it returns or stores is whole, which the
                // other side believes. See ADR-0060.
                departed_in_element(
                    &mut findings,
                    &mut departed,
                    &analysis,
                    function,
                    element,
                    &known,
                );
                // Just before what is pending is cleared, so that a free in
                // the same expression has reported first, with its label.
                if matches!(
                    element,
                    Element::Sequenced { .. } | Element::ArgumentsEvaluated { .. }
                ) {
                    after_a_call(sources, &mut findings, &mut said, &known, func);
                }
                // Wherever the walk reaches it, and asking nothing of `known`:
                // the call it stands before is undefined on every execution
                // that evaluates both accesses, and one behind `&&`, `||` or
                // `?:` is counted all the same. See ADR-0052.
                if let Element::UnsequencedAccess {
                    local,
                    written,
                    accessed,
                    both_write,
                    between,
                    origin: _,
                } = *element
                {
                    findings.push(Finding {
                        function: func,
                        kind: Kind::UnsequencedAccess {
                            written,
                            both_write,
                            between,
                        },
                        conclusion: Conclusion::Unsafe,
                        at: accessed,
                        local: Some(local),
                        freed: None,
                        made: None,
                        unproven: None,
                    });
                }
                analysis.element(function, element, &mut known);
            }

            loaded_indeterminate(
                sources,
                &mut findings,
                &mut said,
                &analysis,
                function,
                read_out_of_memory_in_terminator_after(&stored, &block.terminator),
                &known,
                false,
            );
            unwritten_read(
                sources,
                &mut findings,
                &mut said,
                &analysis,
                assigned::terminator_reads(&block.terminator),
                &taken,
                &known,
                false,
            );
            // Before the terminator's own transfer, which is what turns a live
            // allocation into a freed one: the question is what was true when
            // the call was reached.
            used(
                sources,
                &mut findings,
                &mut said,
                dereferenced_in_terminator(&block.terminator),
                &known,
                func,
            );
            may_not_fit(
                &mut findings,
                &mut asked,
                &analysis,
                function,
                dereferenced_in_terminator(&block.terminator),
                &known,
            );
            reached_by_library(
                &mut findings,
                &mut touched,
                &analysis,
                function,
                &block.terminator,
                &known,
            );
            departed_at_call(
                &mut findings,
                &mut departed,
                &analysis,
                function,
                &block.terminator,
                &known,
            );
            // After the dereferences at this call, so a caret carrying both
            // reads `SC0402` above `SC0407`, and before the transfer, for the
            // reason above.
            // A library copy asked as the store it is, before what it is
            // handed, which then asks nothing of a source it proved. See
            // `report::copied_away`.
            let copied = copied_away(
                &mut findings,
                &mut said,
                &analysis,
                &block.terminator,
                &known,
            );
            handed(
                &mut findings,
                &mut said,
                &analysis,
                function,
                &block.terminator,
                &known,
                copied.as_ref(),
            );
            // And whether it is handed what was stored out, before the
            // transfer marks it as what the call may have freed. See
            // `report::handed_stored_out`.
            handed_stored_out(
                &mut findings,
                &mut said,
                &analysis,
                function,
                &block.terminator,
                &known,
            );
            // **Not a doubt about freeing a pointer already reported where it
            // was read out of fresh memory**: the two would be one defect
            // reported twice, the second time as unknown. Only a doubt, and
            // only about the local freed. See ADR-0057.
            let reported_at_its_read =
                freed_after_its_read_was_reported(&analysis, &block.terminator, &known);
            findings.extend(
                reported(&analysis, &block.terminator, &known, &null, id)
                    .into_iter()
                    .filter(|finding| {
                        finding.conclusion == Conclusion::Unsafe
                            || reported_at_its_read.is_none()
                            || finding.local != reported_at_its_read
                    }),
            );
            // After the free's own finding, which is the one whose caret is
            // here: what this adds are reports about carets further back, and a
            // reader meets them in the order the sort at the end puts them in
            // rather than the order they were made. Still before the transfer,
            // because what it asks about is what had been read when the call
            // was reached.
            used_before(
                &mut findings,
                &mut said,
                &analysis,
                &block.terminator,
                &known,
                &null,
                id,
            );
            if matches!(block.terminator, Terminator::Return) {
                after_a_call(sources, &mut findings, &mut said, &known, func);
            }
        }

        // **And every read inside a loop, as of that loop's first turn**
        // (#670): whatever turn reaches a read, the first ran before it, and
        // what it sees is what arrived from before the loop. The join at the
        // head of a loop shown to run creates no doubt along a back edge, so
        // such a loop that fills an array is not doubted for a path that
        // skips it (#668, ADR-0064), and with nothing else every read the
        // head reached before a write went silent, `r = m[0]; m[0] = &x;` in
        // the body among them. Per loop
        // rather than every back edge at once, which doubted a local made in
        // an outer loop and filled by an inner one wherever it was read after
        // the inner loop. After the every-turn walk above, so that a proof it
        // gave stands at the caret, and a doubt it gave gives way to a proof
        // here: asked first, a first turn's doubt took the place of four such
        // proofs, measured. See ADR-0057.
        //
        // **One loop at a time, its solution dropped before the next, and
        // every first turn together within one more bound**, by the user's
        // decision: each solved within the whole bound and all kept to the
        // end, a nest of thirty loops took twenty-five times as long as
        // `main` and sixty loops in sequence held 5.4 GB, with nothing
        // printed. Found by review. Past it, the bound's doubt.
        let dominators = cfg.dominators();
        let mut left = max_walks;
        for each in cfg.loops() {
            let Some(solution) =
                solve_first_turn_within(&analysis, function, &cfg, each.head(), left)
            else {
                findings.push(Finding {
                    function: func,
                    kind: Kind::TooLong { walks: max_walks },
                    conclusion: Conclusion::Unknown,
                    at: function.name,
                    local: None,
                    freed: None,
                    made: None,
                    unproven: None,
                });
                break;
            };
            left -= solution.walks();
            // **And every block that leaves the loop from inside its body**,
            // one a block of the body other than the head dominates: what runs
            // there on the first turn is what arrived from before the loop
            // too. The every-turn state of a loop shown to run creates no doubt
            // at its back edge, and without this `if (c()) return *m[0] == 0;`
            // before the body's write went silent in a loop that runs. A
            // doubt, since it may not run on the first turn. See ADR-0064.
            //
            // **The nearest dominator in the loop, per block, in one pass**:
            // the head dominates every block of the body, so some block of the
            // body other than the head dominates a block exactly where the
            // nearest one of its dominators in the loop is not the head.
            // `Cfg::order` puts a block's immediate dominator before it. Asked
            // of every block of the body by `Dominators::dominates` instead,
            // this was a square in the blocks per loop.
            let mut nearest_inside: Vec<Option<BlockId>> = vec![None; function.blocks().len()];
            for &id in cfg.order() {
                if let Some(up) = dominators.immediate(id) {
                    nearest_inside[id.index()] = if each.contains(up) {
                        Some(up)
                    } else {
                        nearest_inside[up.index()]
                    };
                }
            }
            for &id in cfg.order() {
                let leaves_early = !each.contains(id)
                    && nearest_inside[id.index()].is_some_and(|inside| inside != each.head());
                if !each.contains(id) && !leaves_early {
                    continue;
                }
                let Some(mut turn) = solution.value(id).cloned() else {
                    continue;
                };
                // **A proof only where the block dominates every block the
                // loop's back edges leave from**, so that every turn that
                // comes round passed it and the first did too. A read on an
                // arm, `if (k == 1) q = *t2;`, may not run on the first turn,
                // and C defines it reading last turn's value, so it is a
                // doubt, by the user's decision. Every latch and not any: no
                // case tells the two apart, measured, since a loop this C can
                // write has one, and `continue` is the first that will not.
                let doubt_only = leaves_early
                    || !each
                        .latches()
                        .iter()
                        .all(|&latch| dominators.dominates(id, latch));
                let block = function.block(id);
                let mut stored = None;
                for element in &block.elements {
                    loaded_indeterminate(
                        sources,
                        &mut findings,
                        &mut said,
                        &analysis,
                        function,
                        read_out_of_memory_after(&mut stored, element),
                        &turn,
                        doubt_only,
                    );
                    unwritten_read(
                        sources,
                        &mut findings,
                        &mut said,
                        &analysis,
                        assigned::element_reads(element),
                        &taken,
                        &turn,
                        doubt_only,
                    );
                    analysis.element(function, element, &mut turn);
                }
                loaded_indeterminate(
                    sources,
                    &mut findings,
                    &mut said,
                    &analysis,
                    function,
                    read_out_of_memory_in_terminator_after(&stored, &block.terminator),
                    &turn,
                    doubt_only,
                );
                unwritten_read(
                    sources,
                    &mut findings,
                    &mut said,
                    &analysis,
                    assigned::terminator_reads(&block.terminator),
                    &taken,
                    &turn,
                    doubt_only,
                );
            }
        }
    }

    // In the order a reader's eye goes rather than the order the walk reached
    // them. `Cfg::of` hands blocks back in reverse postorder, so two findings
    // in two arms of one branch come out with the later line first, and
    // `DiagnosticSink` does not sort. Doing it here rather than there because
    // the sink holds diagnostics from every stage and their order is the order
    // the stages ran, which is right; this is one stage disagreeing with
    // itself. `Terminator::successors` already says its order will change when
    // an unwinding call gains an edge, and this is what keeps that from moving
    // every expectation that holds two findings.
    // By where the code is, so a finding in an included file sits where its
    // `#include` is rather than after every finding in the file including it.
    findings.sort_by_key(|finding| sources.position(finding.at));

    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{
        Block, BlockId, Function, Operand, Operation, Origin, Projection, Rvalue, Terminator, Ty,
    };
    use crate::target::Target;

    /// A unit holding one function of ten locals and three blocks: 37,350 by
    /// [`per_block`].
    fn ten_by_three() -> (SourceMap, TranslationUnit) {
        ten_by_three_deep(0)
    }

    /// The same, with its last local a pointer `depth` `*`s deep, which
    /// [`per_block`] counts `PER_LEVEL` a level a local for.
    fn ten_by_three_deep(depth: usize) -> (SourceMap, TranslationUnit) {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "f");
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(Span::new(file, 0, 1), int, []);
        // Eight, beside the one `Function::new` makes for what it returns
        // and the last, below.
        for _ in 0..8 {
            function.push_local(int);
        }
        let mut deep = int;
        for _ in 0..depth {
            deep = unit.push_type(Ty::Pointer(deep));
        }
        function.push_local(deep);
        let blocks: Vec<_> = (0..3).map(|_| function.reserve_block()).collect();
        for pair in blocks.windows(2) {
            function.fill_block(
                pair[0],
                Block {
                    elements: vec![],
                    terminator: Terminator::Goto(pair[1]),
                },
            );
        }
        function.fill_block(
            blocks[2],
            Block {
                elements: vec![],
                terminator: Terminator::Return,
            },
        );
        unit.push_function(function);
        (sources, unit)
    }

    /// Past the bound, a function is one `Unknown` finding at its name and
    /// is not read. Ten locals are `10 * (10 + 7 + LINEAR + NAMED_AT_READ +
    /// ELEMENTS + CELL)` a block, so three blocks are 46,110.
    ///
    /// Mutation: drop the guard. The function is solved, and no `TooLarge`
    /// finding is made. Mutation: leave out [`NAMED_AT_READ`], [`ELEMENTS`]
    /// or [`CELL`], or the seven tenths of a byte a pair. This fails on
    /// `bytes`.
    #[test]
    fn a_function_past_the_bound_is_one_unknown_finding() {
        let (sources, unit) = ten_by_three();
        let findings = findings_within(&sources, &unit, 46_109, MAX_WORK);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(
            findings[0].kind,
            Kind::TooLarge {
                bytes: 46_110,
                max: 46_109
            }
        );
        assert_eq!(findings[0].conclusion, Conclusion::Unknown);
        assert_eq!(sources.snippet(findings[0].at), "f");
    }

    /// At the bound, a function is read.
    ///
    /// Mutation: refuse at the bound rather than past it. This is refused.
    #[test]
    fn a_function_at_the_bound_is_read() {
        let (sources, unit) = ten_by_three();
        let findings = findings_within(&sources, &unit, 46_110, MAX_WORK);
        assert!(
            findings
                .iter()
                .all(|finding| !matches!(finding.kind, Kind::TooLarge { .. })),
            "{findings:?}"
        );
    }

    /// The lists a local's levels take count toward the bound: ten locals,
    /// one of them three `*`s deep, are
    /// `10 * (10 + 7 + LINEAR + NAMED_AT_READ + ELEMENTS + CELL + 3 *
    /// PER_LEVEL)` a block, so three blocks are 50,430, past a bound the same function with no
    /// pointer in it is read under.
    ///
    /// Mutation: leave out [`PER_LEVEL`]. This is read.
    #[test]
    fn a_function_whose_lists_are_too_large_is_unknown_and_not_read() {
        let (sources, unit) = ten_by_three_deep(3);
        let findings = findings_within(&sources, &unit, 50_429, MAX_WORK);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(
            findings[0].kind,
            Kind::TooLarge {
                bytes: 50_430,
                max: 50_429
            }
        );
    }

    /// The bounds, written out, so that moving one is a decision a test sees.
    ///
    /// Mutation: change either constant. This fails.
    #[test]
    fn the_bounds_are_two_to_the_thirtieth_and_thirty_second_over_1024_a_local() {
        assert_eq!(LINEAR, 1024);
        assert_eq!(PER_LEVEL, 48);
        assert_eq!(NAMED_AT_READ, 96);
        assert_eq!(ELEMENTS, 184);
        assert_eq!(CELL, 216);
        assert_eq!(CELL_PAIRS, 4);
        assert_eq!(MAX_BYTES, 1_073_741_824);
        assert_eq!(MAX_WORK, 4_294_967_296);
    }

    /// Past the work bound, a function is one `Unknown` finding at its name.
    /// Ten locals are 15,370 a walk, so a bound of 46,109 is two walks, and
    /// the three blocks take three.
    ///
    /// Mutation: solve without the bound. No `TooLong` finding is made.
    #[test]
    fn a_function_past_the_work_bound_is_one_unknown_finding() {
        let (sources, unit) = ten_by_three();
        let findings = findings_within(&sources, &unit, MAX_BYTES, 46_109);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].kind, Kind::TooLong { walks: 2 });
        assert_eq!(findings[0].conclusion, Conclusion::Unknown);
        assert_eq!(sources.snippet(findings[0].at), "f");
    }

    /// Within the work bound, with its first turns past one more of it, a
    /// function is the same `Unknown` finding: two loops in sequence solve
    /// as of each one's first turn, each walking the whole function, so the
    /// two together take more walks than the every-turn solve the bound was
    /// set to (#670).
    ///
    /// Mutation: leave a first turn past the bound unreported. Nothing is
    /// found, and this fails.
    #[test]
    fn a_function_whose_first_turns_pass_one_more_bound_is_one_unknown_finding() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "f");
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(Span::new(file, 0, 1), int, []);
        let at = Span::new(file, 0, 1);
        let entry = function.reserve_block();
        let first = function.reserve_block();
        let first_body = function.reserve_block();
        let second = function.reserve_block();
        let second_body = function.reserve_block();
        let exit = function.reserve_block();
        let go = |to| Block {
            elements: vec![],
            terminator: Terminator::Goto(to),
        };
        let branch = |then, otherwise| Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Constant(1),
                then,
                otherwise,
                origin: Origin::Written(at),
            },
        };
        function.fill_block(entry, go(first));
        function.fill_block(first, branch(first_body, second));
        function.fill_block(first_body, go(first));
        function.fill_block(second, branch(second_body, exit));
        function.fill_block(second_body, go(second));
        function.fill_block(
            exit,
            Block {
                elements: vec![],
                terminator: Terminator::Return,
            },
        );
        let locals = function.locals().len();
        let func = unit.push_function(function);
        let walks = crate::dataflow::solve(
            &analysis(&sources, &unit, func),
            unit.function(func),
            &Cfg::of(unit.function(func)),
        )
        .walks();
        let findings = findings_within(
            &sources,
            &unit,
            MAX_BYTES,
            u64::try_from(walks * per_block(locals, 0)).expect("small"),
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].kind, Kind::TooLong { walks });
        assert_eq!(findings[0].conclusion, Conclusion::Unknown);
    }

    /// At the work bound, three walks for three, the function is read.
    ///
    /// Mutation: give up at the bound rather than past it. This is refused.
    #[test]
    fn a_function_at_the_work_bound_is_read() {
        let (sources, unit) = ten_by_three();
        let findings = findings_within(&sources, &unit, MAX_BYTES, 46_110);
        assert!(findings.is_empty(), "{findings:?}");
    }
    /// Where a local's address stored in memory can reach, as a test builds
    /// it: `t = malloc(); *t = &x;`, then `then` from the block after.
    ///
    /// No report shows these facts: every local whose address is stored has
    /// escaped, and this check doubts what such a local holds whatever is
    /// written into it, so a program printed the same with each of them
    /// dropped. So these tests read the value where a block starts instead.
    struct Stored {
        sources: SourceMap,
        unit: TranslationUnit,
        function: Function,
        at: Span,
        c: LocalId,
        t: LocalId,
        x: LocalId,
        u: LocalId,
        /// What `&x` is put in before it is stored, as the lowering does.
        address: LocalId,
        malloc: FuncId,
        realloc: FuncId,
        memcpy: FuncId,
    }

    impl Stored {
        fn new() -> Self {
            let mut sources = SourceMap::new();
            let file = sources.add_virtual("t.c", "f malloc realloc memcpy");
            let at = Span::new(file, 0, 1);
            let mut unit = TranslationUnit::new(
                Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
            );
            let int = unit.push_type(Ty::Int);
            let void = unit.push_type(Ty::Void);
            let ptr = unit.push_type(Ty::Pointer(int));
            let ptr_ptr = unit.push_type(Ty::Pointer(ptr));
            let any = unit.push_type(Ty::Pointer(void));
            let malloc =
                unit.push_function(Function::declaration(Span::new(file, 2, 8), any, [int]));
            let realloc = unit.push_function(Function::declaration(
                Span::new(file, 9, 16),
                any,
                [any, int],
            ));
            let memcpy = unit.push_function(Function::declaration(
                Span::new(file, 17, 23),
                any,
                [any, any, int],
            ));
            let mut function = Function::new(at, int, [int]);
            let c = function.parameters().next().expect("the one parameter");
            let t = function.push_local(ptr_ptr);
            let x = function.push_local(ptr);
            let u = function.push_local(ptr_ptr);
            let address = function.push_local(ptr_ptr);
            Stored {
                sources,
                unit,
                function,
                at,
                c,
                t,
                x,
                u,
                address,
                malloc,
                realloc,
                memcpy,
            }
        }

        fn call(
            &self,
            callee: FuncId,
            arguments: Vec<Operand>,
            into: Option<LocalId>,
            then: BlockId,
        ) -> Block {
            Block {
                elements: vec![],
                terminator: Terminator::Call {
                    callee,
                    arguments,
                    destination: into.map(Place::local),
                    then: Some(then),
                    origin: Origin::Written(self.at),
                },
            }
        }

        /// `*t = &x;`, falling through to `then`.
        fn store(&self, then: BlockId) -> Block {
            Block {
                elements: vec![
                    Element::Assign(Operation {
                        place: Place::local(self.address),
                        value: Rvalue::Address(Place::local(self.x)),
                        origin: Origin::Written(self.at),
                    }),
                    Element::Assign(Operation {
                        place: Place {
                            local: self.t,
                            projection: vec![Projection::Deref],
                        },
                        value: Rvalue::Use(Operand::Copy(Place::local(self.address))),
                        origin: Origin::Written(self.at),
                    }),
                ],
                terminator: Terminator::Goto(then),
            }
        }

        fn goto(then: BlockId) -> Block {
            Block {
                elements: vec![],
                terminator: Terminator::Goto(then),
            }
        }

        /// Whether `x`'s address may be in what `container` points at, where
        /// `at` starts.
        fn holds_the_address(mut self, at: BlockId, container: LocalId) -> bool {
            self.function.fill_block(
                at,
                Block {
                    elements: vec![],
                    terminator: Terminator::Return,
                },
            );
            let func = self.unit.push_function(self.function);
            let analysis = analysis(&self.sources, &self.unit, func);
            let function = self.unit.function(func);
            let solution = crate::dataflow::solve(&analysis, function, &Cfg::of(function));
            let known = solution.value(at).expect("the last block is reached");
            known.contents[index::Site::of(index::LocalIdx::of(container))]
                .names_local(index::LocalIdx::of(self.x))
        }
    }

    /// A local's address stored on either arm is there where the arms meet.
    ///
    /// Mutation: have the join leave the locals in a cell's `names` as the first arm to
    /// arrive had it. One of the two fails, whichever arm the walk takes first.
    #[test]
    fn a_locals_address_stored_on_either_arm_is_in_memory_where_they_meet() {
        for store_on_then in [true, false] {
            let mut built = Stored::new();
            let blocks: Vec<_> = (0..5).map(|_| built.function.reserve_block()).collect();
            let block = built.call(
                built.malloc,
                vec![Operand::Constant(8)],
                Some(built.t),
                blocks[1],
            );
            built.function.fill_block(blocks[0], block);
            built.function.fill_block(
                blocks[1],
                Block {
                    elements: vec![],
                    terminator: Terminator::Branch {
                        condition: Operand::Copy(Place::local(built.c)),
                        then: blocks[2],
                        otherwise: blocks[3],
                        origin: Origin::Written(built.at),
                    },
                },
            );
            let (stores, skips) = if store_on_then {
                (blocks[2], blocks[3])
            } else {
                (blocks[3], blocks[2])
            };
            let block = built.store(blocks[4]);
            built.function.fill_block(stores, block);
            built.function.fill_block(skips, Stored::goto(blocks[4]));
            let t = built.t;
            assert!(
                built.holds_the_address(blocks[4], t),
                "stored on the then arm: {store_on_then}"
            );
        }
    }

    /// What a `realloc` returns may hold every local's address the old
    /// allocation held.
    ///
    /// Mutation: drop the union into the locals in the new site's cell's `names`, or copy
    /// the old site's cell into it instead. This fails.
    #[test]
    fn a_locals_address_stored_in_an_allocation_is_in_what_realloc_returns() {
        let mut built = Stored::new();
        let blocks: Vec<_> = (0..4).map(|_| built.function.reserve_block()).collect();
        let block = built.call(
            built.malloc,
            vec![Operand::Constant(8)],
            Some(built.t),
            blocks[1],
        );
        built.function.fill_block(blocks[0], block);
        let block = built.store(blocks[2]);
        built.function.fill_block(blocks[1], block);
        let arguments = vec![Operand::Copy(Place::local(built.t)), Operand::Constant(16)];
        let block = built.call(built.realloc, arguments, Some(built.u), blocks[3]);
        built.function.fill_block(blocks[2], block);
        let u = built.u;
        assert!(built.holds_the_address(blocks[3], u));
    }

    /// What `memcpy` copies into may hold every local's address the source
    /// held.
    ///
    /// Mutation: have the copy leave the locals in the destination's cell alone.
    /// This fails.
    #[test]
    fn a_locals_address_stored_in_an_allocation_is_in_what_memcpy_copies_it_into() {
        let mut built = Stored::new();
        let blocks: Vec<_> = (0..5).map(|_| built.function.reserve_block()).collect();
        let block = built.call(
            built.malloc,
            vec![Operand::Constant(8)],
            Some(built.t),
            blocks[1],
        );
        built.function.fill_block(blocks[0], block);
        let block = built.store(blocks[2]);
        built.function.fill_block(blocks[1], block);
        let block = built.call(
            built.malloc,
            vec![Operand::Constant(8)],
            Some(built.u),
            blocks[3],
        );
        built.function.fill_block(blocks[2], block);
        let arguments = vec![
            Operand::Copy(Place::local(built.u)),
            Operand::Copy(Place::local(built.t)),
            Operand::Constant(8),
        ];
        let block = built.call(built.memcpy, arguments, None, blocks[4]);
        built.function.fill_block(blocks[3], block);
        let u = built.u;
        assert!(built.holds_the_address(blocks[4], u));
    }
}
