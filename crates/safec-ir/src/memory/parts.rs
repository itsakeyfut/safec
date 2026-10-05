//! The lattice's parts: what a local may hold, [`Held`], and what it reached,
//! [`Reached`]; what is known about a site, [`SiteState`], and where in it a
//! pointer is, [`Offset`]; the facts carried until something answers them, a read
//! awaiting the free it is unordered against and a `realloc` awaiting its branch;
//! and what a call's name says, [`Callee`].
//!
//! [`Known`](super::known::Known) is the value they make up, in its own module.

use std::collections::BTreeSet;

use crate::ir::Place;
use crate::source::Span;

/// What this check can read in a callee's name.
///
/// By name because nothing else is available: no annotation says what a
/// function does to what it is passed. The one annotation there is,
/// `_Nonnull`, says only that a parameter is not null, which is ADR-0037.
/// Besides `malloc` and `free`, the names read are the library functions
/// ADR-0039 lists, each for what its clause says it does to what it is handed.
/// C17 7.1.3 reserves the identifiers the library declares, so a program
/// that defines its own `free` has no behaviour C defines. `clang -std=c17
/// -pedantic-errors` does not diagnose one, measured, so a program that does it
/// anyway is read wrongly here and there is no way to tell from inside.
/// **No `PartialEq`**, so that every reader is a `match` and a new kind is
/// `error[E0004]` wherever it has to be answered. A comparison lets it through
/// in silence: the rule for what an opaque call returns was once gated by one.
#[derive(Clone, Copy)]
pub(super) enum Callee {
    /// C17 7.22.3.3's `free`.
    Frees,
    /// C17 7.22.3.4's `malloc`, and 7.22.3.1's `aligned_alloc` and 7.22.3.2's
    /// `calloc`, which 7.22.3 p1 holds to the same thing: a pointer to the
    /// start of an object disjoint from any other. Read so that its arguments
    /// are left alone, and so that what it returns is a fresh allocation nobody
    /// else has, rather than one an opaque call may hand back. An allocation
    /// is otherwise named by where it landed rather than by which function made
    /// it. See ADR-0039.
    Allocates,
    /// C17 7.22.3.5's `realloc`.
    ///
    /// Its first argument may be freed, and is not when the call fails with a
    /// nonzero size (p3), so what it names becomes unproven rather than freed.
    /// What it returns is a fresh allocation, as `malloc`'s is, holding what
    /// the old one held (p2). See ADR-0039.
    Reallocates,
    /// A C library function that frees nothing and returns its first argument:
    /// `memset`, `strcpy`, `strncpy`, `strcat` and `strncat`, whose clauses in
    /// C17 7.24.2 to 7.24.6 each say so.
    ///
    /// **What it is handed is exposed all the same**, because it copies bytes
    /// and a pointer is bytes, and a local whose address it is handed may be
    /// written through it. See ADR-0039.
    ReturnsFirst,
    /// `memcpy` and `memmove`: what [`Callee::ReturnsFirst`] is, and a copy of
    /// an object besides, so that what the destination may contain gains what
    /// the source contains (C17 7.24.2.1 and 7.24.2.2 copy the object's bytes).
    ///
    /// **Not the string functions**, whose copies end at a null byte (7.24.2.3
    /// and on), which a pointer's representation may hold, so a stored pointer
    /// is not copied whole by them. A variant of its own rather than a name
    /// read inside the transfer, so that every reader of the callee answers
    /// for it. See ADR-0039.
    Copies,
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
pub(super) struct Freeing {
    /// Where the free is.
    pub(super) at: Span,
    /// Whether an [`Element::Sequenced`](crate::ir::Element::Sequenced) has passed since.
    ///
    /// False until one has. C17 6.5 p3 leaves the rest of the full expression
    /// unsequenced with the call, so which happens first is C's to choose and
    /// nothing here is yet a proof.
    pub(super) sequenced: bool,
}

impl Freeing {
    /// A free nothing has sequenced yet.
    pub(super) fn new(at: Span) -> Self {
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
    pub(super) fn joined(self, other: Self) -> Self {
        Freeing {
            at: earlier(self.at, other.at),
            sequenced: self.sequenced && other.sequenced,
        }
    }
}

/// What is known about the allocation one site stands for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SiteState {
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
    pub(super) fn joined(self, other: Self) -> Self {
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
pub(super) enum Offset {
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
    /// pointer on the other. That is a false report the reader can see rather
    /// than saying safe wrongly, and ADR-0036 records it as what three values
    /// cost.
    pub(super) fn joined(self, other: Self) -> Self {
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
pub(super) fn same(here: Option<Span>, there: Option<Span>) -> Option<Span> {
    match (here, there) {
        (Some(here), Some(there)) if here == there => Some(here),
        _ => None,
    }
}

/// The earlier of two spans, by where they are rather than by which arrived.
///
/// A total order over the pair, so a value carrying one can only move one way
/// and the walk ends. The file is the first half because a span names its
/// own file and two of them need not share one.
fn earlier(here: Span, there: Span) -> Span {
    if (here.file().index(), here.start()) <= (there.file().index(), there.start()) {
        here
    } else {
        there
    }
}

/// What one read is filed under. See [`ReadKey`].
pub(super) fn read_key(at: Span, place: &Place, read: Read) -> ReadKey {
    (at.file().index(), at.start(), at.end(), place.clone(), read)
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
pub(super) enum Reached {
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
    /// The sites beside this may not be all the value can hold.
    ///
    /// **Not a doubt on its own, which is what keeps it apart from
    /// [`Reached::Lost`].** A pointer read out of memory holds what
    /// [`Known::inside`](super::known::Known::inside) records for the allocation it was read from, and
    /// that is a lower bound: slots are not told apart, and a store this check
    /// cannot place is exposed rather than recorded. So nothing proves from
    /// such a set, but a set of live sites says nothing either, and a
    /// dereference of a live load is not a report. See ADR-0045.
    Partial,
}

/// What one local may hold.
///
/// A struct rather than the bit vector this was, because the two halves travel
/// together everywhere: a copy writes both, arithmetic unions both, and
/// anything else clears both. Kept apart they have to be kept in step by hand
/// in every arm of [`Allocations::element`](super::transfer::Allocations#method.element), and forgetting one is a silence
/// rather than a build error. See ADR-0018.
///
/// **What lives here is what an assignment destroys.** Giving a local a fresh
/// value replaces everything in this struct, which is why [`Held::clear`] can
/// answer for a field without being told what it means. A fact that outlives
/// an assignment does not belong here however much it looks like one:
/// [`Known::escaped`](super::known::Known::escaped) is per local and stays on [`Known`](super::known::Known) for exactly that
/// reason, and putting it here would answer `false` after `p = q;` and undo
/// #155.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Held {
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
    /// arrives or when somebody hits this on real code.
    ///
    /// **The third square field has arrived**: [`Known::inside`](super::known::Known::inside). Measured on
    /// a 456-line function with 150 allocations, release build, peak memory
    /// went from 485 MB to 711 MB and time from 0.49 s to 0.71 s. The bitset is
    /// #173's; the number is here so that it is a decision rather than a
    /// discovery. See ADR-0039.
    pub(super) sites: Vec<bool>,
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
    pub(super) lost: bool,
    /// Where the allocation this local held was freed, when the free could not
    /// say which member of the set it was.
    ///
    /// A free of a local reaching two sites frees exactly one of them, and
    /// writing `Freed` on both said each was certainly freed: a later free of
    /// one by name became a proved double free about a program with no defect
    /// on one path. What is true is a fact about the **set**, and the local
    /// that named the set is the only place it fits. See ADR-0020.
    pub(super) freed: Option<Freeing>,
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
    pub(super) writes_to: Vec<bool>,
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
    pub(super) writes_elsewhere: bool,
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
    pub(super) offset: Offset,
    /// Whether this local may hold a pointer read out of memory.
    ///
    /// Such a pointer holds what [`Known::inside`](super::known::Known::inside) records for the allocation
    /// it was read from, which may be missing members, and this bit is what
    /// says so: the report reads [`Reached::Partial`] beside those sites, and
    /// a free of it stays a doubt (ADR-0045). **For the other readers**, what
    /// a call reaches and what a write stores, it says the value may be
    /// anything stored, for which "no site" had come to mean "reaches
    /// nothing". It is not the emptiness of [`Held::sites`], because
    /// `int *z = 0;` is empty too, and reading it as a load exposed every
    /// stored pointer at `log_ptr(z)`, measured. See ADR-0040.
    ///
    /// It says that the value was read out of memory and not from where, so a
    /// write through it is unplaced for that part: what it carries is
    /// exposed. See ADR-0044.
    pub(super) loaded: bool,
    /// Whether a call this check cannot read may have written into this
    /// local, through its address, something no site names.
    ///
    /// [`Known::replaced`](super::known::Known::replaced) cannot say this with [`Held::lost`] for a local
    /// holding no site, because the report reads that bit and ADR-0017 does
    /// not report an output parameter. So `get(&u)` sets this instead, and
    /// the report never reads it. A write through such a local exposes what
    /// it carries, as one through a load does. See ADR-0044.
    pub(super) foreign: bool,
    /// Whether this was read out of an allocation [`Known::stale`](super::known::Known::stale) marks, or
    /// otherwise lost an allocation that may be gone, and may be one.
    ///
    /// Set too where a local loses a site whose allocation is not live
    /// ([`Known::reborn`](super::known::Known::reborn)), by a load through a pointer to such a local, and
    /// by a call this check cannot read for what it makes lost of the
    /// caller's. A store of any of them was recorded as nothing and read back
    /// in silence, where read directly it was doubted. See ADR-0045.
    ///
    /// **Not [`Held::lost`], which says the same to the report and more to a
    /// store.** A local loses a site whenever the site is reborn, whatever
    /// became of the allocation it named, so `*node = head; head = node;`
    /// builds a list whose `head` is lost every turn though nothing is freed.
    /// A store of a lost value marking its container doubted every walk of
    /// that list, measured; a store of this one marks it, so a doubted load
    /// copied into another allocation stays doubted. See ADR-0045.
    pub(super) stale_read: bool,
    /// Whether this was read out of memory a pointer parameter points at, so
    /// may be what the caller stored there.
    ///
    /// **Made [`Held::lost`] at a call this check cannot read.** What the
    /// caller stored behind `pp` is recorded nowhere, so a load of it holds no
    /// site and nothing a call does could reach it: `q = *pp; release_all();
    /// return *q;` built. The caller may have stashed it where the call frees
    /// it, which is ADR-0040's reason for exposing the parameter itself, one
    /// level in; and its cost, a read after any such call doubted, is the same
    /// one level in. See ADR-0040.
    pub(super) from_caller: bool,
    /// The site a `realloc` returned, where this local holds exactly that
    /// value: the call's destination and plain copies of it.
    ///
    /// **Points-to cannot say a pointer may be null for another reason**, so
    /// a branch reading only that `q` holds the returned site read `r = q; if
    /// (c) r = 0; if (r == 0) free(p);` as a failed call on the arm where `r`
    /// was nulled by hand, and built a double free. Any other assignment
    /// clears this, arithmetic included, and a join keeps it only where both
    /// arms agree. Found by review. See ADR-0039.
    pub(super) returned_by: Option<usize>,
}

impl Held {
    /// A local holding nothing, in a function with this many sites.
    pub(super) fn none(sites: usize) -> Self {
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
            loaded: false,
            foreign: false,
            stale_read: false,
            from_caller: false,
            returned_by: None,
        }
    }

    /// Also hold this site, at this offset into it.
    ///
    /// **The offset is an argument so that it cannot be forgotten.** A site
    /// arrives from a call's destination or from a parameter, each of which is
    /// the start of whatever it stands for, or as what an opaque call may have
    /// returned, which may point anywhere in it and says `Unknown`. A producer
    /// written later has to say which: dropping the argument is `error[E0061]`
    /// at every call site. See ADR-0036 and ADR-0039.
    ///
    /// **Joined rather than assigned.** A call's destination and a parameter
    /// hold on a local [`Held::none`] or [`Held::clear`] has just left at
    /// `Zero`, where the two agree. An opaque call's result is where they part:
    /// it holds its own site at `Zero` and then what the call may have
    /// returned at `Unknown`, and the join makes the whole `Unknown`, which is
    /// the answer that cannot claim more than both said.
    pub(super) fn hold(&mut self, site: usize, offset: Offset) {
        self.sites[site] = true;
        self.offset = self.offset.joined(offset);
    }

    /// Every site held, in order.
    pub(super) fn sites(&self) -> impl Iterator<Item = usize> + '_ {
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
    pub(super) fn clear(&mut self) {
        // Every field named, never `..`: a field added here and missed is a
        // fact that survives an assignment, which is `..` letting a field walk
        // past an exhaustive match, one type over.
        let Held {
            sites,
            lost,
            freed,
            writes_to,
            writes_elsewhere,
            offset,
            loaded,
            foreign,
            stale_read,
            from_caller,
            returned_by,
        } = self;
        sites.fill(false);
        *from_caller = false;
        *returned_by = None;
        *lost = false;
        *loaded = false;
        *foreign = false;
        *stale_read = false;
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
    pub(super) fn joined(&mut self, other: &Held) {
        // Every field named, never `..`, in this method and in its neighbour
        // alike: a field added to this struct is `error[E0027]` in both and has
        // to say what it means in each. ADR-0024 is why there are two places
        // to answer.
        let Held {
            sites,
            lost,
            freed,
            writes_to,
            writes_elsewhere,
            offset,
            loaded,
            foreign,
            stale_read,
            from_caller,
            returned_by,
        } = self;
        if *returned_by != other.returned_by {
            *returned_by = None;
        }
        *offset = offset.joined(other.offset);
        *from_caller = *from_caller || other.from_caller;
        *loaded = *loaded || other.loaded;
        *foreign = *foreign || other.foreign;
        *stale_read = *stale_read || other.stale_read;
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
    pub(super) fn accumulated(&mut self, other: &Held) {
        let Held {
            sites,
            lost,
            freed,
            writes_to,
            writes_elsewhere,
            offset,
            loaded,
            foreign,
            stale_read,
            from_caller,
            returned_by,
        } = self;
        *from_caller = *from_caller || other.from_caller;
        // Arithmetic builds a new value, which is no longer exactly what a
        // `realloc` returned. See ADR-0039.
        *returned_by = None;

        // `*tab + 1` is built from a pointer read out of memory, and is one.
        *loaded = *loaded || other.loaded;
        *foreign = *foreign || other.foreign;
        *stale_read = *stale_read || other.stale_read;

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
        // flag beside it are decided again by both callers that build a value
        // out of operands, through [`Held::moved_by_arithmetic`]. See
        // ADR-0019.
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
    pub(super) fn lose(&mut self, site: usize) {
        if self.sites[site] {
            self.sites[site] = false;
            self.lost = true;
        }
    }

    /// What pointer arithmetic leaves of the edge to a local.
    ///
    /// **Kept where the offset may be zero, and dropped where it is known not
    /// to be.** C17 6.5.6 p8 treats a local as an array of one, so `&slot +
    /// n` may be dereferenced only for `n` of zero: a pointer this check moved
    /// by an offset it cannot read may be `slot`, and dereferenced it is
    /// `slot` or undefined. `po[k - 1]` read nothing of `slot` while `*po`
    /// read it. A non-zero constant, `pp[1] = q;`, is not `slot`, and
    /// following that edge reported a proved use after free about an
    /// allocation nothing had freed.
    ///
    /// **The set is given up on either way**: kept, it is a may-write by
    /// union, never the replacement a certain write is; emptied, an empty set
    /// is what a pointer this check never followed an address into has, and
    /// giving up on it is saying so (ADR-0028). One method for both callers
    /// that build a value out of operands, because one rule written in two
    /// places drifts apart. See ADR-0019.
    ///
    /// **Two parts no program tells apart, measured.** Leaving
    /// `writes_elsewhere` unset on a kept edge would make a write through it a
    /// replacement, but the local the edge names had its address taken, so
    /// ADR-0017 leaves what it holds unproven and the read is doubted either
    /// way. And the caller in `Allocations::carried` is reached only by an IR
    /// whose arithmetic is written straight into a place, which the C frontend
    /// never builds: it puts the sum in a temporary first.
    ///
    /// **What it does not follow**: a pointer moved off its start and back,
    /// `up = po + 1; back = up - 1;`, since `up` is `Offset::NonZero` and the
    /// edge is gone before `back` could keep it. And it rests on a local being
    /// an array of one: once array locals compile, `pp + 1` may name a real
    /// element, and the `NonZero` drop has to be asked again.
    pub(super) fn moved_by_arithmetic(&mut self) {
        if !matches!(self.offset, Offset::Unknown) {
            self.writes_to.fill(false);
        }
        self.writes_elsewhere = true;
    }
}

/// A dereference this walk has met since the last sequence point, or a pointer
/// a call was handed.
///
/// **The other half of ADR-0022, which this one is ADR-0023 for.** An
/// [`Element::Sequenced`](crate::ir::Element::Sequenced) says what is ordered and a forward walk only ever
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
///
/// **A pointer handed to a call is carried the same way**, because what C
/// leaves unordered against a later free is the callee's body, and that body
/// reads what it was handed. `(memset(a, 0, 4) != 0) + (free(a), 0)` built
/// while its dereference spelling reported. See ADR-0042.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct PendingRead {
    /// The element's span, which is where `used here` goes, or the call's,
    /// which is where `passed here` goes.
    ///
    /// Kept beside the key rather than in it, because a [`Span`] cannot be
    /// rebuilt from the position the key holds.
    pub(super) at: Span,
    /// Which allocations it may have read.
    pub(super) sites: BTreeSet<usize>,
    /// Which of the two reads it is, which decides the code it is reported
    /// under.
    ///
    /// **In [`ReadKey`] as well**, because the place no longer keeps the two
    /// apart: `release(*tab)` dereferences `tab` and hands on what `*tab`
    /// holds, one place at one span. With one entry for both, what was handed
    /// was filed under the dereference and reported as `SC0402`. See ADR-0042.
    pub(super) read: Read,
    /// Which of `sites` code this check cannot read may reach by a route
    /// other than the call this read belongs to.
    ///
    /// **What a later call this check cannot read is asked about, beyond what
    /// it reaches itself.** A read's own call exposing a site does not make it
    /// reachable to a sibling call before the read, because C17 6.5.2.2 p10
    /// orders the call's arguments before its body: `keep(a) + release_all()`
    /// builds. A parameter, a store into an exposed allocation, or another
    /// call in the expression does, since any of them may run first. Filled
    /// by [`Known::meeting`](super::known::Known::meeting) and [`Known::noticed`](super::known::Known::noticed). See ADR-0042.
    pub(super) reachable: BTreeSet<usize>,
    /// Whether something made what this read read reachable to code this
    /// check cannot read while a call that code may be in was pending.
    ///
    /// **The order a forward walk cannot see from either end.** C may run the
    /// exposing event, then the call, then the read, so the call may free what
    /// was read; but the call was walked before the event and the read before
    /// both. [`Known::noticed`](super::known::Known::noticed) sets this at the event, and [`after_a_call`](super::report::after_a_call)
    /// reports it. See ADR-0042.
    pub(super) after_call: bool,
}

/// What a [`PendingRead`] is a read of.
///
/// Two reads of one pointer that a reader is told about in two different
/// codes: `SC0402` is a dereference and `SC0407` a pointer handed on, because
/// the fix for the second is at the call or at the free rather than at a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Read {
    /// `*p`, met by [`Known::met`](super::known::Known::met).
    Dereference,
    /// `g(p)`, met where the call's terminator is.
    Argument,
}

/// What one of them is filed under: where the read is, and what it read
/// through.
///
/// A position rather than the [`Span`] itself, because the map has to order its
/// keys and a span is not ordered. The file is the first part of it for
/// [`earlier`]'s reason.
///
/// **The place is the fourth part because this is the pair a report is
/// collapsed on**, and [`say`](super::report::say) says what each half of it costs when it goes.
/// Two reads at one span through one place are one report; two reads at one
/// span through two places are two. **Which read it is, the fifth**, for the
/// reason [`PendingRead::read`] gives.
///
/// The end as well as the start, so that two elements beginning at one column
/// and covering different extents are two reads rather than one. Nothing
/// reaches that today and ADR-0023 says so: dropping the end leaves the whole
/// suite green, and what it would cost is one of two reports rather than a
/// wrong one.
pub(super) type ReadKey = (usize, u32, u32, Place, Read);

/// Which allocations each local may hold, and what is known about each.
/// What a `realloc` with a non-zero constant size was handed, remembered on
/// the allocation it returned: where the call is, and each allocation it was
/// handed with where that one was made, all live at the call.
///
/// C17 7.22.3.5 p3 and p4: a failed `realloc` returns null and deallocates
/// nothing, a successful one deallocates the old object. Which happened is the
/// branch on the result, so the fact waits for it. With a size of zero, whether
/// a failed call deallocates is implementation-defined, so no fact is kept.
/// See ADR-0039.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Realloced {
    /// Where the `realloc` is, which is where the old allocation is freed on
    /// the arm that learns it succeeded.
    pub(super) at: Span,
    /// Each allocation it was handed, with where it was made.
    pub(super) old: Vec<(usize, Option<Span>)>,
}
