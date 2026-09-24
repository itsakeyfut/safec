//! Whether a program frees one allocation twice, uses one after it was freed,
//! or frees a pointer that is not the start of one.
//!
//! The first checks on [the safety model]'s memory axis, and the first thing
//! this compiler says about what a C program *does* rather than about how it is
//! written.
//!
//! **Three answers out of one walk.** They read one lattice: what a free does to
//! a site is what makes a later use of it a defect, so computing the states
//! twice would be the same computation twice and a second chance for the two
//! copies to disagree. The third asks a free where in its allocation the
//! pointer is, which is ADR-0036. [`Kind`] is how the caller tells them apart.
//!
//! **This answers a [`Finding`] rather than a diagnostic.** ADR-0011 keeps this
//! crate from seeing one, and what that buys is a check testable against IR
//! built by hand instead of only through the frontend. `safec_llvm`'s `Refusal`
//! is the same shape, and `safec` turns one of those into a diagnostic too.
//!
//! **An allocation is named by the local it first landed in.** A call's
//! destination and a parameter are the two places one can arrive from, and both
//! are locals, so a site is a [`LocalId`] and there is no second numbering to
//! keep in step. Two calls are two locals because the lowering gives each call
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
//! [the safety model]: https://github.com/itsakeyfut/safec/blob/main/docs/safety-model.md

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve};
use crate::ir::{
    BinOp, BlockId, Element, FuncId, Function, LocalId, Operand, Place, Projection, Rvalue,
    Terminator, TranslationUnit, Ty,
};
use crate::nullability::{self, NullAtTerminators};
use crate::source::{SourceMap, Span};

/// What this check can read in a callee's name.
///
/// By name because nothing else is available: an annotation saying what a
/// function does to what it is passed is the phase's last issue and does not
/// exist. C17 7.1.3 reserves the identifiers the library declares, so a program
/// that defines its own `free` has no behaviour C defines. `clang -std=c17
/// -pedantic-errors` does not diagnose one, measured, so a program that does it
/// anyway is read wrongly here and there is no way to tell from inside.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Callee {
    /// C17 7.22.3.3's `free`.
    Frees,
    /// C17 7.22.3.4's `malloc`. Read only so that its arguments are left
    /// alone: it is otherwise an ordinary call, and an allocation is named by
    /// where it landed rather than by which function made it.
    Allocates,
    /// Anything else. It may free what it was passed and this cannot tell.
    Opaque,
}

/// A free, and whether C has ordered it before what comes after it.
///
/// **The two halves travel together or the second is forgotten.** A span alone
/// was what this carried, and the check read the order the lowering emitted as
/// though C had chosen it: `int x = *p + (free(p), 0);` was a proved use after
/// free, and so was its mirror, because ADR-0010 makes a call end a block and
/// the rest of the expression lands in the next one whichever side it is
/// written on. See ADR-0022.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Freeing {
    /// Where the free is.
    at: Span,
    /// Whether an [`Element::Sequenced`] has passed since.
    ///
    /// False until one has. C17 6.5 p3 leaves the rest of the full expression
    /// unsequenced with the call, so which happens first is C's to choose and
    /// nothing here is yet a proof.
    sequenced: bool,
}

impl Freeing {
    /// A free nothing has sequenced yet.
    fn new(at: Span) -> Self {
        Freeing {
            at,
            sequenced: false,
        }
    }

    /// The two together, where two paths meet.
    ///
    /// The earlier span, because that is which free the diagnostic names, and
    /// the **conjunction** of the flags, because a path that reached here with
    /// the order still open is a path on which this is not a proof.
    fn joined(self, other: Self) -> Self {
        Freeing {
            at: earlier(self.at, other.at),
            sequenced: self.sequenced && other.sequenced,
        }
    }
}

/// What is known about the allocation one site stands for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SiteState {
    /// Not freed on any path that reaches here, and where it came from.
    ///
    /// `None` where this check did not see the allocation happen: a parameter,
    /// whose allocation is a caller's, and every local nothing has allocated
    /// into. The diagnostic leaves its `allocated here` label off rather than
    /// pointing somewhere it guessed.
    Live(Option<Span>),
    /// Freed, and where the earliest free reaching here is.
    ///
    /// Earliest by position rather than by which path arrived first.
    ///
    /// ADR-0016 records that a value carrying a span can fail to reach a
    /// fixpoint, because a join keeping whichever one arrived oscillates where
    /// two of them meet below a branch inside a loop, and that it was measured
    /// as a hang. **That is not what this rule is doing here**, and borrowing
    /// the record's reason would be a claim nothing holds: `Unknown` is a top
    /// per site, so a span that would have alternated is absorbed before it
    /// can. Measured, on this lattice: keeping whichever arrived still ends
    /// every walk, and the loop-shaped test built to look for the hang passes
    /// under it.
    ///
    /// What the rule is for is **which free the diagnostic names**. Without it
    /// the answer is whichever path the worklist reached last, which is stable
    /// for one program and arbitrary between two that differ only in the order
    /// their blocks were built. `a_join_names_the_earlier_free` is the guard.
    Freed {
        /// Where the allocation came from, on the same terms as [`Self::Live`].
        made: Option<Span>,
        /// The earliest free reaching here, and whether C has sequenced it.
        freed: Freeing,
    },
    /// Freed on one path and not on another, or handed to a call this check
    /// cannot read.
    Unknown,
}

impl SiteState {
    /// The two together, which is `Unknown` unless they agree.
    fn joined(self, other: Self) -> Self {
        match (self, other) {
            (Self::Live(here), Self::Live(there)) => Self::Live(same(here, there)),
            (
                Self::Freed {
                    made: here,
                    freed: from_here,
                },
                Self::Freed {
                    made: there,
                    freed: from_there,
                },
            ) => Self::Freed {
                made: same(here, there),
                freed: from_here.joined(from_there),
            },
            _ => Self::Unknown,
        }
    }
}

/// Where in the allocations it holds a local's value points.
///
/// **Three values, because this check can be certain in both directions.**
/// C17 7.22.3.3 p2 makes a `free` of anything but the pointer an allocation
/// function returned undefined, and `p + 1` reaches the same allocation `p`
/// does, so which allocation a value reaches cannot say whether freeing it is
/// defined. A constant offset can, and one this check cannot evaluate cannot.
/// See ADR-0036.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Offset {
    /// The start of every site this local holds, and what a local holding no
    /// site answers.
    Zero,
    /// A non-zero distance from the start of every site it holds.
    NonZero,
    /// This check cannot say which.
    Unknown,
}

impl Offset {
    /// The two together, which is `Unknown` unless they agree.
    ///
    /// **`NonZero` survives a join, and that is not slack.** Two paths that
    /// each moved the pointer off the start both arrive off the start, whatever
    /// the two distances were, so `if (c) q = p + 1; else q = p + 2; free(q);`
    /// stays a proof.
    ///
    /// **`Zero` is not an identity here**, although it is what a local holding
    /// no site answers. `int *q = 0; if (c) q = p + 1; free(q);` joins a path
    /// holding nothing with one off the start and answers `Unknown`, which is a
    /// warning on a program that frees null on one path and an interior
    /// pointer on the other. That is row 4 rather than row 6, and ADR-0036
    /// records it as what three values cost.
    fn joined(self, other: Self) -> Self {
        if self == other { self } else { Self::Unknown }
    }
}

/// Where an allocation came from, where two paths agree about it.
///
/// `None` where they do not, which only ever loses what was known and so cannot
/// cycle. Two paths reaching one site with two different allocations is not a
/// shape the frontend produces, because a site is the local a call writes into
/// and each call has its own; the type allows it and so this answers for it
/// rather than picking one and being wrong on the day something else does.
fn same(here: Option<Span>, there: Option<Span>) -> Option<Span> {
    match (here, there) {
        (Some(here), Some(there)) if here == there => Some(here),
        _ => None,
    }
}

/// The earlier of two spans, by where they are rather than by which arrived.
///
/// A total order over the pair, so a value carrying one can only move one way
/// and the walk ends. RK-007 in the review knowledge bank is why the file is
/// the first half: a span names its own file and two of them need not share
/// one.
fn earlier(here: Span, there: Span) -> Span {
    if (here.file().index(), here.start()) <= (there.file().index(), there.start()) {
        here
    } else {
        there
    }
}

/// What one read is filed under. See [`ReadKey`].
fn read_key(at: Span, place: &Place) -> ReadKey {
    (at.file().index(), at.start(), at.end(), place.clone())
}

/// What one argument of a call reaches.
///
/// The two are not the same answer and were once the same silence. A `free`
/// whose argument reaches no site used to be indistinguishable from one whose
/// sites were all proved live, and the check reported nothing for both. The
/// second is a proof; the first is this check having lost the pointer, and
/// saying nothing about it is [the safety model]'s worst failure rather than
/// its best one.
///
/// [the safety model]: https://github.com/itsakeyfut/safec/blob/main/docs/safety-model.md
enum Reached {
    /// A site the argument may hold.
    Site(usize),
    /// The set this local names had one member freed, and which one is not
    /// known. Whether C has sequenced that free travels with it, for the reason
    /// [`Freeing`] gives.
    ///
    /// **A proof, not a shrug.** [`Reached::Lost`] beside it is this check
    /// having given up; this is something it worked out and can act on: freeing
    /// the same local again frees the same member, whichever member that was.
    /// The two are separate variants for the reason the enum exists at all.
    /// See ADR-0020.
    SetFreed(Freeing),
    /// A pointer this check was not following: one written through a
    /// projection, or a local whose sites it had and lost.
    ///
    /// The second half was prose until ADR-0018 gave it a producer. A site is
    /// named by a local, so a loop that allocates every turn hands one site to
    /// one allocation after another, and whoever still held the last one is
    /// holding something this check can no longer name. ADR-0029 gave it a
    /// second: a call this check cannot read may write through an address that
    /// escaped, and what it leaves behind has no site either. ADR-0031 gave it
    /// a third, which is that same write performed here rather than by a
    /// callee.
    Lost,
}

/// What one local may hold.
///
/// A struct rather than the bit vector this was, because the two halves travel
/// together everywhere: a copy writes both, arithmetic unions both, and
/// anything else clears both. Kept apart they have to be kept in step by hand
/// in every arm of [`Allocations::element`], and forgetting one is a silence
/// rather than a build error. See ADR-0018.
///
/// **What lives here is what an assignment destroys.** Giving a local a fresh
/// value replaces everything in this struct, which is why [`Held::clear`] can
/// answer for a field without being told what it means. A fact that outlives
/// an assignment does not belong here however much it looks like one:
/// [`Known::escaped`] is per local and stays on [`Known`] for exactly that
/// reason, and putting it here would answer `false` after `p = q;` and undo
/// #155.
#[derive(Clone, PartialEq, Eq)]
struct Held {
    /// A bit per site. A site is a local, so this is square in the locals.
    ///
    /// `Vec<bool>` rather than a packed bitset: this crate takes no
    /// dependencies, and a byte per pair is what an ordinary function costs.
    ///
    /// **It is square in the locals and there are two of these**, so the value
    /// is `blocks * locals * (2 * locals + 72)` bytes, where 72 is
    /// `size_of::<Held>()` measured rather than counted, and the lowering makes
    /// about two and a half locals per line of C. A 489-line function costs
    /// around 2.8 GB and six seconds, against 1.5 GB and three before
    /// [`Held::writes_to`] was added; the field beside it costs about one per
    /// cent more. A packed bitset is the answer when a third square field
    /// arrives or when somebody hits this on real code; until then the number
    /// is here so that it is a decision rather than a discovery.
    sites: Vec<bool>,
    /// Whether this local may hold an allocation this check can no longer
    /// name.
    ///
    /// Three doors lead here. The site that named what it held was handed to a
    /// second allocation, which is ADR-0018; or its address had escaped when a
    /// call this check cannot read ran, and such a call may have written a
    /// pointer this check has never seen into it, which is ADR-0029; or its
    /// address had escaped when a write through a pointer this check cannot
    /// pin down wrote its type, which is ADR-0031 and is the same sentence
    /// with the writer inside this function.
    lost: bool,
    /// Where the allocation this local held was freed, when the free could not
    /// say which member of the set it was.
    ///
    /// A free of a local reaching two sites frees exactly one of them, and
    /// writing `Freed` on both said each was certainly freed: a later free of
    /// one by name became a proved double free about a program with no defect
    /// on one path. What is true is a fact about the **set**, and the local
    /// that named the set is the only place it fits. See ADR-0020.
    freed: Option<Freeing>,
    /// Per local, whether a write **through** this one may land in it.
    ///
    /// The edge `Known::escaped` is the shadow of. That field says a local's
    /// address is held by something; this says by whom, which is what a write
    /// through a pointer needs in order to be followed rather than feared. See
    /// ADR-0019.
    ///
    /// It is here rather than beside `escaped` because an assignment destroys
    /// it: `pp = 0;` ends what a write through `pp` can reach, while the fact
    /// that `p`'s address escaped outlives anything done to `pp`. That is
    /// ADR-0018's rule for what this struct holds.
    writes_to: Vec<bool>,
    /// Whether a write through this local may land somewhere [`writes_to`]
    /// does not name.
    ///
    /// The field above is a may-set, so one member in it means "at most one
    /// target this check has seen an address for" and not "this one". A
    /// pointer this check never saw an address taken into has an empty set,
    /// and a union of an empty set with one member is one member: without
    /// this flag, `pp` that must point at `p` and `pp` that may point
    /// anywhere are the same value. Telling them apart is what lets a write
    /// whose target is certain replace what that target held rather than
    /// union into it. See ADR-0028.
    ///
    /// `true` is the answer that gives up, which is why [`Held::none`] seeds
    /// it: that value is the lattice's identity and is also what every local
    /// holds where a function starts.
    ///
    /// [`writes_to`]: Held::writes_to
    writes_elsewhere: bool,
    /// Where in every site above this local's value points.
    ///
    /// **One answer for the whole set rather than one per site**, because
    /// arithmetic applies its offset to the whole may-set at once: a row per
    /// site would answer the same thing in every column. It also keeps this
    /// struct at two square tables, which is the condition [`Held::sites`]
    /// names for reaching for a packed bitset.
    ///
    /// [`Held::hold`] takes it as an argument, so that a new producer of a site
    /// cannot be written without answering where in it the value points. See
    /// ADR-0036.
    offset: Offset,
}

impl Held {
    /// A local holding nothing, in a function with this many sites.
    fn none(sites: usize) -> Self {
        Held {
            sites: vec![false; sites],
            lost: false,
            freed: None,
            writes_to: vec![false; sites],
            writes_elsewhere: true,
            // Nothing held, so nothing to be off the start of. `Unknown` here
            // would be a warning about an offset nobody wrote, on every path
            // that holds nothing and, through `Held::hold`'s join, on every
            // parameter. See ADR-0036.
            offset: Offset::Zero,
        }
    }

    /// Also hold this site, at this offset into it.
    ///
    /// **The offset is an argument so that it cannot be forgotten.** A site
    /// arrives from a call's destination or from a parameter, each of which is
    /// the start of whatever it stands for, and a third producer written later
    /// has to say the same or something else: dropping the argument is
    /// `error[E0061]` at every call site. See ADR-0036.
    ///
    /// **Joined rather than assigned, and nothing tells the two apart today.**
    /// Both callers hold on a local [`Held::none`] or [`Held::clear`] has just
    /// left at `Zero`, and `Zero` joined with `Zero` is `Zero`. A caller that
    /// holds a second site on a local already holding one is where they part,
    /// and the join is the answer that cannot claim more than both said.
    fn hold(&mut self, site: usize, offset: Offset) {
        self.sites[site] = true;
        self.offset = self.offset.joined(offset);
    }

    /// Every site held, in order.
    fn sites(&self) -> impl Iterator<Item = usize> + '_ {
        self.sites
            .iter()
            .enumerate()
            .filter_map(|(site, held)| held.then_some(site))
    }

    /// Hold nothing, and hold nothing unnameable either.
    ///
    /// What a local is *given* replaces what it held, so a name it had lost
    /// goes with the rest. Keeping it would outlive the thing it was about:
    /// `keep = 0;` after `keep` lost its allocation would still answer
    /// [`Reached::Lost`] for every read, and a null dereference belongs to an
    /// axis this check has none for.
    fn clear(&mut self) {
        // Every field named, never `..`: a field added here and missed is a
        // fact that survives an assignment, which is RK-018's shape one type
        // over.
        let Held {
            sites,
            lost,
            freed,
            writes_to,
            writes_elsewhere,
            offset,
        } = self;
        sites.fill(false);
        *lost = false;
        *freed = None;
        writes_to.fill(false);
        // What a local is given ends the claim that this check knew where a
        // write through it landed, along with the set that claim was about.
        *writes_elsewhere = true;
        // What `Held::none` answers, for its reason.
        *offset = Offset::Zero;
    }

    /// Also hold everything that one holds, where two paths meet.
    ///
    /// **The lattice's join, and one of two algebras this struct has.**
    /// [`Self::accumulated`] is the other, and they differ on exactly one
    /// field. Splitting them is ADR-0024; what made one method wrong for both
    /// is that [`Held::none`] is the identity for three of these fields and the
    /// zero for the fourth, so building a value up from nothing cleared a proof
    /// every time.
    fn joined(&mut self, other: &Held) {
        // Every field named, never `..`, in this method and in its neighbour
        // alike: a field added to this struct is `error[E0027]` in both and has
        // to say what it means in each. RK-018 is the spelling and ADR-0024 is
        // why there are two places to answer.
        let Held {
            sites,
            lost,
            freed,
            writes_to,
            writes_elsewhere,
            offset,
        } = self;
        *offset = offset.joined(other.offset);
        // **A path that knows where a write through this local lands and a
        // path that does not is a path that does not.** This is what keeps a
        // strong update out of `if (c) { pp = &p; } *pp = q;`, where the
        // union below leaves one target and only this says the set is not
        // all of it. See ADR-0028.
        *writes_elsewhere = *writes_elsewhere || other.writes_elsewhere;
        for (here, there) in sites.iter_mut().zip(&other.sites) {
            *here = *here || *there;
        }
        *lost = *lost || other.lost;
        // **An intersection, where every other field here is a union.** The
        // rest of this struct holds may-facts, which grow where paths meet.
        // This one is a proof, and a path that did not free proves nothing:
        // joining it the way its neighbours join would report a proved double
        // free on `if (c) { free(p); } free(p);`. See ADR-0020.
        *freed = match (*freed, other.freed) {
            (Some(here), Some(there)) => Some(here.joined(there)),
            _ => None,
        };
        for (here, there) in writes_to.iter_mut().zip(&other.writes_to) {
            *here = *here || *there;
        }
    }

    /// Also hold everything that one holds, where a value is being built from
    /// the operands that produced it.
    ///
    /// **The proof survives while the set does not grow.** [`Self::freed`] says
    /// the set this local named had one member freed, and what that is worth is
    /// that freeing the local again takes the same member. A set that has
    /// gained a site nothing freed no longer supports it: with `i` a parameter,
    /// and so a site, `free(q); p = q + i; free(p);` reaches `i`'s site as well
    /// as `q`'s, and answering proved there is a proof this check cannot make.
    /// Measured, and it is the reading a reader arrives at first. See ADR-0024.
    ///
    /// **Read before the union, because after it every site is `self`'s.**
    /// Asking afterwards answers that nothing ever grows, which is the same
    /// mistake spelled as an ordering.
    fn accumulated(&mut self, other: &Held) {
        let Held {
            sites,
            lost,
            freed,
            writes_to,
            writes_elsewhere,
            offset,
        } = self;

        // **Where the result points is the fold's to say, and not this
        // method's**, for the reason the line below gives about the proof:
        // what decides it is which operand was the pointer and what the other
        // one was, and this sees neither. [`built_from`] assigns over it. The
        // one other caller is a write through a pointer, whose target has
        // escaped and so is never asked. See ADR-0036.
        *offset = Offset::Unknown;

        // **A second operand takes the proof with it.** What
        // [`Self::freed`] is worth is that freeing this local again takes the
        // same member, and an expression built out of two operands is not that
        // local offset: it may be the *other* operand's value. This check does
        // not read types, so a local holding no site is an integer and a
        // pointer whose allocation it lost at the same time, and `base + ok`
        // with `base` read out of another pointer is `p + n` with `n` an `int`.
        // Keeping the proof for one keeps it for both, and one of them is a
        // certainty about a value nothing here has followed. See ADR-0024.
        *freed = None;

        // The may-facts, which grow wherever anything meets. The edge and the
        // flag beside it are the ones nothing observes here: both callers that
        // build a value out of operands give up on both on the next line,
        // because C17 6.5.6 p8 keeps a pointer's arithmetic inside the object
        // and a local's address plus one is not that local. See ADR-0019, and
        // the two `writes_to.fill(false)` lines in `Allocations::element`.
        for (here, there) in sites.iter_mut().zip(&other.sites) {
            *here = *here || *there;
        }
        *lost = *lost || other.lost;
        for (here, there) in writes_to.iter_mut().zip(&other.writes_to) {
            *here = *here || *there;
        }
        *writes_elsewhere = *writes_elsewhere || other.writes_elsewhere;
    }

    /// Stop pointing at a site that now names something else.
    ///
    /// **Not [`Held::clear`], and the difference is the whole of ADR-0018.**
    /// The name is gone and what was held is not, so a local that reached one
    /// site now reaches none *and says so*. Clearing instead would leave it
    /// reaching nothing with nothing to say, and a read through it would be a
    /// silence.
    fn lose(&mut self, site: usize) {
        if self.sites[site] {
            self.sites[site] = false;
            self.lost = true;
        }
    }
}

/// A dereference this walk has met since the last sequence point.
///
/// **The other half of ADR-0022, which this one is ADR-0023 for.** An
/// [`Element::Sequenced`] says what is ordered and a forward walk only ever
/// looks back, so a free can be compared with the reads behind it and never
/// with the ones ahead. `int x = g(*p) + (free(p), 0);` read `*p` first and was
/// silent under every flag where its mirror reported. What closes it is
/// carrying the read forwards instead of looking backwards for it: everything
/// met since the last marker is still unordered against whatever comes next in
/// the same full expression.
///
/// **The sites are resolved where the use is and not where the free is.**
/// `x = *p + (p = q, free(p), 0)` reads one allocation and frees another, and
/// asking at the free would answer about the wrong one. That is a silence
/// rather than a false positive, which is the direction this check cannot
/// afford.
#[derive(Clone, PartialEq, Eq)]
struct PendingRead {
    /// The element's span, which is where `used here` goes.
    ///
    /// Kept beside the key rather than in it, because a [`Span`] cannot be
    /// rebuilt from the position the key holds.
    at: Span,
    /// Which allocations it may have read.
    sites: BTreeSet<usize>,
}

/// What one of them is filed under: where the read is, and what it read
/// through.
///
/// A position rather than the [`Span`] itself, because the map has to order its
/// keys and a span is not ordered. The file is the first part of it for
/// [`earlier`]'s reason.
///
/// **The place is the fourth part because this is the pair a report is
/// collapsed on**, and [`say`] says what each half of it costs when it goes.
/// Two reads at one span through one place are one report; two reads at one
/// span through two places are two.
///
/// The end as well as the start, so that two elements beginning at one column
/// and covering different extents are two reads rather than one. Nothing
/// reaches that today and ADR-0023 says so: dropping the end leaves the whole
/// suite green, and what it would cost is one of two reports rather than a
/// wrong one.
type ReadKey = (usize, u32, u32, Place);

/// Which allocations each local may hold, and what is known about each.
#[derive(Clone, PartialEq, Eq)]
struct Known {
    /// Per local, what it may hold.
    points_to: Vec<Held>,
    /// Per site, what is known about it. Meaningless for a local nothing points
    /// at, which is most of them.
    state: Vec<SiteState>,
    /// Per local, whether anything holds its address.
    ///
    /// **A property of the local rather than of what it held when the address
    /// was taken.** Marking only the sites it reached at that instant was
    /// #155: assigning to the local afterwards gave it a fresh site nothing
    /// had lost, so `int **pp = &p; p = malloc(8); *pp = q; *p = 1;` read a
    /// freed pointer in silence. Whatever an escaped local is given later is
    /// no more proved than what it held before, because the write that put it
    /// there is not the only write that can reach it.
    escaped: Vec<bool>,
    /// What has been read through a pointer since the last sequence point.
    ///
    /// **Ordered containers because a lattice value has to be canonical, and
    /// the type is what holds that rather than a rule somebody maintains.**
    /// [`crate::dataflow::Analysis::Value`] decides whether the walk has ended
    /// by comparing, so a value holding the same facts in two arrangements
    /// never compares equal and never converges. ADR-0016 measured that as a
    /// hang. Written as sorted vectors this needed four rules kept in step by
    /// hand, and a mutation of each left the whole suite green: a map and a set
    /// have no arrangement to get wrong. [`Place`] carries the `Ord` the map
    /// orders by for no other reason. See ADR-0023.
    pending: BTreeMap<ReadKey, PendingRead>,
}

impl Known {
    /// Every site this local may point at.
    fn sites_of(&self, local: LocalId) -> impl Iterator<Item = usize> + '_ {
        self.points_to[local.index()].sites()
    }

    /// Stop following whatever this local held.
    fn clear(&mut self, local: LocalId) {
        self.points_to[local.index()].clear();
    }

    /// Every site this local may hold, and whether it may hold more.
    ///
    /// The one question both readers ask, so that what a free touches and what
    /// a dereference reads cannot disagree. [`escaped`] says the set may be
    /// stale, which is a fact about this check's knowledge of one local and is
    /// answered here; what an escape does to the allocations themselves is a
    /// fact about the heap and is written on the sites by [`Known::unproved`].
    /// See ADR-0017.
    ///
    /// **A local reaching no site answers nothing, escaped or not.** [`used`]
    /// says nothing about a place it follows no allocation for, however it came
    /// to follow none, and an address taken does not change that: a pointer
    /// this check never had a site for is an indeterminate pointer, which is a
    /// different defect with a check of its own that does not exist yet.
    /// Answering [`Reached::Lost`] here instead put `perhaps after the free` on
    /// a program that frees nothing at all, and made a build that denies
    /// unknown unusable on the output-parameter idiom. `an_escaped_local_that_reaches_no_site_at_all`
    /// is that program and is the guard.
    ///
    /// It used to be `int *p; int **pp = &p; *pp = malloc(4); *p = 1;`, which
    /// ADR-0019 made this check follow: `p` now reaches the site the write put
    /// there, so the rule above no longer covers it and the escape reports it
    /// anyway. The cost that rule was written to avoid is paid on that program
    /// regardless, by a route that knows what it is talking about.
    ///
    /// [`Allocations::touching`] wants the opposite answer for an empty set and
    /// writes its own, which is why the rule is not written here.
    ///
    /// [`escaped`]: Known::escaped
    fn reached_by(&self, local: LocalId) -> Vec<Reached> {
        let mut reached: Vec<Reached> = self.sites_of(local).map(Reached::Site).collect();

        // **Two facts, one answer, and the emptiness condition belongs to only
        // one of them.** A local that *had* a site and lost the name for it is
        // holding something unnameable whether or not it holds anything else,
        // which is what [`Reached::Lost`] is for and is ADR-0018. A local whose
        // address escaped is distrusted only about the sites it has: reaching
        // none of them makes it a pointer this check never followed, and
        // answering here put `perhaps after the free` on programs that free
        // nothing at all, which is ADR-0017.
        //
        // One `push` rather than two conditions, because two would answer
        // `Lost` twice for a local that is both. That is inert, since
        // [`verdict`] reads `Lost` as a flag, and a reader should not have to
        // work that out.
        // **It replaces the members rather than joining them.** What is known
        // is about the set, and its members were each left `SiteState::Unknown`
        // by the free that could not say which one it took. Answering both
        // folds one of those in beside the proof, and `verdict` needs nothing
        // unknown to prove anything, so the proof goes. Written the other way
        // first and measured: `a_branch_that_allocates_either_way` dropped to a
        // warning, which is the rule eating the marking it made. See ADR-0020.
        //
        // **It replaces, and it does not return.** The rules below are about
        // this local too and they still apply: a set this local freed says
        // nothing about whether something else has written a different pointer
        // into it since. Returning here made `free(p); opaque(&p); free(p);`
        // over a two-site `p` a proved double free, which is the false proof
        // this rule exists to stop, arriving through a door it had just
        // opened.
        if let Some(freed) = self.points_to[local.index()].freed {
            reached.clear();
            reached.push(Reached::SetFreed(freed));
        }

        if self.points_to[local.index()].lost
            || (self.escaped[local.index()] && !reached.is_empty())
        {
            reached.push(Reached::Lost);
        }

        reached
    }

    /// Record that this place was read through, where the read reaches an
    /// allocation.
    ///
    /// **A place reaching no site records nothing, and that is a size rather
    /// than a rule.** Measured: recording one anyway changes no answer, because
    /// what [`used_before`] compares is sites and an entry with none can never
    /// meet a free's. So this is skipped to keep the value small, and the
    /// asymmetry RK-049 is about is held by the comparison rather than by this
    /// line: a dereference of a pointer this check never followed says nothing
    /// here for the same reason it says nothing in [`used`], which is that
    /// there is no allocation to say it about.
    ///
    /// The sites arrive ascending, because [`Held::sites`] walks a row of a
    /// table in order, and stay that way.
    ///
    /// It takes what [`dereferenced_in_element`] answers rather than one place,
    /// so that what the walk carries forwards and what [`used`] reports on are
    /// decided by one function with two callers. RK-052 in the review knowledge
    /// bank is one rule written in two places drifting apart inside the change
    /// that touches one of them.
    fn met(&mut self, read: Option<(Span, Vec<&Place>)>) {
        let Some((at, dereferenced)) = read else {
            return;
        };

        for place in dereferenced {
            self.meeting(at, place);
        }
    }

    /// One place of one element, for [`Self::met`].
    fn meeting(&mut self, at: Span, place: &Place) {
        let sites: BTreeSet<usize> = named(&self.reached_by(place.local)).collect();

        if sites.is_empty() {
            return;
        }

        // One entry per key: `*p = *p;` reads through one place twice at one
        // span, and two entries would be one report said twice.
        self.pending
            .entry(read_key(at, place))
            .or_insert(PendingRead {
                at,
                sites: BTreeSet::new(),
            })
            .sites
            .extend(sites);
    }

    /// Every local a write through this one may land in.
    ///
    /// Empty means this check does not know where such a write goes, which is
    /// not the same as knowing it goes nowhere. ADR-0019 records why the answer
    /// to not knowing is to do nothing.
    fn written_through(&self, local: LocalId) -> Vec<usize> {
        (0..self.points_to.len())
            .filter(|target| self.points_to[local.index()].writes_to[*target])
            .collect()
    }

    /// Hand this site to a new allocation.
    ///
    /// **A site names one allocation at a time, and this is where it stops
    /// naming the last one.** A site is a local, so a loop through the same
    /// call files each allocation it makes under one name. Every other local
    /// that still pointed here was following the allocation an earlier turn
    /// made, and `state` below is about the one this call just made: handing
    /// them that proof reported the wrong line and was silent about the right
    /// one. See ADR-0018, which has the program.
    ///
    /// The destination is excluded because the caller has already rebuilt its
    /// row, and the site it holds now is this allocation rather than the one
    /// being displaced.
    ///
    /// **Every field named, never `..`.** A fact filed against a site is a
    /// fact about whatever the site named when it was written, so a field
    /// added to [`Known`] has to answer here for what happens when the site
    /// starts naming something else. `error[E0027]` is what asks, and RK-018
    /// is the same spelling one type over.
    fn reborn(&mut self, site: usize, made: Option<Span>) {
        let Known {
            points_to,
            state,
            escaped: _,
            pending,
        } = self;

        for (other, held) in points_to.iter_mut().enumerate() {
            if other != site {
                held.lose(site);
            }
        }

        state[site] = SiteState::Live(made);

        // **A read of the allocation this site used to name is not a read of
        // the one it names now.** Left standing, a free of the new allocation
        // later in the same full expression would be reported against a read
        // of the old one, which is a caret on a line that read something else.
        // No C program reaches this, because two calls in one full expression
        // are two call sites and a site is the local a call writes into; a
        // frontend whose calls share one can, and this is the answer
        // `error[E0027]` asked for when the field was added.
        for entry in pending.values_mut() {
            entry.sites.remove(&site);
        }
        pending.retain(|_, entry| !entry.sites.is_empty());
    }

    /// Nothing this local holds is proved, once anything holds its address.
    ///
    /// Called wherever a local is *given* something, so that the escape
    /// outlives the sites it was recorded against. Towards `Unknown` and never
    /// away from it, which is why getting this wrong costs a false positive
    /// rather than a silence: a may-set cannot prove anything from one member
    /// and this only ever takes proof away.
    ///
    /// **What a local is given, and not what is done to what it holds.** A
    /// `free` writes `SiteState::Freed` over a site whatever the local's
    /// escape says, so `int **pp = &p; free(p); *pp = q; *p = 1;` is a proved
    /// `Unsafe` about a program with no defect in it. That is not this rule
    /// arriving late, it is the escape and the free recorded in one slot and
    /// overwriting each other; it predates this and is issue #161.
    ///
    /// **This is only half of what an escape means, and the half about the
    /// heap.** What it means about the local's own set is answered at the
    /// report by [`Self::reached_by`], which is why nothing here is about
    /// whether the escaped local itself can be trusted. See ADR-0017.
    ///
    /// An index rather than a [`LocalId`], because [`Self::settle`] walks the
    /// rows of a side table and `LocalId` cannot be built from one.
    fn unproved(&mut self, local: usize) {
        if !self.escaped[local] {
            return;
        }

        for site in self.points_to[local].sites() {
            self.state[site] = SiteState::Unknown;
        }
    }

    /// [`Self::unproved`] for every local at once.
    ///
    /// **A join gives a local sites without assigning to it.** An arm that
    /// took the address and an arm that allocated meet here, and the merged
    /// value held an escaped local reaching a site this check had proved live:
    /// `if (c) { pp = &p; *pp = q; } else { p = malloc(8); } free(q); free(p);`
    /// was silent at `--safety strict` on a double free. The
    /// union of `escaped` alone does not do it, because nothing downstream of
    /// a join reads the bit unless the local is written again.
    ///
    /// **This is why the list of places is closed rather than long.** A value
    /// changes in `Analysis::on_entry`, `join`, `element` and `terminator` and
    /// nowhere else, so covering the last three covers every one: nothing has
    /// escaped where a function starts.
    fn settle(&mut self) {
        for local in 0..self.escaped.len() {
            self.unproved(local);
        }
    }

    /// Everything an escaped local this writer could have reached holds may
    /// have been replaced.
    ///
    /// A writer this check cannot follow may write through any address that
    /// has escaped, and nothing here can say which local that reaches: the
    /// address may have been stashed anywhere on the way. So every escaped
    /// local it could have reached holds something this check cannot name,
    /// which is what [`Held::lost`] says and is ADR-0018's fact through a
    /// second door.
    ///
    /// **Which locals that is belongs to the caller**, because the two writers
    /// know different amounts. A call this check cannot read may write any
    /// type, since nothing here reads a callee's body, and answers `true` for
    /// every local; a write through a pointer writes one type and says so, and
    /// ADR-0031 is why that narrowing is the difference between a rule that
    /// costs what it should and one that costs everything. Deciding it here
    /// instead would make one of the two answer a question written for the
    /// other, which is RK-055 in the review knowledge bank.
    ///
    /// **The bit alone is not the point.** [`Self::reached_by`] already
    /// answers [`Reached::Lost`] for an escaped local that holds a site, so no
    /// report about the local itself moves. What moves is what a later `free`
    /// of it is entitled to write on the sites, which are shared with every
    /// other local holding them. See ADR-0029.
    ///
    /// **A local holding no site is left alone, and that is the reason the
    /// condition is not just the escape.** Its `lost` bit is read with no
    /// emptiness condition, so marking it would answer [`Reached::Lost`] for a
    /// pointer this check never followed: `int *p; get(&p); *p = 1;` is the
    /// output-parameter idiom, and ADR-0017 declined to report it on purpose.
    /// There is nothing to lose by leaving it, because a local holding no site
    /// shares none with anybody, and a `free` of it is reported by
    /// [`Allocations::touching`]'s own rule whatever this says.
    fn replaced(&mut self, may_hold: impl Fn(usize) -> bool) {
        for local in 0..self.escaped.len() {
            if may_hold(local)
                && self.escaped[local]
                && self.points_to[local].sites().next().is_some()
            {
                self.points_to[local].lost = true;
            }
        }
    }
}

/// What the operands of a binary operation build.
///
/// **Where one operand is a pointer, it is the only one that contributed.**
/// C17 6.5.6 p8 keeps the result of pointer arithmetic inside the object the
/// pointer operand points into, so the integer beside it cannot decide what the
/// result reaches whatever that integer happens to hold. That is a statement
/// about the program rather than a belief about a type, which is what lets a
/// may-set be narrowed here at all. See ADR-0030.
///
/// **The proof survives only where nothing else contributed.** One contributing
/// operand and a constant is `p + 1`: the result is that operand offset, and
/// the clause above keeps it inside the same object, so the set the proof is
/// about is the set the result names. Two of them is an expression whose value
/// may be either, and [`Held::accumulated`] drops the proof for the reason
/// written there.
///
/// **Where in its sites the result points is answered last**, by
/// [`offset_of`], and it is the one thing here that turns on the operator.
///
/// One function with two callers, because the same question is asked where a
/// value is assigned and where one is written through a pointer, and RK-052 in
/// the review knowledge bank is one rule in two places drifting apart inside
/// the change that touches one of them.
fn built_from(
    op: BinOp,
    operands: [&Operand; 2],
    value: &Known,
    is_pointer: impl Fn(LocalId) -> bool,
) -> Held {
    let followed: Vec<LocalId> = operands
        .iter()
        .filter_map(|operand| match operand {
            // A constant is not a value this check follows, and neither is a
            // read through a projection: `*pp + 1` reaches whatever `pp` points
            // at, which is a place rather than a local.
            Operand::Copy(source) if source.projection.is_empty() => Some(source.local),
            _ => None,
        })
        .collect();

    // **An operation with no pointer operand keeps every one of them**, and
    // this is the half C says nothing about: two integers added together are
    // not pointer arithmetic, so no clause says the result cannot reach what
    // its operands reach. Narrowing there would be narrowing on nobody's
    // authority, and RK-045 is what an emptied set costs: a dereference of a
    // local that reaches no site is reported by nothing at all.
    //
    // **`i + j` reaches this constantly**, and what is rare is one of those
    // integers holding an allocation. It takes a program C forbids, which this
    // compiler does not yet refuse: `int i = p;` is a constraint violation
    // under C17 6.5.16.1 p1 and #154 is the check that is missing. See
    // ADR-0030, which measures what this branch is worth on such a program.
    let followed: Vec<usize> = if followed.iter().any(|&local| is_pointer(local)) {
        followed
            .iter()
            .filter(|&&local| is_pointer(local))
            .map(|local| local.index())
            .collect()
    } else {
        followed.iter().map(|local| local.index()).collect()
    };

    let mut reached = Held::none(value.points_to.len());
    for &source in &followed {
        reached.accumulated(&value.points_to[source]);
    }

    if let [one] = followed[..] {
        reached.freed = value.points_to[one].freed;
    }

    reached.offset = offset_of(op, operands, &followed, value);

    reached
}

/// Where the result of a binary operation points in the sites it reaches.
///
/// `Offset::NonZero` where all of these hold, and `Offset::Unknown` otherwise:
///
/// * the operator is `+`, or `-` with the followed operand on the left. C17
///   6.5.6 p8 keeps `P + N` and `N + P` inside the object `P` points into, p9
///   is the same with the sign turned round, and p3 allows a pointer only on
///   the left of a `-`.
/// * exactly one operand was followed. Two is `p - q`, which 6.5.6 p9 makes a
///   `ptrdiff_t` rather than a pointer, or a shape no C program builds.
/// * the other operand is a constant, and **the constant is read rather than
///   assumed non-zero**. ADR-0021 folds `p + 0` away where the IR is built, and
///   `docs/c-family.md` records that nothing enforces it, so an IR from a
///   frontend that skipped the fold would hand this a literal zero. Answering
///   `Unknown` there is row 4, where assuming would be row 6. RK-067 is leaning
///   on an invariant nobody enforces.
/// * the followed operand is itself at the start. `q = p + 1; r = q - 1;` has
///   `r` back at the start, and nothing here carries a distance to know it.
///
/// Every other operator answers `Unknown`, a `*` included: C17 6.5.5 p2 gives
/// it arithmetic operands only, and a hand-built IR can hold one anyway. See
/// ADR-0036.
fn offset_of(op: BinOp, operands: [&Operand; 2], followed: &[usize], value: &Known) -> Offset {
    let [one] = followed[..] else {
        return Offset::Unknown;
    };

    let moved = match (op, operands) {
        (BinOp::Add | BinOp::Sub, [Operand::Copy(_), Operand::Constant(by)])
        | (BinOp::Add, [Operand::Constant(by), Operand::Copy(_)]) => *by != 0,
        _ => false,
    };

    if moved && value.points_to[one].offset == Offset::Zero {
        Offset::NonZero
    } else {
        Offset::Unknown
    }
}

/// Distrust every escaped local a write through this place may have reached.
///
/// ADR-0029's fact with the writer inside the function: whoever holds an
/// escaped address may have been handed it through a projection this check
/// does not follow, so a write it cannot pin down may replace what an escaped
/// local holds, and writing `SiteState::Freed` on that local's sites later is
/// a proof about an allocation this write may have swapped out. See ADR-0031.
///
/// **Only a local declared with the type this write writes.** C17 6.5 p7 gives
/// an object an effective type and lets an lvalue of another type access it
/// only where that type is a character type, so a write of an `int` cannot
/// replace a pointer and the rule costs what it should rather than every
/// escaped local at every `*p = 1`. `docs/c-family.md` carries what that asks
/// of a frontend, and getting it wrong keeps a proof rather than losing one,
/// which is a false positive and not a silence.
///
/// **A character type reaches everything, which is the exception that clause
/// carries**, and no cast is needed to reach it: C17 6.3.2.3 p1 and 6.5.16.1
/// p1 make the `void *` round trip implicit both ways, so `void *v = &p;
/// char *c = v;` is a conforming program this frontend accepts, and copying
/// one pointer's object representation through `c` is defined. Review found
/// that one, and compiled and ran the C under a sanitiser to show the program
/// has no use after free in it.
///
/// [`TranslationUnit::place_ty`] answers `None` for a `Deref` of something
/// that is not a pointer, which the lowering does not build and a hand-built
/// unit can. Every local then, because a write whose type this cannot name is
/// a write it cannot narrow.
///
/// A free function rather than a method, because it is the whole of what one
/// call site does and reads nothing of [`Allocations`] but the unit.
fn replaced_by(unit: &TranslationUnit, function: &Function, place: &Place, value: &mut Known) {
    let written_ty = unit.place_ty(function, place);
    let everything = matches!(written_ty.map(|ty| unit.ty(ty)), None | Some(Ty::Char));
    // A row of bytes per local, against a value that is already square in
    // them, because the predicate is asked per index and `LocalId` cannot be
    // built from one.
    let may_hold: Vec<bool> = function
        .locals()
        .map(|local| everything || written_ty == Some(function.local(local)))
        .collect();

    value.replaced(|local| may_hold[local]);
}

/// The sites out of everything a place or an argument reached.
///
/// **One fold with three callers, because the three have to agree.** It is what
/// a free writes `Freed` on, what a read carries forwards as the allocations it
/// may have touched, and what the two are compared against when the order
/// between them is open. A free that wrote on a set this did not answer, or a
/// read that carried one, would be a report about an allocation the other half
/// never considered. RK-052 in the review knowledge bank is one rule written in
/// two places drifting apart.
///
/// Neither of the other two variants names a site: one is a fact about a set,
/// which ADR-0020 records on the local rather than on its members, and the
/// other is this check having lost the pointer.
fn named(reached: &[Reached]) -> impl Iterator<Item = usize> + '_ {
    reached.iter().filter_map(|reached| match reached {
        Reached::Site(site) => Some(*site),
        Reached::SetFreed(_) | Reached::Lost => None,
    })
}

/// The analysis: where an allocation is, and whether it has been freed.
struct Allocations<'a> {
    sources: &'a SourceMap,
    unit: &'a TranslationUnit,
    /// How many locals the function has, which is how many sites there can be.
    locals: usize,
    /// The locals a caller filled, which are sites because an allocation can
    /// arrive through one.
    parameters: Vec<LocalId>,
}

impl Allocations<'_> {
    /// What this check can read in the name of the function being called.
    fn callee(&self, id: FuncId) -> Callee {
        match self.sources.snippet(self.unit.function(id).name) {
            "free" => Callee::Frees,
            "malloc" => Callee::Allocates,
            _ => Callee::Opaque,
        }
    }

    /// Whether this local holds a pointer.
    ///
    /// What tells the pointer operand of an addition from the integer beside
    /// it, which is the question [`built_from`] asks and C17 6.5.6 p8 answers.
    /// See ADR-0030.
    ///
    /// **Written out rather than as a `matches!`, because the answer for a kind
    /// nobody has added yet is not `false`.** A type this does not recognise is
    /// dropped from an addition that has a pointer beside it, and a dropped
    /// operand is a site nothing reports: `error[E0004]` here is what asks a
    /// fourth kind of type whether it is one. RK-015 is the limit of that, and
    /// this is the case where a reader has something to decide rather than a
    /// line to fill in.
    fn is_pointer(&self, function: &Function, local: LocalId) -> bool {
        match self.unit.ty(function.local(local)) {
            Ty::Pointer(_) => true,
            Ty::Int | Ty::Char | Ty::Void => false,
        }
    }

    /// What the arguments of a call reach, in the order they were written.
    ///
    /// Shared with [`findings`], so that the walk which reports and the walk which
    /// computes cannot disagree about what a call touches.
    ///
    /// **What they may disagree about is which arguments are handed here**, and
    /// exactly one thing does it: `asked` leaves out a pointer the nullability
    /// check established is null, because C says such a call does nothing, and
    /// only the walk that reports asks that. See ADR-0027, and `asked` for why
    /// the transfer is deliberately not told.
    fn touching<'o>(
        arguments: impl IntoIterator<Item = &'o Operand>,
        known: &Known,
    ) -> Vec<Reached> {
        let mut reached = Vec::new();

        for argument in arguments {
            let place = match argument {
                // Not a pointer that went missing. `free(0)` is the case, and
                // C17 7.22.3.3 p2 makes it do nothing, so it is written on
                // purpose and is not something this check lost track of.
                //
                // **The arm is wider than the clause**, which covers the null
                // pointer and makes every other address undefined. `free(17)`
                // is skipped here too and this check says nothing about it.
                // What keeps that from mattering is not this line: C17
                // 6.5.2.2 p2 makes the call a constraint violation, so a
                // conforming implementation has to diagnose it before any
                // analysis runs, and the reason this one does not is the
                // missing assignment-constraint check that #154 is about.
                // `a_free_of_a_null_constant` pins the half the clause
                // supports; the other half is held by nobody here and is not
                // this check's to hold.
                //
                // **The same clause reaches a local**, where the constant was
                // given a name first, and that is `asked`'s rather than this
                // arm's: what it takes to recognise is a nullness the other
                // check established, which nothing here can see. See ADR-0027.
                Operand::Constant(_) => continue,
                Operand::Copy(place) => place,
            };

            if !place.projection.is_empty() {
                // `free(*pp)` frees whatever `pp` points at, and this check
                // follows locals rather than what they point at.
                reached.push(Reached::Lost);
                continue;
            }

            let before = reached.len();
            reached.extend(known.reached_by(place.local));
            if reached.len() == before {
                reached.push(Reached::Lost);
            }
        }

        reached
    }

    /// Whether this call is handed a local that may hold an allocation this
    /// check cannot name.
    ///
    /// **A second question about the same arguments**, and it cannot be folded
    /// into [`Allocations::touching`]: that answers [`Reached::Lost`] for an
    /// escaped local as well, which is ADR-0017, and the two facts are not
    /// interchangeable here. Freeing an escaped local that no call has run past
    /// still frees what this check thinks it holds, and reading the folded
    /// answer instead would give up the proof
    /// `a_free_through_an_escaped_local_is_seen_by_a_sharer` holds. RK-052 is
    /// one rule written in two places drifting apart, so the argument walk is
    /// spelled the way its twin above spells it and the reason there are two is
    /// written here. See ADR-0029.
    ///
    /// **Neither arm is observable, and both are here anyway.** `free` takes
    /// one argument, so the argument this walk skips is the only argument
    /// there is, and the branch that reads this then writes on nothing:
    /// answering `true` for a constant, and dropping the projection test, each
    /// leave the whole workspace green, measured. The `reached.len() > 1`
    /// branch says the same thing about the same premise. What they would cost the day something
    /// reaches them is a free refusing to prove because of a row belonging to
    /// a pointer rather than to what it points at. The arm above them is the
    /// one that decides anything.
    fn holds_something_unnameable(arguments: &[Operand], known: &Known) -> bool {
        arguments.iter().any(|argument| match argument {
            // A constant holds no allocation, for the reason `touching` gives.
            Operand::Constant(_) => false,
            // A projection names a place rather than a local, and this check
            // follows locals: `free(*pp)` is already a `Reached::Lost` above,
            // and there is no row here to ask.
            Operand::Copy(place) => {
                place.projection.is_empty() && known.points_to[place.local.index()].lost
            }
        })
    }
}

impl Analysis for Allocations<'_> {
    type Value = Known;

    fn height(&self, function: &Function) -> usize {
        let locals = function.locals().len();
        // Each local's set of sites only grows, so it takes at most one step
        // per site, and a site is a local. Each site's state walks `Live` to
        // `Freed` to `Unknown`; its `freed` span can only move to an earlier
        // one, which it can do at most once per site that frees; and its `made`
        // span can only fall from `Some` to `None`, once. A local's `escaped`
        // bit goes from `false` to `true` and never back, once each, and its
        // `lost` bit costs one more of the same. Each local's set of locals a
        // write through it may reach is a second square table that only grows,
        // so it takes at most one step per pair, and whether that table is all
        // of what a write through it may reach goes from `false` to `true`
        // once per local, for one more step each. Where the set it named was
        // freed goes from `None` to `Some` once per local and a join only takes
        // it away, so it costs one more step each. Whether a free has been
        // sequenced is one bit per site: the transfer sets it and only a join
        // takes it back, which a join can do once, so it is one more step per
        // site and the number below is not changed for it. That is slack being
        // spent rather than a bound being re-derived, and the paragraph below
        // is why that is acceptable here. Where a local points in its sites
        // moves from `Zero` or `NonZero` to `Unknown` at a join and never back,
        // once per local, and the per-local term below counts that step.
        //
        // **The bit is not monotone in the transfer, and does not have to be.**
        // `Held::clear` puts it back at every fresh assignment. What this
        // number bounds is how often a *block's entry value* can move, and an
        // entry value moves only through `join`, which unions. A transfer that
        // takes facts away inside a block cannot make the entry value descend,
        // so it cannot make the solver oscillate. Generous
        // rather than tight, which is the direction `Analysis::height` says to
        // err in: answering too low stops a correct analysis.
        //
        // **What has been read since the last sequence point is a set of
        // positions, so it is counted in positions rather than in locals.** A
        // block's entry value can gain one entry per place read at one element
        // or terminator of the function, and each entry's set of sites can gain
        // one site per local. The same paragraph above applies to it: the
        // transfer empties it at every marker and only a join makes an entry
        // value grow.
        let positions: usize = function
            .blocks()
            .map(|block| block.elements.len() + 1)
            .sum();

        // **Nothing holds this number.** Measured: answering `locals` instead
        // leaves the whole suite passing, because no function here takes more
        // visits to a block than it has locals, and building one that did
        // would be building a program for the bound rather than for the check.
        // What a wrong answer costs is what that method promises: too low is a
        // panic naming `Analysis::height`, which is a build that stops with
        // something to read rather than a wrong answer about a program.
        locals * locals * 2 + locals * (locals + 8) + positions * (locals + 1)
    }

    fn on_entry(&self) -> Self::Value {
        let mut known = Known {
            points_to: vec![Held::none(self.locals); self.locals],
            // Nothing has allocated into any of these yet, so none of them can
            // say where it came from. A parameter stays this way: its
            // allocation happened somewhere this check cannot see.
            state: vec![SiteState::Live(None); self.locals],
            // Nothing holds a local's address where a function starts, a
            // parameter included: what a caller holds is its own local.
            escaped: vec![false; self.locals],
            // Nothing has been read yet, so there is nothing a free could be
            // unordered against.
            pending: BTreeMap::new(),
        };

        // A parameter holds whatever the caller passed, which is a thing this
        // function can free and did not make. The module comment says why one
        // has to be a site of its own.
        //
        // At its start, because the site stands for whatever the caller passed
        // and not for an allocation behind it. ADR-0036 says why a free of
        // `p + 1` is still proved on that reading.
        for &parameter in &self.parameters {
            known.points_to[parameter.index()].hold(parameter.index(), Offset::Zero);
        }

        known
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        // **Every field named, never `..`.** A lattice value whose join
        // forgets a field reaches a fixpoint over a value nobody is joining,
        // and nothing else in the build says so: the field is read, the walk
        // ends, and the answer is wrong on exactly the programs a join is for.
        // This is RK-018 in the review knowledge bank one type over, where
        // `..` let a field walk past a match that was otherwise exhaustive.
        let Known {
            points_to,
            state,
            escaped,
            pending,
        } = into;

        for (here, there) in points_to.iter_mut().zip(&from.points_to) {
            here.joined(there);
        }

        for (here, there) in state.iter_mut().zip(&from.state) {
            *here = here.joined(*there);
        }

        // A local whose address escaped on one arm has escaped where the arms
        // meet: the other arm did not un-take it.
        for (here, there) in escaped.iter_mut().zip(&from.escaped) {
            *here = *here || *there;
        }

        // **A union, because a read on either arm is a read some execution
        // performed.** What this costs when it is wrong is a report about a
        // read that did not happen, which is row 4; the other direction loses
        // the read that did. The arms themselves do not meet before their
        // join, which is what keeps a read on one arm from being reported
        // against a free on the other: each arm is walked from the value that
        // reached it and not from this one.
        for (key, entry) in &from.pending {
            pending
                .entry(key.clone())
                .or_insert(PendingRead {
                    at: entry.at,
                    sites: BTreeSet::new(),
                })
                .sites
                .extend(&entry.sites);
        }

        // And applying it, which the union alone does not do. [`Known::settle`]
        // says why a join needs this and the assignments do not cover it.
        into.settle();
    }

    fn element(&self, function: &Function, element: &Element, value: &mut Self::Value) {
        // What is read here is read where this element runs, against what held
        // before it, which is the same question `findings` asks one line
        // earlier and has to get the same answer to.
        //
        // **Before the arms, so that no arm's early return can skip it, and
        // nothing observes that today.** Measured: moving it below the match
        // changes no answer, because the one arm that returns early is the
        // write through a pointer, and this frontend reads an assignment's
        // value back into a temporary, so the read is recorded by that element
        // instead. That is a property of one lowering rather than of the IR,
        // and RK-051 in the review knowledge bank is a rule skipped by exactly
        // such a return. Written first because the order is free and the
        // alternative is guarded by nothing.
        value.met(dereferenced_in_element(element));

        // Every field written out, never `..`: RK-018 in the review knowledge
        // bank is a field added to a variant that already exists walking past
        // an exhaustive match.
        match element {
            // Evaluating a place writes nowhere, so no local changes what it
            // holds and no site changes what is known about it. What it reads
            // is not nothing, and `Known::met` above has already taken it: this
            // element exists so that `*p;` on its own can be seen at all.
            Element::Evaluate {
                place: _,
                origin: _,
            } => {}
            // **Every free reaching here is now ordered before everything
            // that follows.** No value moves, so no local's set changes; what
            // changes is that a free this check was holding open can be acted
            // on. See ADR-0022.
            Element::Sequenced { origin: _ } => {
                for state in &mut value.state {
                    if let SiteState::Freed { freed, .. } = state {
                        freed.sequenced = true;
                    }
                }
                // The same fact about a free of a may-set, which ADR-0020
                // records on the local that named the set rather than on its
                // members. One rule, applied to both places a free is written.
                for held in &mut value.points_to {
                    if let Some(freed) = &mut held.freed {
                        freed.sequenced = true;
                    }
                }
                // **And every read behind this is now ordered before whatever
                // frees ahead of it**, so there is nothing left for a later
                // free to be unordered against. The same element answering both
                // directions is the point: one marker, one meaning, read from
                // each side. See ADR-0023.
                value.pending.clear();
            }
            // **Every read behind this is ordered before the call that
            // follows**, which is the half of the marker above that this one
            // says. C17 6.5.2.2 p10's first sentence orders a call's arguments
            // before the call unconditionally, so `free(p + *p)` is a program C
            // defines and was reported until this element existed.
            //
            // The other half is left alone, and the element's own doc comment
            // is where the reason is: concluding it here proved a use after
            // free about `g((free(p), 0), *p)`, whose two arguments C leaves
            // unsequenced. See ADR-0026.
            Element::ArgumentsEvaluated { origin: _ } => value.pending.clear(),
            Element::Assign(operation) => {
                // **This check follows a write through exactly one `Deref` and
                // nothing deeper.** The edge recorded at `Rvalue::Address` is
                // one step, and reading it as two would be inventing the
                // second. `**ppp = q` is therefore a write this check cannot
                // follow at all, and so is a write through any other
                // projection.
                let one_step = operation.place.projection.as_slice() == [Projection::Deref];
                let pointer = operation.place.local.index();
                let targets = if one_step {
                    value.written_through(operation.place.local)
                } else {
                    Vec::new()
                };

                // **Whether this write lands in one local and nowhere else.**
                // ADR-0028's condition, read here and at the replacement
                // further down: one is about what the write may have reached
                // *besides* its target and the other about what it does to
                // that target, and the two have to be the same question.
                // Spelled twice they are one rule in two places, which is
                // RK-052 in the review knowledge bank.
                //
                // A deeper projection is never certain, and that is the
                // condition above rather than an extra clause: `written_through`
                // answers about the local the place starts at, so for `**ppp`
                // it answers about `*ppp` and names the wrong thing.
                let certain = one_step
                    && targets.len() == 1
                    && !value.points_to[pointer].writes_elsewhere
                    && !value.escaped[pointer];

                // **Any write through a projection that is not the certain one
                // may have landed in an escaped local.** Not only the one this
                // arm can follow: a write it cannot follow at all is the case
                // that needs this most, and keying the rule on the shape the
                // arm below reads left `**ppp = q` saying nothing whatever
                // while its own corpus case, spelled with a temporary, was
                // answered. Two review lenses found that independently. See
                // ADR-0031.
                if !operation.place.projection.is_empty() && !certain {
                    replaced_by(self.unit, function, &operation.place, value);
                }

                if one_step {
                    if targets.is_empty() {
                        return;
                    }

                    // **What is written, before what it is written into.**
                    // Whether this lands in one certain local or in any of
                    // several possible ones is decided below, once the value
                    // is in hand; a pointer that may point at one local is not
                    // a pointer that must, and telling those apart is
                    // ADR-0028.
                    //
                    // **A `match` rather than an `if let`, so a fifth kind of
                    // rvalue has to answer here too.** Every other reader of
                    // `Rvalue` in this crate is exhaustive and `error[E0004]`
                    // is what asks them; this one was the exception, and what
                    // a missed arm would mean is that a write through a
                    // pointer silently carries nothing, which is a silence
                    // rather than a build error. RK-018 is the same spelling
                    // one type over.
                    let written = match &operation.value {
                        // `*pp = q + 1;` arrives here as a copy, not as the
                        // arithmetic: the lowering puts the addition in a
                        // temporary and copies it out, and the arm above has
                        // already given that temporary `q`'s sites.
                        Rvalue::Use(Operand::Copy(source)) if source.projection.is_empty() => {
                            value.points_to[source.local.index()].clone()
                        }
                        // The arithmetic written straight into the place, which
                        // no C reaches for the reason above and another
                        // frontend may. The operands the types say contributed,
                        // for the reason the arm above gives, and the same
                        // question about the edge: this asked none of it until
                        // review built the shape by hand, and carried an edge
                        // through `qq + 7` that the arm one level up had just
                        // been taught to drop.
                        Rvalue::Binary { op, lhs, rhs } => {
                            let mut reached = built_from(*op, [lhs, rhs], value, |local| {
                                self.is_pointer(function, local)
                            });
                            reached.writes_to.fill(false);
                            // Emptying the set says nothing on its own: an
                            // empty set is what a pointer this check never
                            // followed an address into has. Giving up on the
                            // set is saying so. See ADR-0028, and RK-052 for
                            // why this line is written beside its twin below
                            // rather than anywhere else.
                            reached.writes_elsewhere = true;
                            reached
                        }
                        // A constant, a read through a projection, a unary
                        // operator, an address. None is a pointer this check
                        // follows to an allocation, so the target is given
                        // nothing: a write this check cannot follow is not
                        // evidence that the old contents are gone.
                        Rvalue::Use(_) | Rvalue::Unary { .. } | Rvalue::Address(_) => {
                            Held::none(value.points_to.len())
                        }
                    };

                    // **A write this check can be certain about replaces what
                    // the target held.** The set names one local and says it
                    // names all of them, so this write landed in that local
                    // and whatever was there is gone. Union would manufacture
                    // a two-element may-set out of a program that has none,
                    // and ADR-0020 then reads that as real ambiguity and gives
                    // up a proof over it: `int **pp = &p; *pp = q; free(p);
                    // *q = 1;` is a certain use after free and was reported as
                    // a suspicion. See ADR-0028, which is where ADR-0019's
                    // rejected option was taken up once the flag above made
                    // "one target" distinguishable from "at most one target".
                    //
                    // The whole row, as an assignment to the target would
                    // replace it: the sites, what it had lost, the proof about
                    // the set it named and the edge alike. `unproved` is the
                    // one thing the direct assignment does that this does not,
                    // for the reason the paragraph below gives, which is about
                    // the allocation rather than about the local.
                    //
                    // **Both halves of "the edge is all of it" are read here.**
                    // The flag above lives in `Held`, so an assignment to the
                    // pointer destroys it, and that is right for the assignment
                    // itself and wrong for an escape: `int ***ppp = &pp; pp =
                    // &p; opaque(ppp); *pp = q;` gave `pp` a fresh row after
                    // something had already taken its address, and the
                    // replacement fired on an edge anybody could have
                    // overwritten since. `Known::escaped` is the half that
                    // outlives an assignment, which is ADR-0018's rule for
                    // which struct a fact belongs in, and it is why the answer
                    // cannot be recorded on the local whose address is taken.
                    // See ADR-0028 and RK-061.
                    if certain {
                        value.points_to[targets[0]] = written;
                        return;
                    }

                    // **The union and nothing else.** A write through an alias
                    // does not unprove what the target held, which #155's rule
                    // for a direct assignment would suggest it should. The
                    // difference is whose fact is at stake: `unproved` writes
                    // on the *sites*, which are shared, so doing it here wiped
                    // a `Freed` that a second local holding the same allocation
                    // had proved. `free(p); *pp = 0; *p = 1;` went from a
                    // proved use after free to a suspicion, and exit 1 to exit
                    // 0, with the write carrying nothing at all.
                    //
                    // Nothing is lost by leaving it out. The target's address
                    // was taken, so ADR-0017 answers `Reached::Lost` for it
                    // wherever a report is made, and `Known::settle` applies
                    // the heap half over the merged value at every join. See
                    // ADR-0019.
                    for target in targets {
                        value.points_to[target].accumulated(&written);
                        // **The proof goes, whatever the accumulator would
                        // say about it.** [`Held::accumulated`] keeps it while
                        // the set does not grow, which is right where an
                        // expression is built from its operands and wrong
                        // here: this write may have replaced the pointer, and
                        // then freeing the target again is not a second free
                        // of anything. The sites stay because keeping them is
                        // the conservative direction for a use after free; the
                        // proof goes because keeping *it* is the confident
                        // one. See ADR-0024.
                        //
                        // **No program observes this, and it is here anyway.**
                        // `writes_to` is written only where an address is
                        // taken, so a target of a write through a pointer has
                        // always escaped, and ADR-0017 answers `Reached::Lost`
                        // for an escaped local wherever a report is made: the
                        // answer is `Unknown` whatever this field says.
                        // Measured, and the narrow claim is the true one:
                        // removing this line breaks nothing today. What it
                        // would cost if the escape stopped covering it is a
                        // proof about a pointer this write may have replaced.
                        value.points_to[target].freed = None;
                    }

                    return;
                }

                // A write through any other projection changes what a pointer
                // points at rather than which allocation a local holds, and
                // this check follows locals.
                if !operation.place.projection.is_empty() {
                    return;
                }

                let destination = operation.place.local;
                match &operation.value {
                    // **Not an optimisation.** `int *p = malloc(4);` lowers to
                    // a call into a temporary and a copy out of it, so without
                    // this the allocation never leaves the temporary and the
                    // headline program is not an error at all.
                    Rvalue::Use(Operand::Copy(source)) if source.projection.is_empty() => {
                        value.points_to[destination.index()] =
                            value.points_to[source.local.index()].clone();
                    }
                    // **Arithmetic on a pointer is followed.** C17 6.5.6 p8
                    // keeps the result inside the object the operand points
                    // into, so `p + 1` may hold whatever `p` holds, and `p[i]`
                    // is that addition: 6.5.2.1 p2 defines `E1[E2]` as
                    // `(*((E1)+(E2)))`. This arm used to clear the destination
                    // instead, on the stated ground that nothing read the
                    // precision yet. Something does now, and until it did the
                    // cost was invisible: `free(p); p[i] = 42;` was silence
                    // rather than a diagnostic, which is the worst answer this
                    // compiler has.
                    //
                    // **The pointer operand, and the integer beside it is not
                    // one.** Which is which is a question about types and
                    // [`built_from`] reads them: 6.5.6 p8 keeps the result
                    // inside the object the *pointer* points into, so a site
                    // the index happens to be is not a site the subscript may
                    // reach. A parameter is a site, so until this was read
                    // `p[i]` unioned the allocation `p` holds with the site `i`
                    // is, and a live site stops the result being proved:
                    // `free(p); p[i] = 42;` was a warning where
                    // `free(p); p[0] = 42;` was an error. See ADR-0030.
                    //
                    // Read before the write, so `p = p + 1` keeps what `p`
                    // held rather than clearing it and unioning the result.
                    Rvalue::Binary { op, lhs, rhs } => {
                        let mut reached = built_from(*op, [lhs, rhs], value, |local| {
                            self.is_pointer(function, local)
                        });
                        // **The sites travel and the edge does not.** C17 6.5.6
                        // p8 keeps the result inside the object the operand
                        // points into, which is why the allocation comes along.
                        // A local's address plus one is not that local, so
                        // `*(pp + 1) = q;` must not be read as a write to what
                        // `pp` points at: it is an out of bounds write, and
                        // following it here reported a proved use after free
                        // about an allocation nothing had freed. See ADR-0019.
                        //
                        // Plus zero never arrives: the lowering folds it, so the
                        // two spellings of one C expression are one shape before
                        // anything reads them. See ADR-0021.
                        reached.writes_to.fill(false);
                        // The twin of the line in the `Deref` arm above, and
                        // for the same reason: an emptied set is a set this
                        // check knows nothing about. See ADR-0028.
                        reached.writes_elsewhere = true;
                        value.points_to[destination.index()] = reached;
                    }
                    // A constant, a read through a projection, or a unary
                    // operator. None of the three is a pointer this check can
                    // follow: C17 6.5.3.3 gives unary `+`, `-` and `~`
                    // arithmetic operands only, and `!` yields an `int`.
                    Rvalue::Use(_) | Rvalue::Unary { .. } => {
                        value.clear(destination);
                    }
                    // The address of a place is not an allocation this check
                    // follows: nothing here frees a local's own storage, which
                    // is `StorageDead` and is the lifetime phase's.
                    //
                    // **But the place whose address is taken stops being
                    // something anything here proved.** A write through the
                    // pointer that just escaped can put a different allocation
                    // in it, and this check does not follow what a pointer
                    // points at, so it would not see the write. `Rvalue::Address`
                    // is the only way a local's address is taken in this IR, so
                    // this is the one door, and leaving it open is what made
                    // `int **pp = &p; *pp = q; free(q); free(p);` silent about
                    // a double free.
                    Rvalue::Address(taken) => {
                        value.clear(destination);
                        // **The edge and the bit, and both are needed.** The
                        // bit outlives everything done to this destination and
                        // is what keeps an escaped local unproved; the edge
                        // dies with the destination and is what lets a write
                        // through it be followed. See ADR-0019.
                        value.points_to[destination.index()].writes_to[taken.local.index()] = true;
                        // **And the set is all of it, where the address is of
                        // the local itself.** `Held::clear` ran a line above,
                        // so this destination points at exactly this local,
                        // which is what lets a write through it replace rather
                        // than union. See ADR-0028.
                        //
                        // `&*pp` is not that. C17 6.5.3.2 p3 makes it `pp`, so
                        // the place it names is what `pp` points at and not
                        // `pp`, while the edge above can only name a local. The
                        // union is the right answer to an edge that names the
                        // wrong thing and a replacement is not, so the flag
                        // stays set and the write stays a may-write. No C
                        // reaches this: the lowering applies the same clause and
                        // folds `&*pp` to a copy of `pp`. Another frontend need
                        // not, which is `docs/c-family.md`'s reason for the IR
                        // expressing the shape at all.
                        value.points_to[destination.index()].writes_elsewhere =
                            !taken.projection.is_empty();
                        value.escaped[taken.local.index()] = true;
                        value.unproved(taken.local.index());
                    }
                }

                // **After the match rather than inside it**, so that every arm
                // is covered and the next one is covered before it is written.
                // Placed per arm, this was two calls and a hole: `p = r + i;`
                // lowers to arithmetic into a temporary and a copy out of it,
                // so the arm that looks like the arithmetic case is reached
                // through the copy, and a mutation of the arithmetic arm broke
                // nothing at all. One call cannot be put in the wrong place.
                value.unproved(destination.index());
            }
            // Storage beginning or ending says nothing about what the local
            // held before, and what it holds now is nothing.
            Element::StorageLive { local, origin: _ } => value.clear(*local),
            Element::StorageDead { origin: _, local } => value.clear(*local),
        }
    }

    fn terminator(&self, _function: &Function, terminator: &Terminator, value: &mut Self::Value) {
        // Before the `let ... else` below, which returns for every terminator
        // that is not a call. A `Terminator::Branch` reads its condition and a
        // condition that is exactly a place never becomes an element, so
        // leaving it to the arm that handles calls would be silent about
        // `(*p ? 1 : 0) + (free(p), 0)`.
        value.met(dereferenced_in_terminator(terminator));

        let Terminator::Call {
            callee,
            arguments,
            destination,
            then: _,
            origin,
        } = terminator
        else {
            // Nothing else moves an allocation. Written out rather than `_`
            // for RK-018's reason: a terminator added later has to be answered
            // for here.
            match terminator {
                Terminator::Goto(_)
                | Terminator::Branch { .. }
                | Terminator::Return
                | Terminator::Abnormal { .. } => return,
                Terminator::Call { .. } => unreachable!("the let above took it"),
            }
        };

        // What the call does to what it was handed, before what it leaves
        // behind, which is the order the two happen in.
        // A `Reached::Lost` moves nothing, because there is nothing to move:
        // what it says is that this call touched something the check was not
        // following, which is a fact about the report rather than about the
        // lattice.
        let touched = Self::touching(arguments.iter(), value);
        let sites = || named(&touched);

        match self.callee(*callee) {
            Callee::Frees => {
                let reached: Vec<usize> = sites().collect();

                // **A free of a local that may hold something this check
                // cannot name does not say which allocation went.** The sites
                // below are what the local is *thought* to hold, and a call
                // this check cannot read may have written a fresh pointer over
                // it since; writing `Freed` on them would be a proof about an
                // allocation the callee may have swapped out. The site is
                // shared, so that proof is handed to every other local holding
                // it, which is how a program C defines became an `error` no
                // flag suppresses. See ADR-0029.
                //
                // **It supersedes both rules below and skips nothing else.**
                // Neither of them applies once the set is not known to be what
                // was freed: the one is about which member of a set went, and
                // the other about a single member going. RK-051 is an early
                // return that left the conservative rules behind it unrun, and
                // the rule here is the conservative one.
                //
                // **What it does skip is the destination.** The fall-through
                // path clears the local `free` is written into and this does
                // not, which the `reached.len() > 1` branch below does too.
                // That local is `void` and holds none of this, which is #135;
                // measured, putting the clear back inside this branch leaves
                // the whole workspace green.
                if Self::holds_something_unnameable(arguments, value) {
                    for site in reached {
                        // **A free cannot un-free an allocation.** A site an
                        // earlier free this check *could* follow has already
                        // proved is not something this call has anything to
                        // say about, and the site is shared, so blanking it
                        // takes the proof away from every other local holding
                        // it. Found by review;
                        // `a_free_this_check_could_not_follow_leaves_a_proved_free_alone`
                        // is the program and is the guard.
                        if !matches!(value.state[site], SiteState::Freed { .. }) {
                            value.state[site] = SiteState::Unknown;
                        }
                    }

                    return;
                }

                // **A may-set is not a must-set, and this is where the two used
                // to be confused.** Freeing a local that may hold either of two
                // allocations frees exactly one of them; writing `Freed` on
                // both claimed each was certainly freed, and a later free of
                // one of them by name was then a proved double free about a
                // program that frees each exactly once on one of its paths.
                //
                // So nothing is written on the members. They stop being
                // provable, and the fact that one of them went is recorded on
                // the local that named the set, which is the only thing in this
                // lattice that names a set. See ADR-0020.
                if reached.len() > 1 {
                    for site in &reached {
                        value.state[*site] = SiteState::Unknown;
                    }

                    for argument in arguments {
                        // A constant frees nothing and a projection names a
                        // place this check does not follow, which is what
                        // `Allocations::touching` already said about both.
                        //
                        // **No program reaches the second of those, and it is
                        // here anyway.** `free` takes one argument, and an
                        // argument with a projection reaches no site at all,
                        // so this branch is not entered with one. What it
                        // would cost if that changed is a proof recorded
                        // against the pointer rather than against what it
                        // points at, which is the false proof this whole rule
                        // is against.
                        //
                        // Measured, and the narrow claim is the true one:
                        // *removing* the guard breaks nothing, because the
                        // projection here is always empty. Negating it breaks
                        // two named tests, because then nothing is recorded at
                        // all.
                        let Operand::Copy(place) = argument else {
                            continue;
                        };
                        if place.projection.is_empty() {
                            value.points_to[place.local.index()].freed =
                                Some(Freeing::new(origin.span()));
                        }
                    }

                    return;
                }

                for site in reached {
                    // Whatever the site was known to have come from survives
                    // the free: the diagnostic wants to name it.
                    let made = match value.state[site] {
                        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                        SiteState::Unknown => None,
                    };
                    // **A free already ordered before here is the one to
                    // keep.** It is what proves anything about what follows,
                    // and it is what the diagnostic has to point at for the
                    // proof to be readable: `free(p); x = (free(p), 0) + *p;`
                    // is a proved use after free because of the first line,
                    // and replacing it with the second would name a free that
                    // is in the same unsequenced expression as the use and
                    // leave the reader with two carets that prove nothing.
                    // Without this the second free took the proof away with
                    // it, which review measured. See ADR-0022.
                    //
                    // It is also the earliest, which is what this variant's
                    // own doc comment has always said it holds.
                    let freed = match value.state[site] {
                        SiteState::Freed { freed, .. } if freed.sequenced => freed,
                        _ => Freeing::new(origin.span()),
                    };
                    value.state[site] = SiteState::Freed { made, freed };
                }
            }
            // It does not free what it is passed, which is the whole of why the
            // name is read.
            Callee::Allocates => {}
            Callee::Opaque => {
                for site in sites().collect::<Vec<_>>() {
                    value.state[site] = SiteState::Unknown;
                }

                // **What it was handed is not all it can reach.** The loop
                // above is about the allocations the arguments name; this is
                // about the *locals* whose addresses are out there, which this
                // call may write a fresh pointer into whether or not it was
                // passed one.
                //
                // **`Callee::Frees` and `Callee::Allocates` are not here**, and
                // what says so is what each is handed rather than a sentence
                // forbidding the write: C17 7.22.3.4 gives `malloc` a size and
                // no address at all, and 7.22.3.3 gives `free` the pointer's
                // *value*, which p2 of that subclause requires to be one an
                // allocation function returned and which 7.22.3 requires to be
                // disjoint from every other object. Neither is ever handed
                // `&p`. That is the whole reason the name is read.
                //
                // **No test holds this**: marking at either arm leaves the
                // whole workspace green, measured. The record says so rather
                // than leaving the next reader to find out by widening it.
                // See ADR-0029.
                //
                // **Every local, because a callee's write has no type this
                // function knows.** The other producer of this fact narrows by
                // the type it writes, which is ADR-0031; a callee's body is not
                // read, so there is nothing here to narrow by. Narrowing this
                // one would need what #134's annotation says.
                value.replaced(|_| true);
            }
        }

        let Some(place) = destination else {
            return;
        };
        if !place.projection.is_empty() {
            return;
        }

        if self.callee(*callee) == Callee::Frees {
            // `free` returns nothing. The local the lowering writes it into is
            // `void` and holds none of this, which is #135.
            value.clear(place.local);
            return;
        }

        // A call leaves behind something this function did not have before, and
        // the local it landed in is what names it. **Live rather than joined
        // with what was there**: a second turn of a loop through the same call
        // is a second allocation, and carrying the first one's `Freed` across
        // would report a double free for code that allocates each time round.
        value.clear(place.local);
        let site = place.local.index();
        // At its start: the site is whatever the call handed back, so the
        // value is that value and not an offset into it.
        value.points_to[site].hold(site, Offset::Zero);
        // **The span only where this check saw an allocation.** Every call's
        // destination is a site, because a call this cannot read may hand back
        // anything and a site is how that is tracked. But `allocated here` is a
        // claim, and `void *p = bar();` gives no evidence that `bar` allocated
        // anything. Naming that line was a caret asserting something nothing
        // had established, so a site whose call is not `malloc` is `Live(None)`
        // and the diagnostic leaves the label off.
        let made = (self.callee(*callee) == Callee::Allocates).then(|| origin.span());
        value.reborn(site, made);
        // After the state, because this is what takes it away again. No C
        // reaches here with an escaped destination: the lowering writes every
        // call into a fresh temporary and copies it out, so the copy above is
        // what a C program goes through. Another frontend need not, and
        // `a_call_into_a_local_whose_address_escaped` builds the shape by hand.
        value.unproved(site);
    }
}

/// Which of the three things this check answers about a finding is.
///
/// One walk over one lattice, so this is not three checks and
/// `docs/diagnostics.md` says so where it hands them their codes. What differs
/// is the question: the codes and the words are different, and the thing a
/// caret lands on is a call in two of them and a dereference in the other.

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
}

/// One thing this check concluded, and where.
///
/// Not a diagnostic: this crate cannot see one. What each conclusion costs a
/// build is `Diagnostic::concluded`'s in `safec`, which is the one place that
/// answers it, and ADR-0001 is why there is only one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Which of the two this is.
    pub kind: Kind,
    /// What that check concluded.
    pub conclusion: Conclusion,
    /// Where a caret goes: the call that frees, or the element that reads or
    /// writes through a freed pointer.
    ///
    /// Not the place's own span, which a [`Place`] does not have: the element's
    /// or the terminator's.
    pub at: Span,
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

/// Why a finding could not be proven.
///
/// **Four reasons and not two, because one of them is about this check and
/// the others are about the program.** A reader told a value may have been
/// freed already is being told something was established somewhere; where this
/// check lost the pointer, nothing was, and saying it anyway is a claim about
/// a program that nobody worked out. Which words each reason gets is
/// `memory_finding`'s in `safec`, for the reason [`Finding`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unproven {
    /// The paths or the sites reaching here disagree about a site that really
    /// was freed, or a call this check cannot read was handed one and may have
    /// freed it. There is a free to suspect, and no single one to point at.
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
    Lost,
    /// C has not said which order runs. See ADR-0022.
    Unsequenced,
    /// A pointer this check followed was moved by something it cannot
    /// evaluate, so whether it is still at the start of its allocation is not
    /// known. Only [`Kind::InteriorFree`] carries it. See ADR-0036.
    Offset,
}

/// Every double free this unit contains, and every one it cannot rule out.
///
/// One walk per function: the fixpoint answers what holds where each block
/// starts, and this replays each block from there to find the calls to report.
/// The replay rather than a second lattice, because the transfer is what
/// decides which sites a call touches and having two answers to that is having
/// one of them be wrong.
pub fn findings(sources: &SourceMap, unit: &TranslationUnit) -> Vec<Finding> {
    let mut findings = Vec::new();

    for id in unit.functions() {
        let function = unit.function(id);
        // A declaration has no blocks, and `Function::blocks` panics rather
        // than answering for one.
        if !function.is_defined() {
            continue;
        }

        let analysis = Allocations {
            sources,
            unit,
            locals: function.locals().len(),
            parameters: function.parameters().collect(),
        };
        let cfg = Cfg::of(function);
        let solution = solve(&analysis, function, &cfg);
        // The other check's answer, asked at each block's terminator, which is
        // where a free is reached. A free of a pointer that check established
        // is null frees nothing, and `asked` is the one place that is applied.
        // See ADR-0027.
        let null = nullability::null_at_terminators(unit, function, &cfg);

        // Per function, because a span belongs to one of them. The third
        // field is where in `findings` the report standing at that caret is,
        // so that a proof arriving later can replace a suspicion: `findings`
        // is appended to and assigned into, never removed from or reordered,
        // until the sort at the end of this function.
        let mut said: Vec<(Span, Place, usize)> = Vec::new();

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
            for element in &block.elements {
                // Before the transfer, which is what the element does: the
                // question is what was true where it runs.
                used(
                    &mut findings,
                    &mut said,
                    dereferenced_in_element(element),
                    &known,
                );
                analysis.element(function, element, &mut known);
            }

            // Before the terminator's own transfer, which is what turns a live
            // allocation into a freed one: the question is what was true when
            // the call was reached.
            used(
                &mut findings,
                &mut said,
                dereferenced_in_terminator(&block.terminator),
                &known,
            );
            findings.extend(reported(&analysis, &block.terminator, &known, &null, id));
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
    findings.sort_by_key(|finding| (finding.at.file().index(), finding.at.start()));

    findings
}

/// What a set of sites says about whatever touched them.
///
/// The fold from many sites to one conclusion, and the one place either kind
/// of finding makes it. RK-035 in the review knowledge bank is why there is
/// one: a
/// may-analysis's join is forced to be right by the lattice, and the code that
/// reads the answer is where the same rule gets lost.
struct Verdict {
    conclusion: Conclusion,
    /// The earliest free reaching here, where there is one to name.
    freed: Option<Span>,
    /// Where that allocation came from, where this check saw it happen.
    made: Option<Span>,
    /// Why this could not be proven, and `None` where it was. Carried through
    /// to [`Finding::unproven`] unchanged.
    unproven: Option<Unproven>,
}

/// What these sites amount to, or nothing where they amount to no report.
///
/// The caller decides what reaching nothing means, by what it puts in
/// `reached`: a free hands a [`Reached::Lost`] for an argument it stopped
/// following, and a dereference hands an empty iterator. That asymmetry is the
/// design rather than an accident, and [`used`] says why.
fn verdict(
    kind: Kind,
    reached: impl IntoIterator<Item = Reached>,
    known: &Known,
) -> Option<Verdict> {
    // The earliest free reaching here, and whether C has sequenced **every**
    // free that reaches here. [`Freeing::joined`] is both rules, because
    // folding over the sites reached at one point wants exactly what folding
    // over two paths wants: the earlier span, and the conjunction.
    let mut earliest: Option<Freeing> = None;
    // Where the allocation came from, kept only while every freed site agrees.
    // **RK-035 one level down**: proving a double free from one of several
    // sites is the may-set mistake the fold below is written to avoid, and
    // naming one of several allocations as *the* one is the same mistake about
    // a label. `if (c) p = malloc(); else p = malloc();` reaches both, and
    // pointing at either would be a caret on an allocation the value may not
    // hold.
    let mut made: Option<Span> = None;
    let mut any_freed = false;
    let mut live = false;
    let mut unknown = false;
    // Kept apart from `unknown` because they are unproven for opposite
    // reasons, and the words a reader is given turn on which. A site the paths
    // disagree about was freed on one of them, and a site an opaque call was
    // handed may have been freed by it; a pointer this check lost says nothing
    // about any free anywhere. Counting both as one flag is what put
    // `may free it again here` on a program with one free in it.
    let mut lost = false;
    // Whether more than one free was folded in. The span below is the earliest
    // of them and the flag beside it is the conjunction, so where they differ
    // the span can be a free this check *has* seen sequenced while the flag is
    // false because another was not. Saying "the order is what is open" about
    // that pair would point two carets at two evaluations C does order.
    let mut several = false;

    for entry in reached {
        let site = match entry {
            Reached::Site(site) => site,
            // A proof about the set: freeing it again takes the same member,
            // whichever it was. It carries no `made`, because naming one of
            // several allocations as the one that was freed is the may-set
            // mistake this whole rule is against.
            Reached::SetFreed(freed) => {
                several |= any_freed;
                any_freed = true;
                earliest = Some(match earliest {
                    Some(already) => already.joined(freed),
                    None => freed,
                });
                continue;
            }
            // **Not the same as proving it live.** This is the check having
            // lost the pointer, and answering nothing about it is the failure
            // `docs/safety-model.md` is written to prevent rather than the one
            // it tolerates.
            Reached::Lost => {
                lost = true;
                continue;
            }
        };

        match known.state[site] {
            SiteState::Live(_) => live = true,
            SiteState::Freed {
                made: from,
                freed: before,
            } => {
                made = if any_freed { same(made, from) } else { from };
                several |= any_freed;
                any_freed = true;
                earliest = Some(match earliest {
                    Some(already) => already.joined(before),
                    None => before,
                });
            }
            SiteState::Unknown => unknown = true,
        }
    }

    // **`points_to` is a may-set, so one freed site among several is not a
    // proof.** `SiteState::joined` already answers `Unknown` where one path
    // freed a site and another did not; this is the same question across two
    // sites rather than across two paths, and answering it differently let the
    // spelling of a program decide whether it was a warning or an error. A
    // proof needs every site reached to have been freed, and nothing about it
    // to have been lost.
    // Every site reached was freed and nothing about them was lost. That is
    // the whole of what this check can work out from the states; whether C has
    // put the free first is a separate question with a separate answer, and
    // running them together is RK-034's shape: one test meaning "proved" and
    // "gave up" at once reports the second as the first.
    let settled = !live && !unknown && !lost;

    // **A double free does not turn on which ran first.** Two frees of one
    // allocation are a double free in either order, so there is nothing for a
    // sequence point to settle and asking for one turned `(free(p), 0) +
    // (free(p), 0)` from an error into a warning. Issue #142 said so in as many
    // words and this check did it anyway until review measured it. A use is the
    // other way round: one allowed order reads freed storage and another does
    // not, which is the whole of what this field is for.
    let ordered = match kind {
        Kind::DoubleFree => true,
        Kind::UseAfterFree => earliest.is_some_and(|freed| freed.sequenced),
        // Never asked: `interior` answers that one without folding frees at
        // all. `true` because a pointer that is not the start of an
        // allocation is the wrong thing to hand `free` whatever ran first.
        Kind::InteriorFree => true,
    };

    match earliest {
        // **A null nothing here established does not weaken this.** The site a
        // parameter stands for carries no allocation of its own, so asking for one
        // before answering `Unsafe` would drop the proof on the commonest double
        // free there is, and C17 7.22.3.3 p2 exempts a failed allocation by the
        // same sentence it exempts a null caller passes. What this asserts is that
        // some execution of this function is undefined, which is ADR-0027.
        Some(freed) if settled && ordered => Some(Verdict {
            conclusion: Conclusion::Unsafe,
            freed: Some(freed.at),
            made,
            unproven: None,
        }),
        // **Unproven, and the free is still named.** What is open here is only
        // the order: the sites agree, nothing was lost, and there is exactly
        // one free to point at. A reader given two carets and the note beside
        // them can see the shape of it. See ADR-0022.
        Some(freed) if settled && !several => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: Some(freed.at),
            made,
            unproven: Some(Unproven::Unsequenced),
        }),
        // Neither span is carried. What makes this unproven is that the sites
        // or the paths disagree, so there is no one free that every execution
        // reaching here went through, and no one allocation to name beside it.
        Some(_) => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Disagreement),
        }),
        // **Nothing established a free at all**, so nothing here may say one
        // happened. This check lost the pointer and no site it reached says
        // otherwise, which is what `!unknown` is doing: a site the paths
        // disagree about, and a site an opaque call was handed, are each a
        // free worth suspecting, and the arm below is right about them.
        None if lost && !unknown => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Lost),
        }),
        None if unknown => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Disagreement),
        }),
        None => None,
    }
}

/// What this terminator is worth reporting as a free, if anything.
///
/// One finding per call and question rather than one per site: a local may
/// point at several allocations where a branch put them there, and two carets
/// on one `free` say one thing twice. **Two questions, though, and one caret
/// can carry both**: `int *q = p + 1; free(q); free(p);` frees one allocation
/// twice and the first time through a pointer that is not its start.
///
/// The double free first, so that a caret carrying both reads `SC0401` above
/// `SC0404`: the sort at the end of [`findings`] is stable and the two share a
/// span. See ADR-0036.
fn reported(
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
    null: &NullAtTerminators,
    block: BlockId,
) -> Vec<Finding> {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return Vec::new();
    };

    if analysis.callee(*callee) != Callee::Frees {
        return Vec::new();
    }

    // **One answer to what the arguments reached feeds both questions**,
    // because the two have to agree about it, and two walks deciding it is
    // RK-052's shape. The offset is read beside it from the same `asked`, so an
    // argument established null is left out of both or of neither.
    let reached = Allocations::touching(asked(arguments, null, block), known);
    let offset = asked(arguments, null, block)
        .filter_map(|argument| match argument {
            Operand::Copy(place) if place.projection.is_empty() => {
                Some(known.points_to[place.local.index()].offset)
            }
            // A constant frees nothing and a projection is already a
            // `Reached::Lost` above, which `interior` answers nothing for.
            Operand::Copy(_) | Operand::Constant(_) => None,
        })
        .reduce(Offset::joined)
        .unwrap_or(Offset::Zero);

    let finding = |kind, verdict: Verdict| Finding {
        kind,
        conclusion: verdict.conclusion,
        at: origin.span(),
        freed: verdict.freed,
        made: verdict.made,
        unproven: verdict.unproven,
    };

    // Asked first because `verdict` takes what was reached by value; pushed
    // second, for the reason above.
    let inside = interior(&reached, offset, known);

    let mut found = Vec::new();
    if let Some(verdict) = verdict(Kind::DoubleFree, reached, known) {
        found.push(finding(Kind::DoubleFree, verdict));
    }
    if let Some(verdict) = inside {
        found.push(finding(Kind::InteriorFree, verdict));
    }
    found
}

/// Whether a free hands `free` something other than the start of what it
/// reached.
///
/// C17 7.22.3.3 p2 makes a `free` of anything but a pointer an allocation
/// function returned undefined, and `p + 1` reaches the allocation `p` does,
/// so [`verdict`] cannot see this: the sites are the same and so are their
/// states. What differs is [`Held::offset`].
///
/// Nothing, where any of these holds:
///
/// * a [`Reached::Lost`] is among them. ADR-0017 makes [`Known::reached_by`]
///   answer it for an escaped local that holds sites, so `int **pp = &p; *pp =
///   p + 1; free(p);` never reaches the offset at all, and neither does
///   anything a call this check cannot read may have written. The offset is a
///   positive claim about a local and the address-taken set is what stops it
///   being believed, which is RK-061. **That rule is what covers this line**,
///   and the day something narrows what an escaped local is reported as, this
///   line stops being covered: RK-064.
/// * a [`Reached::SetFreed`] is. The sites are gone from `reached` by then, so
///   there is nothing left to be an offset into.
/// * no site at all. A local that reaches nothing is one this check never
///   followed, and [`verdict`] already answers [`Unproven::Lost`] for the same
///   call.
///
/// **A parameter is proved on, and that is sound.** `void f(int *p) { free(p +
/// 1); }` can only free the start of an object if a caller passed a `p` one
/// element before the start of one, which C17 6.5.6 p8 already makes
/// undefined, so every conforming caller leaves this call undefined. ADR-0027
/// is the record that says a conclusion is about an execution this function
/// has.
///
/// `freed` is `None` whatever the answer: what this reports is not about a free
/// that already happened.
fn interior(reached: &[Reached], offset: Offset, known: &Known) -> Option<Verdict> {
    let mut sites = Vec::new();
    for entry in reached {
        match entry {
            Reached::Site(site) => sites.push(*site),
            // `SetFreed` is answered twice: `Known::reached_by` clears the
            // sites whenever it answers it, so the rule below this loop says
            // the same. Written here so the arm says which rule owns it.
            Reached::SetFreed(_) | Reached::Lost => return None,
        }
    }

    let (&first, rest) = sites.split_first()?;

    // The allocation, where every site agrees about it, which is RK-035: naming
    // one of several as the one freed is a caret on an allocation the value
    // may not hold.
    let made_of = |site: usize| match known.state[site] {
        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
        SiteState::Unknown => None,
    };
    let made = rest
        .iter()
        .fold(made_of(first), |made, &site| same(made, made_of(site)));

    let (conclusion, unproven) = match offset {
        Offset::Zero => return None,
        Offset::NonZero => (Conclusion::Unsafe, None),
        Offset::Unknown => (Conclusion::Unknown, Some(Unproven::Offset)),
    };

    Some(Verdict {
        conclusion,
        freed: None,
        made,
        unproven,
    })
}

/// The arguments of a `free` this report asks about, which is every one except
/// a pointer this compiler established is null.
///
/// C17 7.22.3.3 p2: if the argument is a null pointer, no action occurs. So a
/// call whose only argument is such a pointer frees nothing, and there is
/// nothing here for a double free to be about. `Allocations::touching` already
/// applies this to an argument *written* as a constant, with the clause quoted;
/// this is the same rule reaching a local the nullability check proved. See
/// ADR-0027, which is also why nothing weaker exempts one: a pointer nobody
/// established anything about still weakens no proof.
///
/// **Only the report, never the transfer.** This is called from the walk that
/// reports and `Analysis::terminator` cannot reach it, so a free of a pointer
/// established null still marks its sites freed and still leaves the local
/// holding them. Clearing them instead would make the local reach no site,
/// which is how this check spells having lost a pointer, and the reader would
/// get a warning about the wrong thing. That is the third of ADR-0027's three
/// conditions and it is held by where this function is called from.
///
/// **The result being empty is not [`Reached::Lost`]**, and the two are worth
/// keeping apart here because RK-045 and RK-049 are both about this check
/// having two emptinesses. `verdict` answers `None` for no sites and nothing
/// lost, which is silence, and that is the right answer to a call C says does
/// nothing. What is lost still arrives as a `Reached::Lost` from the arguments
/// that were asked about.
fn asked<'o>(
    arguments: &'o [Operand],
    null: &'o NullAtTerminators,
    block: BlockId,
) -> impl Iterator<Item = &'o Operand> + 'o {
    arguments
        .iter()
        .filter(move |argument| !established_null(argument, null, block))
}

/// Whether this argument is a pointer this compiler established is null where
/// the call runs.
///
/// **A projection answers `false`.** `free(*pp)` asks what a *place* holds, and
/// the nullability lattice is keyed by the local, which its own doc comment
/// says it pays for. Answering anything else here would be reading a claim
/// about `pp` as a claim about what `pp` points at. Mutation: drop the
/// `projection.is_empty()` guard. `a_free_read_out_of_a_pointer_proved_null`
/// loses its `error[SC0401]` and fails, and nothing else in the suite moves.
///
/// A constant answers `false` as well, although `free(0)` is exempt: it is
/// exempt in `Allocations::touching`, where the operand is read, and two rules
/// for one argument is one of them being wrong. **That arm is held by nothing
/// and cannot be**: answering `true` for a constant changes no program,
/// because `touching` has already skipped every one of them before this is
/// asked. It is written out so the arm says which rule owns it.
fn established_null(argument: &Operand, null: &NullAtTerminators, block: BlockId) -> bool {
    match argument {
        Operand::Copy(place) if place.projection.is_empty() => null.established(block, place.local),
        Operand::Copy(_) | Operand::Constant(_) => false,
    }
}

/// Report every dereference at `at` of something that was freed.
///
/// **A place this check follows no allocation for says nothing**, which is the
/// opposite of what a free of one says, and the asymmetry is deliberate. A free
/// acts on an allocation, so freeing something the check stopped following may
/// be a second free. A dereference only reads one, and a pointer with no
/// allocation behind it is an uninitialised pointer or one into storage that is
/// not the heap: different defects, with checks of their own that do not exist
/// yet. Answering `Unknown` here would warn on every `*p` whose pointer came
/// from anywhere this does not follow, which is most of them.
///
/// **What that silence covers is a boundary rather than a rule.** A pointer
/// written behind this check's back arrives here as `SiteState::Unknown`
/// rather than as no site at all, because taking a local's address is what
/// makes its sites unknown, and `Known::escaped` is what keeps them that way
/// for the rest of the function: `int **pp = &p; p = malloc(8); *pp = q;` used
/// to hand `p` a fresh site nothing had lost, and reading through it was exit
/// 0 on a freed pointer. Pointer arithmetic is followed for the same reason,
/// and so is a controlling expression, which needed `Terminator::Branch` to
/// carry a span before it could be.
///
/// A place whose value is thrown away is read too, because
/// [`Element::Evaluate`] exists to say that it was evaluated: `*p;` on its own
/// used to leave no element at all, so there was nothing here to look at.
///
/// **The rule, rather than a list of what falls outside it: a place whose root
/// reaches no site says nothing, however it came to reach none.** Two ways are
/// known, and the third was closed by making the lowering apply C17 6.5.3.2
/// p3, so `int *r = &*p;` now copies the pointer rather than taking an address
/// of what it reaches. A pointer read out of another pointer, `int *p = *pp;`,
/// never had a site. And a bare name is never given an element at all, so
/// `free(p); p;` is quiet about reading an indeterminate pointer, which 6.2.4
/// p2 makes undefined and which belongs to an axis with no check.
/// `docs/diagnostics.md` says what exit 0 does not mean here, because a
/// boundary that lives only in a comment is one no user can find.
///
/// [`Element::Evaluate`]: crate::ir::Element::Evaluate
fn used(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    at: Option<(Span, Vec<&Place>)>,
    known: &Known,
) {
    let Some((at, dereferenced)) = at else {
        return;
    };

    for place in dereferenced {
        // **One place at one span said once.** `*p = 42;` lowers to two
        // operations that both read through `p`, because an assignment is an
        // expression with a value and the lowering reads the place back into a
        // temporary. Both are genuine dereferences of one thing and two carets
        // on one line would say it twice.
        //
        // The place and not the finding. Deduplicating finished findings was
        // wrong in both directions at once, measured: two unproven uses on one
        // line carry no spans at all, so they were field-identical and one was
        // thrown away, while `*p = *q;` produced `p`, `q`, `p` in that order
        // and the pair that should have collapsed was not adjacent for
        // `Vec::dedup` to see. What decides whether two reports are one report
        // is which place was dereferenced, and only this knows it.
        //
        let Some(verdict) = verdict(Kind::UseAfterFree, known.reached_by(place.local), known)
        else {
            continue;
        };

        say(
            findings,
            said,
            place,
            Finding {
                kind: Kind::UseAfterFree,
                conclusion: verdict.conclusion,
                at,
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }
}

/// Put this finding at its caret, or leave the one already standing there.
///
/// **Both halves of the key, and a case for each.** Keying on the span alone
/// collapses `*p = *q;` after two frees into one report, which
/// `two_pointers_used_after_a_free_on_one_line` fails on. Keying on the place
/// alone collapses `*p = 1; *p = 2;` after one free into one, which
/// `one_pointer_used_after_a_free_on_two_lines` fails on. Neither case reaches
/// the other's mutation, which is why there are two.
///
/// **Recorded where the report is made, and not a line earlier.** Marking the
/// place as said when it had only been looked at spent the right to report it:
/// a dereference this check proved live said nothing and registered anyway, so
/// a later one of the same place at the same span was skipped as a repeat of a
/// report that never happened. Two dereferences do share a span, because both
/// operands of a `&&` or a `||` are written into one temporary at the whole
/// expression's span, and `if (*p || (free(p), *p))` was exit 0 with no output:
/// a proved use of a freed value, silent, which is the worst answer
/// `docs/safety-model.md` allows for. A function rather than the tail of
/// [`used`] because [`used_before`] reaches the same caret from the other
/// direction, and two copies of this rule would be two answers to which report
/// stands.
///
/// **A proof replaces the suspicion standing at this caret**, rather than the
/// first report of a pair winning whatever it concluded.
/// `int **q = &p; if (*p || (free(p), *p))` reports the first read as unproven,
/// because taking a local's address is what makes its sites unknown, and the
/// second read is proved. Keeping the first threw the proof away and exited 0,
/// which is the silence the paragraph above describes arriving through the
/// other door. [`supersedes`] is the rule, and says why only this direction
/// replaces anything.
///
/// The index `said` carries, not the position within `said`: `findings` holds
/// what every caret in this function has said, so anything reported between the
/// pair sits between them. Taking the wrong one overwrites a finding nobody was
/// replacing, and `a_proof_replaces_the_suspicion_at_one_caret` puts a double
/// free in front of the pair so that the two indices differ.
fn say(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    place: &Place,
    finding: Finding,
) {
    let standing = said
        .iter()
        .position(|(said_at, said_place, _)| *said_at == finding.at && said_place == place);

    let Some(standing) = standing else {
        said.push((finding.at, place.clone(), findings.len()));
        findings.push(finding);
        return;
    };

    let index = said[standing].2;
    if supersedes(findings[index].conclusion, finding.conclusion) {
        findings[index] = finding;
    }
}

/// Report every read behind this call that it may be about, where nothing
/// orders the two.
///
/// **The half a forward walk cannot see, arriving from the other side.** A read
/// the walk meets before the call is never asked about it, because a call marks
/// only what follows; carrying the read forwards to the call asks the same
/// question at the only point where both are in hand. `Element::Sequenced` is
/// what says a read is behind rather than beside, and clearing
/// [`Known::pending`] is where that happens. See ADR-0023.
///
/// **Two callees ask it, and they are asking about different things.** A
/// `free` took the site away, so the read may have run after the free. A call
/// this check cannot read may have freed what it was handed, which is the same
/// suspicion one step weaker and is what `Allocations::terminator` writes as
/// `SiteState::Unknown` for everything the call touched. Both are the question
/// the forward walk already asks on the other side of the call, so refusing one
/// of them here left `g(*p) + h(p)` silent while `h(p) + g(*p)` reported.
/// What differs is the report: an opaque call has no free to point a second
/// caret at, and saying it had one would be this compiler asserting something
/// it did not establish.
///
/// **Always unproven, and that is the shape rather than a caution.** The read
/// and the call are in one full expression with nothing sequencing them, so one
/// allowed order reads freed storage and another does not, and which an
/// implementation picks is unspecified. There is no program this can be right
/// to call `Unsafe` about, so the worst it can do when it is wrong is a report
/// about a read that was ordered after all.
///
/// **It does not go through [`verdict`].** That answers what a set of sites is
/// worth *now*, and now is before the free, where every one of them is still
/// live: it answers `None` here, correctly, to a different question. RK-055 in
/// the review knowledge bank is one judgement point inheriting a rule written
/// for the other question, which is the mistake in the other direction.
///
/// **It asks [`Allocations::touching`] over the arguments themselves, where
/// [`reported`] asks it over [`asked`], and the two cannot be made to agree
/// because they can never both have something to say.** ADR-0027's exemption
/// takes an argument out wherever [`NullAtTerminators::established`] holds, and
/// three lines put that answer and this walk in different programs:
///
/// 1. `nullability::null_at_terminators` zeroes a block's whole row unless
///    `block.elements.last()` is [`Element::ArgumentsEvaluated`], which is
///    ADR-0027's ordering condition;
/// 2. [`Allocations::element`] answers that same element by clearing
///    [`Known::pending`], because C17 6.5.2.2 p10 orders a call's arguments
///    before the call;
/// 3. this runs after every element of the block and reports only out of
///    `pending`.
///
/// So where a row can exempt anything the marker is last, `pending` was just
/// emptied, and there is nothing here to report; and where this reports, the
/// free is under an unsequenced operator, the marker is absent, and the row is
/// `false` for every local. The `Callee::Opaque` half is the same `pending`
/// reached through the same elements.
///
/// **Written because it was measured and not because it follows**, which is
/// RK-050: the filter was added here, and the reproducer on #219 and the whole
/// workspace suite came back byte for byte the same. The `debug_assert!` below
/// is what keeps that true, because a paragraph does not.
///
/// What would make the filter live is an ordering term asked per local across a
/// block edge rather than per block, which is a lattice dimension and is
/// ADR-0027's own Consequences rather than this function's.
fn used_before(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
    null: &NullAtTerminators,
    block: BlockId,
) {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return;
    };

    // Which of the two questions above this is, and the one callee that asks
    // neither. Written as a match rather than as a comparison so that a fourth
    // callee is answered for here by `error[E0004]` rather than falling into a
    // row decided before it existed.
    let frees = match analysis.callee(*callee) {
        Callee::Frees => true,
        Callee::Opaque => false,
        // `malloc` frees nothing and takes no pointer, so a read carried to it
        // is a read this call has nothing to say about.
        Callee::Allocates => return,
    };

    // The machine behind the paragraph above. Placed here because this is
    // where the two rules would have met: `frees` is decided and `touching` is
    // about to be asked over arguments no exemption has been applied to.
    //
    // An assertion rather than the filter, because the filter is a line no
    // mutation can break, which `CLAUDE.md` calls decoration, and because it
    // would go quiet on its own the day the ordering term widens: it would
    // exempt a read without anybody asking whether ADR-0027's four conditions
    // still hold where the exemption had newly arrived.
    //
    // **This junction is one the corpus reaches**, which is what makes the
    // assertion worth its line. Deleting the `known.pending.is_empty()`
    // disjunct panics `a_free_of_a_pointer_proved_null`,
    // `a_local_given_nothing_forgets_the_set_it_freed` and
    // `a_pointer_set_to_nothing_after_a_free_holds_nothing`: all three reach
    // here with an argument this check established null, and an empty
    // `pending` is the only reason nothing is reported about it.
    //
    // **What it will not do is catch the widening it is written for, on the
    // corpus.** Forcing `nullability`'s `ordered` true panics nothing; it
    // fails `a_free_in_an_unsequenced_operand_is_not_exempt` and
    // `..._across_a_call_is_not_exempt`, which go from `error[SC0401]` to exit
    // 0 with nothing said. So the widening is already guarded, one row above
    // what this offers, and what this adds is a debug run on a program the
    // corpus does not have: a read carried through a *second* pointer, so that
    // ADR-0025's refinement does not clean the one being freed.
    //
    // Two further measurements, so that the next reader does not repeat them.
    // Weakening this `any` to `all` breaks nothing. Deleting the assertion
    // leaves `null` and `block` unused, which is two warnings, and errors only
    // because the gate runs clippy with `-D warnings`; deleting the two
    // parameters along with it compiles clean. That is a guard against
    // forgetting rather than against deciding.
    debug_assert!(
        !frees
            || known.pending.is_empty()
            || !arguments
                .iter()
                .any(|argument| established_null(argument, null, block)),
        "a read was carried to a free of a pointer established null: `asked` now reaches this walk"
    );

    let touched = Allocations::touching(arguments.iter(), known);
    let taken: Vec<usize> = named(&touched).collect();

    for ((.., place), read) in &known.pending {
        let both: Vec<usize> = read
            .sites
            .iter()
            .copied()
            .filter(|site| taken.contains(site))
            .collect();

        if both.is_empty() {
            continue;
        }

        // **Only while every allocation they have in common agrees**, which is
        // `verdict`'s rule about `made` and is here for its reason: naming one
        // of several allocations as *the* one is the may-set mistake about a
        // label. RK-035 is the entry and `allocated here` is the claim.
        let mut made = None;
        for (index, site) in both.iter().enumerate() {
            let from = match known.state[*site] {
                SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                SiteState::Unknown => None,
            };
            made = if index == 0 { from } else { same(made, from) };
        }

        say(
            findings,
            said,
            place,
            Finding {
                kind: Kind::UseAfterFree,
                conclusion: Conclusion::Unknown,
                at: read.at,
                // Exactly one free, which is this one: the reads are carried
                // to each free separately, so there is nothing folded here and
                // nothing for the caret to be wrong about. An opaque call has
                // freed nothing this check established, so there is no span to
                // put `freed here` on and no order to explain, which is what
                // keeps C17 6.5.2.2 p10's note off a program with no free in
                // it. RK-036 is a label claiming more than the analysis
                // established.
                freed: frees.then(|| origin.span()),
                made,
                unproven: Some(if frees {
                    Unproven::Unsequenced
                } else {
                    // The reason this is, rather than one lent to it: the
                    // variant's own doc says a call this check cannot read was
                    // handed a pointer and may have freed it.
                    Unproven::Disagreement
                }),
            },
        );
    }
}

/// Whether a report at a caret replaces the one already standing there.
///
/// **A proof replaces a suspicion, and nothing else replaces anything.** Where
/// two proofs meet at one caret the first stands: both are true of the same
/// place and there is nothing to choose between them. [`used`] is where a pair
/// at one caret comes from and why one is collapsed at all.
///
/// **Answered per pair rather than by an ordering.** [`Conclusion`] does not
/// derive `Ord` and should not: its three variants are three answers rather
/// than three degrees, and `Safe` is not a weaker `Unsafe`. The pairs that
/// cannot arise say so rather than falling through, because RK-034 in the
/// review knowledge bank is a fallthrough in this file that was reached by
/// "proved" and by "gave up" at once and reported the second as the first.
fn supersedes(standing: Conclusion, new: Conclusion) -> bool {
    match (standing, new) {
        // First, because a wildcard below would absorb them. [`verdict`]
        // answers `None` where there is nothing to report, so no `Finding`
        // carries `Safe` and no verdict reaching here concludes one; written
        // last, `(Conclusion::Unsafe, _)` answered `(Unsafe, Safe)` in silence
        // while this said every impossible pair is declared.
        (Conclusion::Safe, _) => unreachable!("a finding standing at a caret concluded Safe"),
        (_, Conclusion::Safe) => unreachable!("a verdict about a dereference concluded Safe"),
        (Conclusion::Unknown, Conclusion::Unsafe) => true,
        (Conclusion::Unknown, Conclusion::Unknown) | (Conclusion::Unsafe, _) => false,
    }
}

/// Where this element runs, and every place it reads or writes through a
/// pointer there.
///
/// Through a pointer, so a projection: an unprojected place is the local itself
/// and holding a freed pointer is not using it. The span is the element's,
/// because a [`Place`] has none of its own.
pub(crate) fn dereferenced_in_element(element: &Element) -> Option<(Span, Vec<&Place>)> {
    // Every field written out, never `..`: RK-018 in the review knowledge bank
    // is a field added to a variant that already exists walking past an
    // exhaustive match.
    match element {
        Element::Assign(operation) => {
            let mut places = projected(&operation.place);
            places.extend(dereferenced_in_rvalue(&operation.value));
            Some((operation.origin.span(), places))
        }
        // The element that exists so this can see it. `projected` rather than
        // the place itself, even though `Element::Evaluate`'s doc says a
        // producer owes a projection: one rule about what counts as reaching
        // through a pointer, applied everywhere, beats two that agree today.
        Element::Evaluate { place, origin } => Some((origin.span(), projected(place))),
        // Neither marker, nor storage beginning or ending, reads anything
        // through anything.
        Element::Sequenced { origin: _ } | Element::ArgumentsEvaluated { origin: _ } => None,
        Element::StorageLive {
            local: _,
            origin: _,
        } => None,
        Element::StorageDead {
            origin: _,
            local: _,
        } => None,
    }
}

/// The same, for what a terminator reads.
pub(crate) fn dereferenced_in_terminator(terminator: &Terminator) -> Option<(Span, Vec<&Place>)> {
    match terminator {
        Terminator::Call {
            callee: _,
            arguments,
            destination,
            then: _,
            origin,
        } => {
            let mut places: Vec<&Place> = arguments.iter().flat_map(dereferenced_in).collect();
            if let Some(destination) = destination {
                places.extend(projected(destination));
            }
            Some((origin.span(), places))
        }
        // **A condition is read here and not from an element, because a
        // condition that is exactly a place never becomes one.** `if (*p + 1)`
        // computes into a temporary and the `Operation` that does it carries
        // the dereference; `if (*p)` hands the place straight to the
        // terminator. Both are one defect and the difference is whether the
        // expression needed a temporary, which is not something a reader could
        // predict, so this arm is what makes the answer the same for both.
        Terminator::Branch {
            condition,
            then: _,
            otherwise: _,
            origin,
        } => Some((origin.span(), dereferenced_in(condition))),
        Terminator::Goto(_) | Terminator::Return | Terminator::Abnormal { to: _ } => None,
    }
}

/// The same, for what an rvalue reads.
fn dereferenced_in_rvalue(value: &Rvalue) -> Vec<&Place> {
    match value {
        Rvalue::Use(operand) => dereferenced_in(operand),
        Rvalue::Unary { op: _, operand } => dereferenced_in(operand),
        Rvalue::Binary { op: _, lhs, rhs } => {
            let mut places = dereferenced_in(lhs);
            places.extend(dereferenced_in(rhs));
            places
        }
        // **Taking an address is not a dereference**, whatever the place
        // it is taken of looks like. C17 6.5.3.2 p3 is why `&*p` used to be
        // the example: "neither that operator nor the `&` operator is
        // evaluated and the result is as if both were omitted". The lowering
        // applies that clause now, so no C program reaches here with one, and
        // what does reach here is an address of a place a frontend really
        // meant to take. Reporting it would be a use of a freed value in a
        // program that never touched one.
        Rvalue::Address(_) => Vec::new(),
    }
}

/// The same, for one operand.
fn dereferenced_in(operand: &Operand) -> Vec<&Place> {
    match operand {
        Operand::Copy(place) => projected(place),
        Operand::Constant(_) => Vec::new(),
    }
}

/// The place, where it goes through a projection, and nothing where it does not.
fn projected(place: &Place) -> Vec<&Place> {
    if place.projection.is_empty() {
        Vec::new()
    } else {
        vec![place]
    }
}
