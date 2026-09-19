//! Whether a program frees one allocation twice, or uses one after it was
//! freed.
//!
//! The first checks on [the safety model]'s memory axis, and the first thing
//! this compiler says about what a C program *does* rather than about how it is
//! written.
//!
//! **Two answers out of one walk.** They read one lattice: what a free does to
//! a site is what makes a later use of it a defect, so computing the states
//! twice would be the same computation twice and a second chance for the two
//! copies to disagree. [`Kind`] is how the caller tells them apart.
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
    Element, FuncId, Function, LocalId, Operand, Place, Projection, Rvalue, Terminator,
    TranslationUnit,
};
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
    /// holding something this check can no longer name.
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
    /// name, because the site that named it was handed to a second one.
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
}

impl Held {
    /// A local holding nothing, in a function with this many sites.
    fn none(sites: usize) -> Self {
        Held {
            sites: vec![false; sites],
            lost: false,
            freed: None,
            writes_to: vec![false; sites],
        }
    }

    /// Also hold this site.
    fn hold(&mut self, site: usize) {
        self.sites[site] = true;
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
        } = self;
        sites.fill(false);
        *lost = false;
        *freed = None;
        writes_to.fill(false);
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
        } = self;
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
        } = self;

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

        // The three may-facts, which grow wherever anything meets. The edge is
        // the one nothing observes here: both callers that build a value out of
        // operands empty it on the next line, because C17 6.5.6 p8 keeps a
        // pointer's arithmetic inside the object and a local's address plus one
        // is not that local. See ADR-0019, and the two `writes_to.fill(false)`
        // lines in `Allocations::element`.
        for (here, there) in sites.iter_mut().zip(&other.sites) {
            *here = *here || *there;
        }
        *lost = *lost || other.lost;
        for (here, there) in writes_to.iter_mut().zip(&other.writes_to) {
            *here = *here || *there;
        }
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
    /// a program that frees nothing at all, and made `--deny-unknown` unusable
    /// on the output-parameter idiom. `an_escaped_local_that_reaches_no_site_at_all`
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
    /// was silent at `--deny-unknown --safety strict` on a double free. The
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
}

/// What the operands of a binary operation build.
///
/// **The proof survives only where nothing else contributed.** One followed
/// operand and a constant is `p + 1`: the result is that operand offset, and
/// C17 6.5.6 p8 keeps it inside the same object, so the set the proof is about
/// is the set the result names. Two followed operands is an expression whose
/// value may be either of them, and [`Held::accumulated`] drops the proof for
/// the reason written there.
///
/// One function with two callers, because the same question is asked where a
/// value is assigned and where one is written through a pointer, and RK-052 in
/// the review knowledge bank is one rule in two places drifting apart inside
/// the change that touches one of them.
fn built_from(operands: [&Operand; 2], value: &Known) -> Held {
    let followed: Vec<usize> = operands
        .iter()
        .filter_map(|operand| match operand {
            // A constant is not a value this check follows, and neither is a
            // read through a projection: `*pp + 1` reaches whatever `pp` points
            // at, which is a place rather than a local.
            Operand::Copy(source) if source.projection.is_empty() => Some(source.local.index()),
            _ => None,
        })
        .collect();

    let mut reached = Held::none(value.points_to.len());
    for &source in &followed {
        reached.accumulated(&value.points_to[source]);
    }

    if let [one] = followed[..] {
        reached.freed = value.points_to[one].freed;
    }

    reached
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

    /// What the arguments of a call reach, in the order they were written.
    ///
    /// Shared with [`check`], so that the walk which reports and the walk which
    /// computes cannot disagree about what a call touches.
    fn touching(arguments: &[Operand], known: &Known) -> Vec<Reached> {
        let mut reached = Vec::new();

        for argument in arguments {
            let place = match argument {
                // Not a pointer that went missing. `free(0)` is the case, and
                // C17 7.22.3.3 p2 makes it do nothing, so it is written on
                // purpose and is not something this check lost track of.
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
        // so it takes at most one step per pair. Where the set it named was
        // freed goes from `None` to `Some` once per local and a join only takes
        // it away, so it costs one more step each. Whether a free has been
        // sequenced is one bit per site: the transfer sets it and only a join
        // takes it back, which a join can do once, so it is one more step per
        // site and the number below is not changed for it. That is slack being
        // spent rather than a bound being re-derived, and the paragraph below
        // is why that is acceptable here.
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
        locals * locals * 2 + locals * (locals + 6) + positions * (locals + 1)
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
        for &parameter in &self.parameters {
            known.points_to[parameter.index()].hold(parameter.index());
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

    fn element(&self, _function: &Function, element: &Element, value: &mut Self::Value) {
        // What is read here is read where this element runs, against what held
        // before it, which is the same question `check` asks one line earlier
        // and has to get the same answer to.
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
            Element::Assign(operation) => {
                // **A write through a pointer, where this check knows where it
                // lands.** `*pp = q` is what makes `p` hold `q`'s allocation,
                // and until it was followed the free that came after it was
                // read against an allocation nobody had written there:
                // `int **pp = &p; *pp = q; free(p); *q = 1;` said nothing at
                // all about the last line. See ADR-0019, which also records
                // why not knowing where the write lands is answered by doing
                // nothing.
                //
                // Exactly one `Deref` and nothing deeper. The edge recorded at
                // `Rvalue::Address` is one step, and reading it as two would
                // be inventing the second.
                if operation.place.projection.as_slice() == [Projection::Deref] {
                    let targets = value.written_through(operation.place.local);
                    if targets.is_empty() {
                        return;
                    }

                    // **A union rather than a replacement**, because a pointer
                    // that may point at one local is not a pointer that must:
                    // two arms of a branch can leave `pp` with one target each
                    // and neither is certain here. Replacing would erase a
                    // value nothing wrote over, which is a silence rather than
                    // a false positive.
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
                        // frontend may. Both operands, for the reason the arm
                        // above gives, and the same question about the edge:
                        // this asked none of it until review built the shape by
                        // hand, and carried an edge through `qq + 7` that the
                        // arm one level up had just been taught to drop.
                        Rvalue::Binary { op: _, lhs, rhs } => {
                            let mut reached = built_from([lhs, rhs], value);
                            reached.writes_to.fill(false);
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
                    // Both operands, and a union rather than a choice. Which
                    // one is the pointer is a question about types and this
                    // does not read them; taking both is a may-set growing,
                    // which is the direction that cannot make a proof out of
                    // nothing. `q - p` is an integer and picks up both, and
                    // nothing dereferences an integer.
                    //
                    // Read before the write, so `p = p + 1` keeps what `p`
                    // held rather than clearing it and unioning the result.
                    //
                    // **It costs a proof where the index is a local.** A
                    // parameter is a site, so `p[i]` unions the allocation `p`
                    // holds with the site `i` is, and a site that is live stops
                    // the result being proved: `free(p); p[i] = 42;` is a
                    // warning where `free(p); p[0] = 42;` is an error. The
                    // types are in the IR and reading them would separate the
                    // two, which is #143 rather than a line here.
                    Rvalue::Binary { op: _, lhs, rhs } => {
                        let mut reached = built_from([lhs, rhs], value);
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
        let touched = Self::touching(arguments, value);
        let sites = || named(&touched);

        match self.callee(*callee) {
            Callee::Frees => {
                let reached: Vec<usize> = sites().collect();

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
        value.points_to[site].hold(site);
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

/// Which of the two things this check answers about a finding is.
///
/// One walk over one lattice, so this is not two checks and `docs/diagnostics.md`
/// says so where it hands the two their codes. What differs is the question:
/// the codes and the words are different, and the thing a caret lands on is a
/// call in one and a dereference in the other.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `free(p); free(p);`
    DoubleFree,
    /// `free(p); *p = 42;`
    UseAfterFree,
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
/// **Three reasons and not two, because one of them is about this check and
/// the other two are about the program.** A reader told a value may have been
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
    /// `Reached::Lost` with nothing else contributing. The two producers are
    /// `Known::reached_by`'s: a local that held a site and lost the name for
    /// it, which is ADR-0018, and one whose address escaped, which is
    /// ADR-0017. Neither says a free happened; both say this check can no
    /// longer say what the pointer points at.
    ///
    /// Those two are private, so they are named here rather than linked: a
    /// link out of a public item to one of them is
    /// `rustdoc::private_intra_doc_links`, which this crate denies.
    Untracked,
    /// C has not said which order runs. See ADR-0022.
    Unsequenced,
}

/// Every double free this unit contains, and every one it cannot rule out.
///
/// One walk per function: the fixpoint answers what holds where each block
/// starts, and this replays each block from there to find the calls to report.
/// The replay rather than a second lattice, because the transfer is what
/// decides which sites a call touches and having two answers to that is having
/// one of them be wrong.
pub fn check(sources: &SourceMap, unit: &TranslationUnit) -> Vec<Finding> {
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
            if let Some(finding) = reported(&analysis, &block.terminator, &known) {
                findings.push(finding);
            }
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
    };

    match earliest {
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
            unproven: Some(Unproven::Untracked),
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
/// One finding per call rather than one per site: a local may point at several
/// allocations where a branch put them there, and two carets on one `free` say
/// one thing twice.
fn reported(analysis: &Allocations<'_>, terminator: &Terminator, known: &Known) -> Option<Finding> {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return None;
    };

    if analysis.callee(*callee) != Callee::Frees {
        return None;
    }

    let verdict = verdict(
        Kind::DoubleFree,
        Allocations::touching(arguments, known),
        known,
    )?;

    Some(Finding {
        kind: Kind::DoubleFree,
        conclusion: verdict.conclusion,
        at: origin.span(),
        freed: verdict.freed,
        made: verdict.made,
        unproven: verdict.unproven,
    })
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

/// Report every read behind this free that it may be about, where nothing
/// orders the two.
///
/// **The half a forward walk cannot see, arriving from the other side.** A read
/// the walk meets before the free is never asked about it, because a free marks
/// only what follows; carrying the read forwards to the free asks the same
/// question at the only point where both are in hand. `Element::Sequenced` is
/// what says a read is behind rather than beside, and clearing
/// [`Known::pending`] is where that happens. See ADR-0023.
///
/// **Always unproven, and that is the shape rather than a caution.** The read
/// and the free are in one full expression with nothing sequencing them, so one
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
fn used_before(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
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

    // Only a `free`. A call this check cannot read may free what it was passed
    // and the same asymmetry is there, wider: `h(p) + g(*p)` reports today and
    // `g(*p) + h(p)` does not. That is issue #184 and not this rule, because
    // what it costs is every dereference beside an opaque call rather than
    // beside a free.
    if analysis.callee(*callee) != Callee::Frees {
        return;
    }

    let touched = Allocations::touching(arguments, known);
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
                // Exactly one free, which is this one: the reads are carried to
                // each free separately, so there is nothing folded here and
                // nothing for the caret to be wrong about.
                freed: Some(origin.span()),
                made,
                unproven: Some(Unproven::Unsequenced),
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
fn dereferenced_in_element(element: &Element) -> Option<(Span, Vec<&Place>)> {
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
        // Neither a sequence point nor storage beginning or ending reads
        // anything through anything.
        Element::Sequenced { origin: _ } => None,
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
fn dereferenced_in_terminator(terminator: &Terminator) -> Option<(Span, Vec<&Place>)> {
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
