//! The lattice's parts: what a local may hold, [`Held`], and what it reached,
//! [`Reached`]; what is known about a site, [`SiteState`], and where in it a
//! pointer is, [`Offset`]; the facts carried until something answers them, a read
//! awaiting the free it is unordered against and a `realloc` awaiting its branch;
//! and what a call's name says, [`Callee`].
//!
//! [`Known`](super::known::Known) is the value they make up, in its own module.

use std::collections::BTreeSet;

use crate::ir::Place;
use crate::source::{SourceMap, Span};

use super::LostReason;
use super::bits::Bits;

/// What this check can read in a callee's name.
///
/// By name because nothing else is available: no annotation says what a
/// function does to what it is passed. The nullability specifiers say only
/// whether a pointer a function takes or returns may be null, which is
/// ADR-0037 and ADR-0050.
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
    /// One of [`DOES_NOT_RETURN`](crate::ir::DOES_NOT_RETURN): `abort`,
    /// `exit`, `_Exit` and `quick_exit`, believed by name as the lowering and
    /// the reserved-name check believe them (ADR-0051).
    ///
    /// **It touches nothing.** What it is handed is asked as an argument, so
    /// `exit(*p)` reads `*p`, but it exposes, frees and returns nothing. The
    /// lowering keeps its edge inside a larger full expression, and read as
    /// [`Callee::Opaque`] that edge carried a call that may have freed
    /// everything it could reach to code after it: `n > 0 || (abort(), 0);
    /// use(p);` doubted `p` (#586). No execution continues past it, so the
    /// value before it is the one that claims least beyond it.
    Ends,
    /// Anything else. It may free what it was passed and this cannot tell.
    Opaque,
}

/// How far a library call reaches through one pointer it is handed.
///
/// See [`LIBRARY`], which is the only place one is written.
#[derive(Clone, Copy)]
pub(super) enum Reach {
    /// As many bytes as the argument at this index says, read as the
    /// parameter receives it.
    Length(usize),
    /// As far as a string runs, which this check does not read, so it is
    /// never shown to fit.
    String,
}

/// How many bytes an allocation function makes, out of its arguments.
#[derive(Clone, Copy)]
pub(super) enum Makes {
    /// The argument at this index.
    Argument(usize),
    /// The product of the two arguments at these indices, as `calloc`'s.
    Product(usize, usize),
}

/// A library function this check reads by name.
///
/// **One row says everything that is read about the name**, and a row that
/// leaves a field out is `error[E0063]`: what it does to what it is handed,
/// for the memory transfer; how far it reaches through which argument, for
/// whether an access fits; and what it makes, where it allocates. Kept as
/// three readers matching names of their own, adding a function that takes
/// a length to the first and forgetting the second left `memmove(p, q, 64)`
/// on four bytes silent with every test green. Found by review. See
/// ADR-0060.
pub(super) struct Library {
    /// What the source calls it.
    pub(super) name: &'static str,
    /// What it does to what it is handed.
    pub(super) callee: Callee,
    /// How far it reaches through each argument it reads or writes through.
    pub(super) reaches: &'static [(usize, Reach)],
    /// How many bytes it makes, where it makes an allocation.
    pub(super) makes: Option<Makes>,
}

/// Every library function this check reads by name, with what each is read
/// as. A name not here is [`Callee::Opaque`] and reaches nothing this check
/// asks about: it is believed, as a parameter is.
///
/// The clauses: `free` 7.22.3.3, `malloc` 7.22.3.4, `calloc` 7.22.3.2,
/// `aligned_alloc` 7.22.3.1, `realloc` 7.22.3.5; `memcpy` 7.24.2.1,
/// `memmove` 7.24.2.2, `strcpy` 7.24.2.3, `strncpy` 7.24.2.4, which writes
/// exactly `n` characters (p2, p3), `strcat` 7.24.3.1, `strncat` 7.24.3.2,
/// which appends after a string whose length nothing here reads; `memcmp`
/// 7.24.4.1, `strcmp` 7.24.4.2, `strncmp` 7.24.4.4, which stops at a null
/// character before `n`; `memchr` 7.24.5.1, `strchr` 7.24.5.2, `strrchr`
/// 7.24.5.5, `strstr` 7.24.5.7; `memset` 7.24.6.1, `strlen` 7.24.6.3.
pub(super) const LIBRARY: &[Library] = &[
    Library {
        name: "free",
        callee: Callee::Frees,
        reaches: &[],
        makes: None,
    },
    Library {
        name: "malloc",
        callee: Callee::Allocates,
        reaches: &[],
        makes: Some(Makes::Argument(0)),
    },
    Library {
        name: "calloc",
        callee: Callee::Allocates,
        reaches: &[],
        makes: Some(Makes::Product(0, 1)),
    },
    Library {
        name: "aligned_alloc",
        callee: Callee::Allocates,
        reaches: &[],
        makes: Some(Makes::Argument(1)),
    },
    Library {
        name: "realloc",
        callee: Callee::Reallocates,
        reaches: &[],
        makes: Some(Makes::Argument(1)),
    },
    Library {
        name: "memcpy",
        callee: Callee::Copies,
        reaches: &[(0, Reach::Length(2)), (1, Reach::Length(2))],
        makes: None,
    },
    Library {
        name: "memmove",
        callee: Callee::Copies,
        reaches: &[(0, Reach::Length(2)), (1, Reach::Length(2))],
        makes: None,
    },
    Library {
        name: "memset",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::Length(2))],
        makes: None,
    },
    Library {
        name: "strcpy",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
    },
    Library {
        name: "strncpy",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::Length(2)), (1, Reach::String)],
        makes: None,
    },
    Library {
        name: "strcat",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
    },
    Library {
        name: "strncat",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
    },
    Library {
        name: "memcmp",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::Length(2)), (1, Reach::Length(2))],
        makes: None,
    },
    Library {
        name: "memchr",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::Length(2))],
        makes: None,
    },
    Library {
        name: "strlen",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String)],
        makes: None,
    },
    Library {
        name: "strcmp",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
    },
    Library {
        name: "strncmp",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
    },
    Library {
        name: "strchr",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String)],
        makes: None,
    },
    Library {
        name: "strrchr",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String)],
        makes: None,
    },
    Library {
        name: "strstr",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
    },
];

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
    pub(super) fn joined(self, other: Self, sources: &SourceMap) -> Self {
        Freeing {
            at: earlier(sources, self.at, other.at),
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
    /// cannot read: something this check saw may have freed it.
    Unknown,
    /// Named by nothing on this path: no local, allocation, pending read or
    /// call can reach it, so no execution's next use of it starts here.
    ///
    /// **The join's identity**, so that a path on which nothing holds a site
    /// says nothing about it where paths meet. ADR-0048's rule was that such a
    /// side takes the other side's state, and it was a special case in the join
    /// that asked whether each side held the site; as a state, it holds
    /// whatever arrived before, which the special case did not. `Live(None)`
    /// cannot be this, because a site that is live and was allocated where this
    /// check did not see is a fact, and it joins a span away. See ADR-0048.
    ///
    /// Set where a value crosses an edge, after liveness has cleared what the
    /// successor does not read. A reader reaches a site through something that
    /// holds it, so a reader meeting one means
    /// [`Known::held_sites`](super::known::Known::held_sites) missed a holder,
    /// and every reader takes it as it takes [`Self::Unknown`]. Taken as
    /// `Live`, that miss would be a proof that the allocation is live and a
    /// silence; taken as a doubt, it is a report the reader can see. No case
    /// in the corpus meets one, measured by panicking in every reader's arm
    /// for it, so **no test holds how a reader answers here**: these arms are
    /// where a miss in `held_sites` would land, and no program that makes one
    /// is known.
    Unheld,
    /// Live as far as this check saw, and within reach of code it cannot read:
    /// a local holding it had its address taken, or it was exposed to a call,
    /// or a call reached it only through an address. Nothing that may have
    /// freed it happened here, except a call that could also have replaced
    /// every local still read after it that holds it, which is what
    /// [`Known::held_out_of_reach`](super::known::Known::held_out_of_reach)
    /// asks.
    ///
    /// **A doubt everywhere `Unknown` is**, so no report says less for it.
    /// What tells the two apart is the one question a call handed an address
    /// asks, [`Known::handed_below`](super::known::Known::handed_below): what
    /// the address reaches is asked about when it is `Unknown`, and not when
    /// it is only `Reachable`, which every `use2(&a)` over a live pointer is.
    /// See ADR-0047.
    Reachable,
}

impl SiteState {
    /// The two together, which is `Unknown` unless they agree.
    ///
    /// `sources` for which free a joined state names, the earlier one by
    /// where the code is. See [`earlier`].
    pub(super) fn joined(self, other: Self, sources: &SourceMap) -> Self {
        match (self, other) {
            (Self::Unheld, other) | (other, Self::Unheld) => other,
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
                freed: from_here.joined(from_there, sources),
            },
            // Live on both sides as far as this check saw, and reachable on
            // at least one. See ADR-0047.
            (Self::Live(_) | Self::Reachable, Self::Live(_) | Self::Reachable) => Self::Reachable,
            _ => Self::Unknown,
        }
    }

    /// Something this check saw may have freed it: a call handed the pointer
    /// itself, a free it cannot pin down, a `realloc`. A proved free stays
    /// proved, since nothing un-frees an allocation.
    ///
    /// **A writer of a doubt that keeps a proved free goes through this or
    /// [`Self::doubted`]**, so that a new one has to choose between them rather
    /// than write `Unknown` over `Live` alone and leave a `Reachable` site
    /// looking unfreed. The few that write `Unknown` over anything, a proof
    /// included, need neither, since nothing is a stronger doubt. See
    /// ADR-0047.
    pub(super) fn may_be_freed(&mut self) {
        match self {
            Self::Live(_) | Self::Reachable => *self = Self::Unknown,
            Self::Freed { .. } | Self::Unknown | Self::Unheld => {}
        }
    }

    /// Code this check cannot read can now reach it, and nothing that may
    /// have freed it has happened: an address taken, an exposure, a call that
    /// reaches it only through an address. See ADR-0047.
    pub(super) fn doubted(&mut self) {
        match self {
            Self::Live(_) => *self = Self::Reachable,
            Self::Freed { .. } | Self::Unknown | Self::Reachable | Self::Unheld => {}
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

/// How many bytes the object a site stands for takes, as far as this check
/// read it.
///
/// **Two kinds of answer, because only one of them can prove an access
/// outside.** An allocation of a constant size is exactly that size, and an
/// access wider than it from its start lands outside on every execution. A
/// parameter and a call's result are only believed to point at one whole
/// object of the type they arrive as, which is a lower bound: an access wider
/// than that may still fit. `AtLeast(0)` is nothing read at all. See ADR-0060.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Extent {
    /// This many bytes, read off an allocation's constant size.
    Exactly(u64),
    /// At least this many bytes, and perhaps more.
    AtLeast(u64),
}

impl Extent {
    /// The fewest bytes it may take.
    pub(super) fn lower(self) -> u64 {
        match self {
            Self::Exactly(bytes) | Self::AtLeast(bytes) => bytes,
        }
    }

    /// The two together: itself where they agree, and the smaller lower
    /// bound where they do not.
    ///
    /// **It only descends**, and every value it can take is a constant the
    /// function wrote or the size of a type it names, so a loop cannot climb
    /// it: two different exact sizes meet as a bound below both, and a bound
    /// meets anything as the smaller bound. That is what ends a walk over it,
    /// rather than the walk's own limit. See ADR-0060.
    pub(super) fn joined(self, other: Self) -> Self {
        if self == other {
            self
        } else {
            Self::AtLeast(self.lower().min(other.lower()))
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
///
/// **By where the code is, across an instance.** A free in an included file
/// whose `#include` is above a free written in the including file is the
/// earlier one, as written out, though its file was registered later: the
/// order reads each span's chain from its root. See
/// [`SourceMap::position`] and ADR-0054.
pub(super) fn earlier(sources: &SourceMap, here: Span, there: Span) -> Span {
    if sources.position(here) <= sources.position(there) {
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
    /// callee. Each producer says why, as far as it can tell.
    Lost(LostReason),
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
    /// **One of five tables square in the locals**, with [`Held::writes_to`],
    /// [`Held::named_at_read`],
    /// [`Known::inside`](super::known::Known::inside) and
    /// [`Known::inside_locals`](super::known::Known::inside_locals), and the
    /// value holds one for every block and edge. So they are [`Bits`], a bit a
    /// pair, where a `Vec<bool>` spent a byte. Measured on a loop per write
    /// over 100 allocating locals, 401 blocks of 702 locals, release build:
    /// peak memory went from 2,025 MB to 427 MB and time from 2.29 s to
    /// 0.72 s once the four there were then were packed. Most of what is left is linear in the
    /// locals for every block, which is what `memory::LINEAR` counts. See
    /// ADR-0039.
    pub(super) sites: Bits,
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
    ///
    /// Two things set it: taking an address, `pp = &p;`, and a call this check
    /// cannot read, whose result may be any local whose address escaped. See
    /// ADR-0039.
    pub(super) writes_to: Bits,
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
    /// site would answer the same thing in every column, and a row per site
    /// would be one more table square in the locals.
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
    /// The locals code this check cannot read could name where this value
    /// was read out of memory, or where such code may have written it
    /// ([`Held::foreign`], #661), what
    /// [`Known::locals_named_by_unknown_code`](super::known::Known::locals_named_by_unknown_code)
    /// answered then. Empty where it was neither.
    ///
    /// **As of the read, because the value was fixed there.** A local whose
    /// address such code could find only later cannot be what was read, so a
    /// write through this lands in these and not in what is named by the
    /// time of the write: `t = *pp; *b = m; *t = 0;` does not write `m`
    /// (#648). The edges alone are not enough: memory a call wrote records
    /// nothing of what it wrote, so `keep(&slot, box); slot = m; t = *box[0];`
    /// has no edge to `m` though `t` may be it. A union where paths meet and
    /// through arithmetic, since one path may have read the value after the
    /// local became nameable. Read by
    /// [`Known::locals_a_write_may_land_in`](super::known::Known::locals_a_write_may_land_in)
    /// alone. See ADR-0057.
    pub(super) named_at_read: Bits,
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
    /// caller's, and on every lost local when one is freed ([`Known::lost_may_be_gone`](super::known::Known::lost_may_be_gone)). A
    /// store of any of them was recorded as nothing and read back
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
    /// Whether this was read out of an allocation
    /// [`Known::unreplaced`](super::known::Known::unreplaced) marks, which may
    /// hold a slot a call this check cannot read could not replace, or built
    /// from such a value.
    ///
    /// **Not [`Held::stale_read`], which makes a load lost.** A lost local's
    /// load is followed no further, so marking that way silenced `d = *c;
    /// return *d;` after `c = *h;`, a read of a proved free. This one is read
    /// by [`Known::handed_below`](super::known::Known::handed_below) alone.
    /// See ADR-0047.
    pub(super) unreplaced_read: bool,
    /// Whether this was read out of an allocation
    /// [`Known::holds_lost`](super::known::Known::holds_lost) marks, which
    /// may hold a value this check lost, or built from such a value.
    ///
    /// **Not [`Held::lost`] at the read**, which would doubt every walk of a
    /// list whose nodes hold a lost head. It is made lost only when a lost
    /// value may have gone, by
    /// [`Known::lost_may_be_gone`](super::known::Known::lost_may_be_gone):
    /// `*box = u; r = *box; free(u); v = *r;` with `u` lost to a rebirth read
    /// `*r` in silence, since `r` was read out before the free (#505). A
    /// store of it marks where it lands, as a store of a lost value does, so
    /// a copy into another allocation carries it. See ADR-0045.
    pub(super) lost_read: bool,
    /// Whether this was read out of memory a pointer parameter points at, so
    /// may be what the caller stored there.
    ///
    /// Or handed over by a callee: read out of what a call this check cannot
    /// read returned, or written by one through an escaped local's address.
    /// Memory a callee hands back it may still hold, as a caller may (#394).
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
    /// Whether this was read, on every path, out of an allocation
    /// [`Known::fresh`](super::known::Known::fresh) marks, so is
    /// indeterminate (C17 7.22.3.1 p2, 7.22.3.4 p2), and was reported
    /// where it was read. **It decides no report.** It is what drops the
    /// doubt a free or a `realloc` of the value would raise beside that
    /// report, which is the same defect again. Copied with the value and kept
    /// only where both paths agree. Arithmetic drops it, so `t + 1` is not
    /// followed. See ADR-0057.
    pub(super) fresh_read: bool,
    /// How many bytes this value is believed to point at the start of, where
    /// it is, on every path, a pointer read out of memory or returned by a
    /// call this check cannot read, and nothing since.
    ///
    /// **The size of what the pointer read points to, and not of what the
    /// local it lands in points to.** `void **pp; ... int *q = *pp;` reads a
    /// `void *`, which arrives believing nothing, and a `char`'s address
    /// stored that way and read back into `q` is not a whole `int`: believing
    /// `q`'s own type put four bytes on one. The conversion to and from
    /// `void *` needs no cast (C17 6.3.2.3 p1, 6.5.16.1 p1), so a type this
    /// frontend parses no cast for is still reached. A proof, so an intersection at a join, as [`Self::fresh_read`]
    /// is, and copied with the value and dropped by anything else, arithmetic
    /// included. What the sites a load holds say about where it points is
    /// not read for this, since they are held at an offset nobody said. See
    /// ADR-0060.
    pub(super) believed: Option<u64>,
    /// How many bytes each pointer stored in what this value points at is
    /// believed to point at, at most, and so on down, one [`Levels`] entry
    /// per level, with no bound where nothing bounds it beyond what this
    /// check recorded there.
    ///
    /// **Set from the type a pointer arrives as**, a parameter, a call's
    /// result or a load, which is a belief and not what C fixes: a `void **`
    /// is believed to hold `void *`s, which nobody was asked the width of,
    /// so `0`; an `int **` is believed to hold pointers to whole `int`s, so
    /// `4`. A `void **` that does point at an `int *` is refused when read
    /// through an `int **`, which is the cost of believing the type. A
    /// conversion
    /// to another pointer type keeps it, which is the point: `int **pi =
    /// pm;` with `pm` a `void **` parameter converted through `void *` read
    /// `*pi` as a whole `int` and wrote four bytes into one `char` (#632).
    /// A local's address and an allocator's result bound nothing here, since
    /// the local and the allocation's record are asked instead. **Every
    /// level the type has**: a `void ***` believes `8` and then `0`, so `int **a =
    /// *pi; int *b = *a;` with `pi` that pointer converted to `int ***` reads
    /// `b` believing nothing, where the type `a` is read as alone said four
    /// bytes (#634). A load takes what its memory believed one level down,
    /// and a store leaves this behind in the memory it lands in
    /// ([`Known::stored_at_most`](super::known::Known::stored_at_most)).
    /// **A bound and not a proof**, so a join keeps
    /// the lower of two and a side with none is no bound, the reverse of
    /// [`Self::believed`]. See ADR-0060.
    pub(super) contents_at_most: Levels,
    /// Whether an edge in [`Self::writes_to`] may be the address of
    /// something inside the local it names rather than of the local itself.
    ///
    /// **An edge can only name a local**, so `&*pp`, which C17 6.5.3.2 p3
    /// makes `pp`'s value, puts an edge on `pp`, and an access through it is
    /// not an access of `pp`'s eight bytes. Asking the local's size for one
    /// would answer about the wrong object. A constant zero index is the
    /// local's own start and does not set it, which is what the lowering
    /// builds to convert a local array to a pointer to its first element
    /// (#621); an array reached through a pointer is converted as a copy of
    /// the pointer instead, so no C reaches the `&*pp` shape, and it is here
    /// for the frontend that does not fold it, and for the first address of a
    /// member. A may-fact, so a union. See ADR-0060.
    pub(super) inner_address: bool,
    /// Which element of a local array the edges in [`Self::writes_to`] point
    /// at, counted in the array's innermost elements, where it is known:
    /// `&m[c]` with constant indices, and a constant move of such a pointer.
    /// `None` where it is not, and meaningless where there is no edge.
    ///
    /// **What lets one element be written and another stay unwritten**
    /// (#667). One record per array made a write to `m[0]` a write of `m[1]`,
    /// and a write through `m + 1` either all of `m` or none of it. A join
    /// keeps it where both paths agree. See `Known::unwritten_elements` and
    /// ADR-0057.
    pub(super) element: Option<u64>,
    /// Whether an edge to an array in [`Self::writes_to`] survives only by
    /// the array rule of [`Self::moved_by_arithmetic`], a constant move off
    /// the array's start that `main` dropped the edge for (#639).
    ///
    /// **What a write through it is, once [`Self::element`] is lost**: a
    /// write that may be of any element, and so leaves the array "may have
    /// been written" rather than written. Where the edge survives the way it
    /// always did, through an offset this check cannot read, the write is of
    /// the whole array, as it always was. That is the line between not losing
    /// a proof and not doubting every loop that fills an array. A union. See
    /// `Known::partly_written` and ADR-0057.
    pub(super) array_edge: bool,
    /// Whether a store placed through this pointer has written what it
    /// points at since it was given its value, on every path. A read through
    /// it then reads the element it wrote, whichever that is.
    ///
    /// **What keeps the lowering's own read of an assignment's value from
    /// being doubted** (#667): `*e = &x;` reads `*e` back as the value of the
    /// expression, and with `e` moved through a loop the element is not
    /// known, so the write left the array [`Known::partly_written`] and the
    /// read back of it was a doubt about the element just written. Held by
    /// the pointer, so a new value for it clears it and a copy of it keeps
    /// it. An intersection at a join. See `Known::may_be_unwritten`.
    ///
    /// [`Known::partly_written`]: super::known::Known::partly_written
    pub(super) stored_through: bool,
}

/// The lower of two bounds, where `None` is no bound: a side that bounds
/// nothing does not loosen one that does. See [`Held::contents_at_most`].
///
/// **A copy is one of these joins**: `pm = m;` builds `pm` from a value
/// with no bound and accumulates `m` into it, so a version that let a side
/// with none loosen the other lost every bound at its first copy. Mutation:
/// answer `None` where one side has no bound;
/// `a_void_pointer_to_pointer_parameter_read_through_an_int_alias_is_asked`
/// goes silent, among others.
pub(super) fn bounded(here: Option<u64>, there: Option<u64>) -> Option<u64> {
    match (here, there) {
        (Some(here), Some(there)) => Some(here.min(there)),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

/// How many bytes the pointers each level below a pointer's memory hold are
/// believed to point at, at most: index 0 for the pointers stored in that
/// memory, index 1 for the pointers stored where those point, and so on. A
/// level past the end, and an entry of `None`, is no bound. See
/// [`Held::contents_at_most`] and ADR-0060.
///
/// **No list is longer than [`Allocations::deepest_pointer`](super::transfer::Allocations::deepest_pointer)**,
/// the deepest pointer type among the function's locals. The two that build
/// one are [`Allocations::type_levels`](super::transfer::Allocations::type_levels),
/// which stops there, and [`Self::stored_under`], which drops what would pass it;
/// [`Self::below`] and a join make none longer than their inputs. So the
/// lattice this sits in has a finite height by construction, and no loop of
/// stores and loads can grow one for a fixpoint to chase. What is dropped is
/// a level no read in this function can ask, since a place dereferenced `d`
/// times to a pointer goes through a local at least `d + 1` deep and asks
/// index `d - 1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Levels(Vec<Option<u64>>);

impl Levels {
    /// One bound per level, from the shallowest, as given.
    pub(super) fn from_bounds(bounds: Vec<Option<u64>>) -> Self {
        Levels(bounds)
    }

    /// The bound at one level, `None` past the end.
    pub(super) fn at(&self, level: usize) -> Option<u64> {
        self.0.get(level).copied().flatten()
    }

    /// The lower of the two at every level, where `None` is no bound, as
    /// [`bounded`] says for one: a side that bounds nothing at a level does
    /// not loosen one that does, and a shorter list bounds nothing past its
    /// end.
    pub(super) fn bound_by(&mut self, other: &Levels) {
        if self.0.len() < other.0.len() {
            self.0.resize(other.0.len(), None);
        }
        for (level, here) in self.0.iter_mut().enumerate() {
            *here = bounded(*here, other.at(level));
        }
    }

    /// What the memory `levels` levels further down holds, as a pointer read
    /// that far down believes it: the first `levels` dropped.
    pub(super) fn below(&self, levels: usize) -> Levels {
        Levels(self.0.iter().skip(levels).copied().collect())
    }

    /// Whether memory believed `self` is narrower at some level than a
    /// reader of it believing `read` takes it to be: what a departure asks
    /// of the memory it lets leave. A level either side has no bound at asks
    /// nothing, and a reader's `0` is below nothing, so a receiver's type
    /// stopping short of the memory's asks nothing below it.
    pub(super) fn narrower_than(&self, read: &Levels) -> bool {
        self.0
            .iter()
            .zip(&read.0)
            .any(|pair| matches!(pair, (Some(held), Some(read)) if held < read))
    }

    /// What memory holding a pointer that believes `self` of its own memory
    /// holds: `first` for that pointer, which is what a store of it records
    /// at its own level, and `self` one level down, no longer than `depth`.
    pub(super) fn stored_under(&self, first: Option<u64>, depth: usize) -> Levels {
        Levels(
            std::iter::once(first)
                .chain(self.0.iter().copied())
                .take(depth)
                .collect(),
        )
    }
}

impl Held {
    /// Whether this local may hold a local's address, on some path.
    ///
    /// [`Held::writes_to`] is read for it, because a write through this local
    /// lands in a local exactly when it points at one, and the join keeps
    /// the edge where it keeps the sites the other path brought. A call
    /// handed such a pointer may be handed something that is no allocation
    /// at all, which a free or a `realloc` of it leaves undefined (C17
    /// 7.22.3.3 p2 and 7.22.3.5 p3), so neither may be read as freeing only
    /// the sites. See `Allocations::touching` and
    /// `Allocations::holds_something_unnameable`, which ask it.
    pub(super) fn may_be_a_locals_address(&self) -> bool {
        self.writes_to.any()
    }

    /// A local holding nothing, in a function with this many sites.
    pub(super) fn none(sites: usize) -> Self {
        Held {
            sites: Bits::new(sites),
            lost: false,
            freed: None,
            writes_to: Bits::new(sites),
            writes_elsewhere: true,
            // Nothing held, so nothing to be off the start of. `Unknown` here
            // would be a warning about an offset nobody wrote, on every path
            // that holds nothing and, through `Held::hold`'s join, on every
            // parameter. See ADR-0036.
            offset: Offset::Zero,
            loaded: false,
            named_at_read: Bits::new(sites),
            foreign: false,
            stale_read: false,
            unreplaced_read: false,
            lost_read: false,
            from_caller: false,
            returned_by: None,
            fresh_read: false,
            believed: None,
            contents_at_most: Levels::default(),
            inner_address: false,
            element: None,
            array_edge: false,
            stored_through: false,
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
        self.sites.set(site);
        self.offset = self.offset.joined(offset);
    }

    /// The one local this is certainly the address of: one address edge, and
    /// that edge all of what it may point at. `None` for anything less
    /// certain.
    ///
    /// **What an argument must be to hand a call
    /// [`Known::handed_away`](super::known::Known::handed_away)**, which
    /// exempts a holder and so has to hold rather than may. A load out of
    /// memory that names one local is not certain, `writes_elsewhere` says
    /// so, and neither is a local that may be either of two. See ADR-0047.
    ///
    /// **No program tells "either of two" from "the first of them"**, so the
    /// guard is the unit test `a_certain_target_is_one_edge_and_nothing_elsewhere`
    /// and not a corpus case. The second edge names a local whose address
    /// escaped, so the call may write into it whatever it reaches through the
    /// first, and that local counts as live after the call however the
    /// function goes on: it stays a holder of the first local's allocation
    /// that nothing exempts. Six programs answered the same with the first
    /// edge taken alone: `q = &a; if (c) q = &b; grow(q);` followed by a read
    /// of `a` by address, by value or by `free`, with `b` read again or not,
    /// handed to a call first, or a copy of `a`. A rule that stops counting
    /// such a local as live is one to ask this again under.
    pub(super) fn certain_target(&self) -> Option<usize> {
        if self.writes_elsewhere {
            return None;
        }
        let mut targets = self.writes_to.ones();
        match (targets.next(), targets.next()) {
            (Some(target), None) => Some(target),
            _ => None,
        }
    }

    /// Every site held, in order.
    pub(super) fn sites(&self) -> impl Iterator<Item = usize> + '_ {
        self.sites.ones()
    }

    /// Hold nothing, and hold nothing unnameable either.
    ///
    /// What a local is *given* replaces what it held, so a name it had lost
    /// goes with the rest. Keeping it would outlive the thing it was about:
    /// `keep = 0;` after `keep` lost its allocation would still answer
    /// [`Reached::Lost`] for every read, and a null dereference belongs to an
    /// axis this check has none for.
    ///
    /// **The edges go too, and only a unit test says so**,
    /// `clearing_a_local_leaves_it_no_site_and_no_edge`. An edge kept past
    /// `pp = &q;` or `pp = 0;` names a local whose address escaped, and this
    /// check doubts what such a local holds whatever is written into it. Six
    /// programs printed the same with the edges kept: a write through `pp`
    /// after `pp = &q;` read back through `p` or `q`, or `q` freed, and a free
    /// of `pp` after `pp = 0;` or after it was given an allocation.
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
            named_at_read,
            foreign,
            stale_read,
            unreplaced_read,
            lost_read,
            from_caller,
            returned_by,
            fresh_read,
            believed,
            contents_at_most,
            inner_address,
            element,
            array_edge,
            stored_through,
        } = self;
        sites.clear_all();
        *fresh_read = false;
        *believed = None;
        *contents_at_most = Levels::default();
        *inner_address = false;
        *element = None;
        *array_edge = false;
        *stored_through = false;
        *from_caller = false;
        *returned_by = None;
        *lost = false;
        *loaded = false;
        named_at_read.clear_all();
        *foreign = false;
        *stale_read = false;
        *unreplaced_read = false;
        *lost_read = false;
        *freed = None;
        writes_to.clear_all();
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
    pub(super) fn joined(&mut self, other: &Held, sources: &SourceMap) {
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
            named_at_read,
            foreign,
            stale_read,
            unreplaced_read,
            lost_read,
            from_caller,
            returned_by,
            fresh_read,
            believed,
            contents_at_most,
            inner_address,
            element,
            array_edge,
            stored_through,
        } = self;
        *inner_address = *inner_address || other.inner_address;
        // A bound, so the lower of two, and a side with none is no bound.
        contents_at_most.bound_by(&other.contents_at_most);
        if *returned_by != other.returned_by {
            *returned_by = None;
        }
        // A proof, so an intersection, as `freed` is.
        *fresh_read = *fresh_read && other.fresh_read;
        // A proof as well: believed where every path read it out of memory,
        // and then only as much as the path that believed least.
        *believed = match (*believed, other.believed) {
            (Some(here), Some(there)) => Some(here.min(there)),
            _ => None,
        };
        *offset = offset.joined(other.offset);
        *from_caller = *from_caller || other.from_caller;
        *loaded = *loaded || other.loaded;
        named_at_read.union_with(&other.named_at_read);
        *foreign = *foreign || other.foreign;
        *stale_read = *stale_read || other.stale_read;
        *unreplaced_read = *unreplaced_read || other.unreplaced_read;
        *lost_read = *lost_read || other.lost_read;
        // **A path that knows where a write through this local lands and a
        // path that does not is a path that does not.** This is what keeps a
        // strong update out of `if (c) { pp = &p; } *pp = q;`, where the
        // union below leaves one target and only this says the set is not
        // all of it. See ADR-0028.
        *writes_elsewhere = *writes_elsewhere || other.writes_elsewhere;
        sites.union_with(&other.sites);
        *lost = *lost || other.lost;
        // **An intersection, where every other field here is a union.** The
        // rest of this struct holds may-facts, which grow where paths meet.
        // This one is a proof, and a path that did not free proves nothing:
        // joining it the way its neighbours join would report a proved double
        // free on `if (c) { free(p); } free(p);`. See ADR-0020.
        *freed = match (*freed, other.freed) {
            (Some(here), Some(there)) => Some(here.joined(there, sources)),
            _ => None,
        };
        // The element where both paths agree, and a side with no edge
        // says nothing about it. Before the union below. See `Held::element`.
        *element = joined_element(writes_to.any(), *element, &other.writes_to, other.element);
        *array_edge = *array_edge || other.array_edge;
        *stored_through = *stored_through && other.stored_through;
        writes_to.union_with(&other.writes_to);
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
            named_at_read,
            foreign,
            stale_read,
            unreplaced_read,
            lost_read,
            from_caller,
            returned_by,
            fresh_read,
            believed,
            contents_at_most,
            inner_address,
            element,
            array_edge,
            stored_through,
        } = self;
        *inner_address = *inner_address || other.inner_address;
        // `*tab + 1` points into the same memory, which holds what it held.
        contents_at_most.bound_by(&other.contents_at_most);
        *from_caller = *from_caller || other.from_caller;
        // Arithmetic builds a new value, which is no longer exactly what a
        // `realloc` returned. See ADR-0039.
        *returned_by = None;
        // Nor the start of what was read, so nothing about it is believed.
        // See ADR-0060.
        *believed = None;
        // Nor certainly what was read: not followed. **No case holds this**,
        // measured: arithmetic builds on `Held::none`, and a store into a
        // local through its address is into one whose address escaped,
        // which the report does not ask.
        *fresh_read = false;

        // `*tab + 1` is built from a pointer read out of memory, and is one.
        *loaded = *loaded || other.loaded;
        named_at_read.union_with(&other.named_at_read);
        *foreign = *foreign || other.foreign;
        *stale_read = *stale_read || other.stale_read;
        *unreplaced_read = *unreplaced_read || other.unreplaced_read;
        *lost_read = *lost_read || other.lost_read;

        // **Where the result points is the fold's to say, and not this
        // method's**, for the reason the line below gives about the proof:
        // what decides it is which operand was the pointer and what the other
        // one was, and this sees neither. [`built_from`] assigns over it. The
        // other callers write into a local through a pointer, which joins
        // where the target and the value written start over it, or copy
        // into its address with `memcpy`, which is unknown either way. See
        // ADR-0036.
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
        sites.union_with(&other.sites);
        *lost = *lost || other.lost;
        // As the join keeps it, before the union. See `Held::element`.
        *element = joined_element(writes_to.any(), *element, &other.writes_to, other.element);
        *array_edge = *array_edge || other.array_edge;
        // A pointer built from others points at an element none of them was
        // known to have written. See `Held::stored_through`.
        *stored_through = false;
        writes_to.union_with(&other.writes_to);
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
        if self.sites.get(site) {
            self.sites.remove(site);
            self.lost = true;
        }
    }

    /// What pointer arithmetic leaves of the edge to a local, where
    /// `is_array` says which locals are arrays.
    ///
    /// **Kept where the offset may be zero, and dropped where it is known not
    /// to be, for a local that is not an array.** C17 6.5.6 p8 treats such a
    /// local as an array of one, so `&slot + n` may be dereferenced only for
    /// `n` of zero: a pointer this check moved by an offset it cannot read may
    /// be `slot`, and dereferenced it is `slot` or undefined. `po[k - 1]` read
    /// nothing of `slot` while `*po` read it. A non-zero constant, `pp[1] =
    /// q;`, is not `slot`, and following that edge reported a proved use after
    /// free about an allocation nothing had freed.
    ///
    /// **Kept for an array whatever the offset**, since the same paragraph
    /// lets a pointer into an array move to any element of it: `&m[0] + 1` is
    /// `m[1]`. Dropped, a pointer stored there, freed and read back was not
    /// named as used after it was freed, and a store through it wrote no
    /// local, so a read of the array it did write was a certain `SC0411`
    /// (#639). The edge names the array and not the element, as it does for a
    /// pointer at its start, so a free through one element is answered for
    /// the whole array. Past the end is the bounds rule's to report
    /// (ADR-0060). See ADR-0019.
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
    /// edge is gone before `back` could keep it, for a local that is not an
    /// array.
    pub(super) fn moved_by_arithmetic(
        &mut self,
        is_array: impl Fn(usize) -> bool,
        by: Option<i128>,
    ) {
        if !matches!(self.offset, Offset::Unknown) {
            let kept: Vec<usize> = self
                .writes_to
                .ones()
                .filter(|&local| is_array(local))
                .collect();
            // **Kept by the array rule alone**, where the move is known not
            // to be zero: `main` dropped this edge. See `Held::array_edge`.
            if matches!(self.offset, Offset::NonZero) && !kept.is_empty() {
                self.array_edge = true;
            }
            self.writes_to.clear_all();
            for local in kept {
                self.writes_to.set(local);
            }
        }
        // **The element moves by what was added**, where both are known: `by`
        // is the constant counted in the array's innermost elements. See
        // `Held::element`.
        self.element = match (self.element, by) {
            (Some(element), Some(by)) => u64::try_from(i128::from(element) + by).ok(),
            _ => None,
        };
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

/// The element two values point at once joined, where `here` and `there`
/// are each meaningful only beside an edge: a side with none says nothing,
/// and two that disagree know nothing. See [`Held::element`].
fn joined_element(
    here_has_edge: bool,
    here: Option<u64>,
    there_edges: &Bits,
    there: Option<u64>,
) -> Option<u64> {
    match (here_has_edge, there_edges.any()) {
        (false, _) => there,
        (true, false) => here,
        (true, true) if here == there => here,
        (true, true) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Extent, Freeing, Held, Levels, Offset, SiteState, earlier};
    use crate::memory::report::inside;
    use crate::source::{FileId, SourceMap, Span};

    /// A store cannot make a list longer than the function's depth, which is
    /// what keeps the lattice [`Levels`] sits in finite in height.
    ///
    /// A unit test and not a corpus case, because no C program found grows
    /// one past it: a store adds a level and a load takes one away, so a
    /// loop of the two holds its length. The cap is what makes that a
    /// property of the type rather than of every program. Mutation: drop the
    /// `take` in `Levels::stored_under`. This fails at the first assertion.
    #[test]
    fn a_store_cannot_make_a_list_longer_than_the_function_s_depth() {
        let held = Levels::from_bounds(vec![Some(8), Some(0)]);
        assert_eq!(
            held.stored_under(Some(4), 2),
            Levels::from_bounds(vec![Some(4), Some(8)])
        );
        assert_eq!(
            held.stored_under(None, 3),
            Levels::from_bounds(vec![None, Some(8), Some(0)])
        );
    }

    /// A shorter list bounds nothing past its end, and a side with no bound
    /// at a level does not loosen the other's, at every level.
    ///
    /// Mutation: stop at the shorter list's end. This fails at the first
    /// assertion, which loses the `0` at index 1.
    #[test]
    fn a_shorter_list_bounds_nothing_past_its_end() {
        let mut here = Levels::from_bounds(vec![Some(8)]);
        here.bound_by(&Levels::from_bounds(vec![None, Some(0)]));
        assert_eq!(here, Levels::from_bounds(vec![Some(8), Some(0)]));
        here.bound_by(&Levels::from_bounds(vec![Some(4)]));
        assert_eq!(here, Levels::from_bounds(vec![Some(4), Some(0)]));
    }

    /// Two sizes for one object meet as a bound below both, and one size
    /// meets itself unchanged.
    ///
    /// A unit test and not a corpus case, because no C program brings two
    /// sizes for one site: a site is one call's destination or one
    /// parameter, made with the same size every time.
    ///
    /// Mutation: keep the larger lower bound. This fails at the first
    /// assertion, and an access of four bytes into an object that may be one
    /// would be proved to fit. Mutation: drop the arm for two equal sizes.
    /// This fails at the second, and an exact size could never be proved
    /// outside after a loop.
    #[test]
    fn two_sizes_for_one_object_meet_as_a_bound_below_both() {
        assert_eq!(
            Extent::Exactly(4).joined(Extent::Exactly(1)),
            Extent::AtLeast(1)
        );
        assert_eq!(
            Extent::Exactly(4).joined(Extent::Exactly(4)),
            Extent::Exactly(4)
        );
        assert_eq!(
            Extent::AtLeast(8).joined(Extent::Exactly(2)),
            Extent::AtLeast(2)
        );
    }

    /// What a local is given replaces every site and every edge it had, and
    /// every mark about how it came by them.
    ///
    /// Mutation: have `clear` leave `writes_to`, `sites`, `offset`,
    /// `lost_read`, `unreplaced_read`, `foreign` or `freed`. This fails. The
    /// marks beside those have corpus cases of their own.
    #[test]
    fn clearing_a_local_leaves_it_no_site_and_no_edge() {
        let mut held = Held::none(4);
        held.sites.set(1);
        held.writes_to.set(3);
        held.writes_elsewhere = false;
        held.offset = Offset::NonZero;
        held.lost_read = true;
        held.unreplaced_read = true;
        held.foreign = true;
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "free(p);");
        held.freed = Some(Freeing {
            at: Span::new(file, 0, 7),
            sequenced: true,
        });
        held.clear();
        assert!(held.sites().next().is_none());
        assert!(!held.may_be_a_locals_address());
        assert!(held.writes_elsewhere);
        assert_eq!(held.offset, Offset::Zero);
        assert!(!held.lost_read);
        assert!(!held.unreplaced_read);
        assert!(!held.foreign);
        assert!(held.freed.is_none());
    }

    /// One edge that is all of where a write lands is one certain local, and
    /// two edges, one with a write that may land elsewhere, or none at all,
    /// are none.
    ///
    /// Mutation: have `certain_target` take only the first edge. The second
    /// assertion fails. Mutation: ignore `writes_elsewhere`. The third fails.
    /// Mutation: answer some local for no edge at all. The fourth fails.
    #[test]
    fn a_certain_target_is_one_edge_and_nothing_elsewhere() {
        let mut one = Held::none(4);
        one.writes_to.set(2);
        one.writes_elsewhere = false;
        assert_eq!(one.certain_target(), Some(2));

        let mut two = one.clone();
        two.writes_to.set(3);
        assert_eq!(two.certain_target(), None);

        let mut elsewhere = one.clone();
        elsewhere.writes_elsewhere = true;
        assert_eq!(elsewhere.certain_target(), None);

        let mut none = Held::none(4);
        none.writes_elsewhere = false;
        assert_eq!(none.certain_target(), None);
    }

    /// `main.c`, `x = 0;` then `#include "a.h"` then `y = 1;`, and `a.h`,
    /// `int z;`, included at that directive with its extent recorded. Written
    /// to a directory named by `tag`, so tests can run at once.
    fn included(tag: &str) -> (SourceMap, FileId, FileId) {
        let directory =
            std::env::temp_dir().join(format!("safec_parts_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("a.h"), "int z;").unwrap();
        let mut sources = SourceMap::new();
        let main = sources.add_virtual("main.c", "x = 0;\n#include \"a.h\"\ny = 1;\n");
        let a = sources
            .include(directory.join("a.h"), Span::new(main, 7, 21))
            .unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        sources.set_extent(a, Span::new(a, 0, 3), Span::new(a, 5, 6));
        (sources, main, a)
    }

    /// A read in an included file is inside a call that holds its
    /// `#include`, as it is written out, and the equal-span rule still holds
    /// of the spans themselves. See ADR-0043 and ADR-0054.
    ///
    /// Mutation: compare in `inside` without locating. The first assertion
    /// fails.
    #[test]
    fn a_read_in_an_included_file_is_inside_the_call_that_holds_its_include() {
        let (sources, main, a) = included("inside");
        let call = Span::new(main, 0, 28);

        assert!(inside(&sources, Span::new(a, 4, 5), call));
        assert!(!inside(&sources, Span::new(a, 4, 5), Span::new(main, 0, 6)));
        assert!(!inside(&sources, call, call));
    }

    /// A span that locates to exactly the outer one is not inside it, so a
    /// call that is exactly an instance's use takes no read in the instance
    /// for its own, and the read is reported. See ADR-0043.
    ///
    /// Mutation: ask equality of the spans before locating them. The read in
    /// `a.h` is then inside the call that is its `#include`.
    #[test]
    fn a_span_located_to_exactly_the_outer_one_is_not_inside_it() {
        let (sources, main, a) = included("located_equal");

        assert!(!inside(
            &sources,
            Span::new(a, 4, 5),
            Span::new(main, 7, 21)
        ));
    }

    /// A free in an included file whose `#include` is above a free written
    /// in the including file is the earlier, though its file was registered
    /// later.
    ///
    /// Mutation: order by file index first, as `earlier` did. The first
    /// assertion fails.
    #[test]
    fn a_free_in_an_included_file_above_a_written_one_is_the_earlier() {
        let (sources, main, a) = included("earlier");
        let included_free = Span::new(a, 4, 5);
        let written_free = Span::new(main, 22, 23);

        assert_eq!(
            earlier(&sources, written_free, included_free),
            included_free
        );
        assert_eq!(
            earlier(&sources, Span::new(main, 0, 1), included_free),
            Span::new(main, 0, 1)
        );
    }

    /// `SiteState::Unheld` joined with any state, from either side, is that
    /// state.
    ///
    /// That is what makes a path on which nothing holds a site say nothing
    /// about it where paths meet, whichever path arrived first, which is the
    /// whole of what ADR-0048's rule asks of the join.
    ///
    /// Mutation: answer `Unknown` for `Unheld` joined with anything. This
    /// fails, and so do the free-and-null cases in the corpus.
    #[test]
    fn a_site_nothing_holds_is_the_identity_of_the_join() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int f(void) { return 0; }\n");
        let made = Span::new(file, 0, 3);
        let freed = Span::new(file, 4, 5);

        for state in [
            SiteState::Live(None),
            SiteState::Live(Some(made)),
            SiteState::Freed {
                made: Some(made),
                freed: Freeing::new(freed),
            },
            SiteState::Unknown,
            SiteState::Reachable,
            SiteState::Unheld,
        ] {
            assert!(SiteState::Unheld.joined(state, &sources) == state);
            assert!(state.joined(SiteState::Unheld, &sources) == state);
        }
    }
}
