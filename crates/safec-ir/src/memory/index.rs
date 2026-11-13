//! The two kinds of index this check keeps tables by: an allocation site,
//! [`Site`], and a local, [`LocalIdx`], and a table indexed by one of them,
//! [`Table`].
//!
//! **Two types, so that one cannot be passed for the other.** A site is
//! named by the local the allocation was written into, so both used to be a
//! bare `usize`, and `contents[container]` with `container` a local or
//! `points_to[target]` with `target` a site compiled and answered about the
//! wrong object. Now passing one for the other does not compile, measured
//! by swapping each at a reader: an argument and a table of sites indexed by
//! a local are `error[E0308]`, and a table of locals indexed by a site is
//! `error[E0277]`, since that table is also indexed by the IR's `LocalId`.
//! And where the two really are the same object, as for a
//! parameter or a call's result, [`Site::of`] and [`Site::local`] say so
//! where it happens. See ADR-0063.
//!
//! **And one kind of place, [`Loc`], where a question is asked of both.** A
//! table of places is indexed by a site and by a local alike, so the two
//! index types guard nothing there; what does is that a reader answering
//! the two kinds apart is a `match` on [`Loc`], which a third kind would
//! have to answer. **And what a pointer names, [`Target`]**, a place or
//! something this check does not model, so that a table of what places
//! hold has no slot for the second.

use std::ops::{Index, IndexMut};

use crate::ir::LocalId;

pub(super) use crate::index::{Idx, LocalIdx, Table};

/// An allocation site: the local the call that made it wrote into, as
/// the module comment of `memory` says, but not that local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct Site(usize);

impl Site {
    /// The site an allocation written into `local` is named by.
    pub(super) fn of(local: LocalIdx) -> Site {
        Site(local.index())
    }

    /// The local this site's allocation was written into, which is what
    /// `Known::points_to` says it holds.
    pub(super) fn local(self) -> LocalIdx {
        LocalIdx::at(self.0)
    }
}

impl Idx for Site {
    fn index(self) -> usize {
        self.0
    }
    fn at(position: usize) -> Self {
        Site(position)
    }
}

/// One kind of place: an allocation site or a local, which this check used
/// to keep in two tables per question and answer twice (#779). **Not the
/// IR's [`Place`](crate::ir::Place)**, which is a local with a projection
/// that a statement reads or writes: this is what such a place may name.
///
/// **Interleaved**, `Heap(s)` at `2s` and `Local(l)` at `2l + 1`, so that a
/// [`Table`] of places needs no count of locals to be indexed and takes the
/// memory the two tables took. A reader that answers the two kinds
/// differently is a `match` on it, so a kind added is `error[E0004]` at
/// each. See ADR-0063.
///
/// **Storage, and nothing a pointer merely names**: what is written, how
/// wide it is, what part of a pointer it holds. What a pointer may name is
/// a [`Target`], which has a third kind, the place this check does not
/// model. Kept as one type, a table of what a place holds had a slot per
/// reason this check does not model, which no reader asked and any could
/// have written, and every reader of the places a pointer names filtered
/// the unknown ones out by hand (the memory model rebuilt on #924, found by
/// the architecture review of 2026-11-13).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Loc {
    /// An allocation site.
    Heap(Site),
    /// A local.
    Local(LocalIdx),
}

/// **What a pointer may name**: a place, or something this check does not
/// model, for the reason [`Why`] gives. A member of a points-to set like
/// any other, so a value that may name what was not recorded says so by
/// what it names, joined and poured with the rest of the set rather than by
/// a mark of its own carried beside it (the memory model rebuilt on #924,
/// stage 1). And what code this check cannot read may hold,
/// `Known::exposed`, a lost value among it.
///
/// `Unknown(why)` at `why`, then a [`Loc`] at its own index past them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Target {
    /// An allocation site.
    Heap(Site),
    /// A local.
    Local(LocalIdx),
    /// Something this check does not model.
    Unknown(Why),
}

/// One list of reasons, from which both the enum and [`Why::ALL`] are made,
/// so that a reason added is in both: written apart, a reason added to the
/// enum and not to the array compiled, and its place took the index of the
/// first allocation site (found by the architecture review of 2026-11-13).
macro_rules! reasons {
    ($(#[$enum_doc:meta])* enum Why { $($(#[$doc:meta])* $name:ident,)* }) => {
        $(#[$enum_doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(super) enum Why {
            $($(#[$doc])* $name,)*
        }

        impl Why {
            /// Every reason, in its place order: the order they are listed in,
            /// which is the order of their discriminants.
            pub(super) const ALL: &'static [Why] = &[$(Why::$name,)*];
        }
    };
}

reasons! {
/// Why a value may name a place this check does not model: each is one of
/// the marks a value carried before it was a place.
enum Why {
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
    Lost,
    /// Whether this local may hold a pointer read out of memory.
    ///
    /// Such a pointer holds what a cell's `names` records for the allocation
    /// it was read from, which may be missing members, and this bit is what
    /// says so: the report reads [`Reached::Partial`](super::parts::Reached::Partial) beside those sites, and
    /// a free of it stays a doubt (ADR-0045). **For the other readers**, what
    /// a call reaches and what a write stores, it says the value may be
    /// anything stored, for which "no site" had come to mean "reaches
    /// nothing". It is not the emptiness of [`Held::sites`](super::parts::Held::sites), because
    /// `int *z = 0;` is empty too, and reading it as a load exposed every
    /// stored pointer at `log_ptr(z)`, measured. See ADR-0040.
    ///
    /// It says that the value was read out of memory and not from where, so a
    /// write through it is unplaced for that part: what it carries is
    /// exposed. See ADR-0044.
    Loaded,
    /// Whether a call this check cannot read may have written into this
    /// local, through its address, something no site names.
    ///
    /// [`Known::replaced`](super::known::Known::replaced) cannot say this with [`Why::Lost`] for a local
    /// holding no site, because the report reads that bit and ADR-0017 does
    /// not report an output parameter. So `get(&u)` sets this instead, and
    /// the report never reads it. A write through such a local exposes what
    /// it carries, as one through a load does. See ADR-0044.
    Foreign,
    /// Whether this was read out of memory a pointer parameter points at, so
    /// may be what the caller stored there.
    ///
    /// Or handed over by a callee: read out of what a call this check cannot
    /// read returned, or written by one through an escaped local's address.
    /// Memory a callee hands back it may still hold, as a caller may (#394).
    ///
    /// **Made [`Why::Lost`] at a call this check cannot read.** What the
    /// caller stored behind `pp` is recorded nowhere, so a load of it holds no
    /// site and nothing a call does could reach it: `q = *pp; release_all();
    /// return *q;` built. The caller may have stashed it where the call frees
    /// it, which is ADR-0040's reason for exposing the parameter itself, one
    /// level in; and its cost, a read after any such call doubted, is the same
    /// one level in. See ADR-0040.
    FromCaller,
    /// Whether this was read out of an allocation whose cell is `stale_read`, or
    /// otherwise lost an allocation that may be gone, and may be one.
    ///
    /// Set too where a local loses a site whose allocation is not live
    /// ([`Known::reborn`](super::known::Known::reborn)), by a load through a pointer to such a local, and
    /// by a call this check cannot read for what it makes lost of the
    /// caller's, and on every lost local when one is freed ([`Known::lost_may_be_gone`](super::known::Known::lost_may_be_gone)). A
    /// store of any of them was recorded as nothing and read back
    /// in silence, where read directly it was doubted. See ADR-0045.
    ///
    /// **Not [`Why::Lost`], which says the same to the report and more to a
    /// store.** A local loses a site whenever the site is reborn, whatever
    /// became of the allocation it named, so `*node = head; head = node;`
    /// builds a list whose `head` is lost every turn though nothing is freed.
    /// A store of a lost value marking its container doubted every walk of
    /// that list, measured; a store of this one marks it, so a doubted load
    /// copied into another allocation stays doubted. See ADR-0045.
    StaleRead,
    /// Whether this was read out of an allocation whose cell is `lost`,
    /// which
    /// may hold a value this check lost, or built from such a value.
    ///
    /// **Not [`Why::Lost`] at the read**, which would doubt every walk of a
    /// list whose nodes hold a lost head. It is made lost only when a lost
    /// value may have gone, by
    /// [`Known::lost_may_be_gone`](super::known::Known::lost_may_be_gone):
    /// `*box = u; r = *box; free(u); v = *r;` with `u` lost to a rebirth read
    /// `*r` in silence, since `r` was read out before the free (#505). A
    /// store of it marks where it lands, as a store of a lost value does, so
    /// a copy into another allocation carries it. See ADR-0045.
    LostRead,
    /// Whether this was read out of an allocation whose cell is
    /// `unreplaced_read`, which may
    /// hold a slot a call this check cannot read could not replace, or built
    /// from such a value.
    ///
    /// **Not [`Why::StaleRead`], which makes a load lost.** A lost local's
    /// load is followed no further, so marking that way silenced `d = *c;
    /// return *d;` after `c = *h;`, a read of a proved free. This one is read
    /// by [`Known::handed_below`](super::known::Known::handed_below) alone.
    /// See ADR-0047.
    UnreplacedRead,
}
}

impl Loc {
    /// How many places a function with `locals` locals has: a site and a
    /// local per local, since a site is named by the local it landed in.
    pub(super) fn count(locals: usize) -> usize {
        2 * locals
    }
}

impl Idx for Loc {
    fn index(self) -> usize {
        match self {
            Loc::Heap(site) => 2 * site.index(),
            Loc::Local(local) => 2 * local.index() + 1,
        }
    }
    fn at(position: usize) -> Self {
        if position % 2 == 0 {
            Loc::Heap(Site::at(position / 2))
        } else {
            Loc::Local(LocalIdx::at(position / 2))
        }
    }
}

impl Target {
    /// The site this names, if it is one.
    pub(super) fn site(self) -> Option<Site> {
        match self {
            Target::Heap(site) => Some(site),
            Target::Local(_) | Target::Unknown(_) => None,
        }
    }

    /// The local this names, if it is one.
    pub(super) fn local(self) -> Option<LocalIdx> {
        match self {
            Target::Heap(_) | Target::Unknown(_) => None,
            Target::Local(local) => Some(local),
        }
    }

    /// The place this names, if it is one this check models.
    pub(super) fn place(self) -> Option<Loc> {
        match self {
            Target::Heap(site) => Some(Loc::Heap(site)),
            Target::Local(local) => Some(Loc::Local(local)),
            Target::Unknown(_) => None,
        }
    }

    /// How many targets a function with `locals` locals has: one unknown
    /// place per [`Why`], then every [`Loc`].
    pub(super) fn count(locals: usize) -> usize {
        Why::ALL.len() + Loc::count(locals)
    }
}

impl From<Loc> for Target {
    fn from(place: Loc) -> Target {
        match place {
            Loc::Heap(site) => Target::Heap(site),
            Loc::Local(local) => Target::Local(local),
        }
    }
}

impl Idx for Target {
    fn index(self) -> usize {
        match self {
            Target::Unknown(why) => why as usize,
            Target::Heap(site) => Why::ALL.len() + Loc::Heap(site).index(),
            Target::Local(local) => Why::ALL.len() + Loc::Local(local).index(),
        }
    }
    fn at(position: usize) -> Self {
        match position.checked_sub(Why::ALL.len()) {
            Some(position) => Target::from(Loc::at(position)),
            None => Target::Unknown(Why::ALL[position]),
        }
    }
}

/// An index a table is kept by that has a slot for every site and every
/// local: [`Loc`] and [`Target`]. So that a table of either is read by the
/// kind of place a reader holds, through one set of impls rather than two.
pub(super) trait ByPlace: Idx {
    /// Where the places start, past what is not one.
    const PLACES_FROM: usize;
    /// This index for a place.
    fn of_place(place: Loc) -> Self;
}

impl ByPlace for Loc {
    const PLACES_FROM: usize = 0;
    fn of_place(place: Loc) -> Self {
        place
    }
}

impl ByPlace for Target {
    const PLACES_FROM: usize = Why::ALL.len();
    fn of_place(place: Loc) -> Self {
        Target::from(place)
    }
}

/// A table of places, read by the kind of place a reader holds: a site, a
/// local, or the IR's own name for a local. One table where there were two
/// (#779), so a reader of a site and a reader of a local read the same
/// memory, each through its own index type.
///
/// An inherent impl per index, by a macro, since one generic over
/// [`ByPlace`] would be reachable wider than the trait is.
macro_rules! places_of {
    ($index:ty) => {
        impl<T> Table<$index, T> {
            /// Every site the table has, in order.
            pub(super) fn sites(&self) -> impl Iterator<Item = Site> + use<T> {
                (0..(self.len() - <$index as ByPlace>::PLACES_FROM) / 2).map(Site::at)
            }

            /// Every local the table has, in order.
            pub(super) fn locals(&self) -> impl Iterator<Item = LocalIdx> + use<T> {
                (0..(self.len() - <$index as ByPlace>::PLACES_FROM) / 2).map(LocalIdx::at)
            }
        }
    };
}

places_of!(Loc);
places_of!(Target);

impl<P: ByPlace, T> Index<Site> for Table<P, T> {
    type Output = T;
    fn index(&self, site: Site) -> &T {
        &self[P::of_place(Loc::Heap(site))]
    }
}

impl<P: ByPlace, T> IndexMut<Site> for Table<P, T> {
    fn index_mut(&mut self, site: Site) -> &mut T {
        &mut self[P::of_place(Loc::Heap(site))]
    }
}

impl<P: ByPlace, T> Index<LocalIdx> for Table<P, T> {
    type Output = T;
    fn index(&self, local: LocalIdx) -> &T {
        &self[P::of_place(Loc::Local(local))]
    }
}

impl<P: ByPlace, T> IndexMut<LocalIdx> for Table<P, T> {
    fn index_mut(&mut self, local: LocalIdx) -> &mut T {
        &mut self[P::of_place(Loc::Local(local))]
    }
}

impl<P: ByPlace, T> Index<LocalId> for Table<P, T> {
    type Output = T;
    fn index(&self, local: LocalId) -> &T {
        &self[P::of_place(Loc::Local(LocalIdx::of(local)))]
    }
}

impl<P: ByPlace, T> IndexMut<LocalId> for Table<P, T> {
    fn index_mut(&mut self, local: LocalId) -> &mut T {
        &mut self[P::of_place(Loc::Local(LocalIdx::of(local)))]
    }
}

impl<P: ByPlace, T> Index<&LocalId> for Table<P, T> {
    type Output = T;
    fn index(&self, local: &LocalId) -> &T {
        &self[P::of_place(Loc::Local(LocalIdx::of(local)))]
    }
}

impl<P: ByPlace, T> IndexMut<&LocalId> for Table<P, T> {
    fn index_mut(&mut self, local: &LocalId) -> &mut T {
        &mut self[P::of_place(Loc::Local(LocalIdx::of(local)))]
    }
}

#[cfg(test)]
mod tests {
    use super::{Idx, LocalIdx, Site, Target, Why};

    /// Every target has an index of its own, and the round trip through it is
    /// the identity: an unknown place for each reason, then a site and a local
    /// per local. Mutation: give `Target::Unknown` an index of `why as usize + 1`;
    /// this fails, as a reason sharing the first site's index once did.
    #[test]
    fn every_place_has_an_index_of_its_own() {
        let mut places: Vec<Target> = Why::ALL.iter().map(|&why| Target::Unknown(why)).collect();
        for at in 0..3 {
            places.push(Target::Heap(Site::at(at)));
            places.push(Target::Local(LocalIdx::at(at)));
        }
        let indices: Vec<usize> = places.iter().map(|&place| place.index()).collect();
        for (place, &index) in places.iter().zip(&indices) {
            assert_eq!(Target::at(index), *place);
        }
        let mut sorted = indices.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), indices.len(), "{indices:?}");
        assert_eq!(Target::count(3), indices.len());
    }
}
