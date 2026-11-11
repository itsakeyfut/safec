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
//! have to answer.

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
/// [`Bits`](super::bits::Bits) or a [`Table`] of places needs no count of
/// locals to be indexed and takes the memory the two tables took. A reader
/// that answers the two kinds differently is a `match` on it, so a kind
/// added is `error[E0004]` at each: measured, at the five there are, the
/// two projections below, [`Idx::index`], and the unwritten belief's join
/// and back edge. See ADR-0063.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Loc {
    /// An allocation site.
    Heap(Site),
    /// A local.
    Local(LocalIdx),
}

impl Loc {
    /// The site this place is, if it is one.
    pub(super) fn site(self) -> Option<Site> {
        match self {
            Loc::Heap(site) => Some(site),
            Loc::Local(_) => None,
        }
    }

    /// The local this place is, if it is one.
    pub(super) fn local(self) -> Option<LocalIdx> {
        match self {
            Loc::Heap(_) => None,
            Loc::Local(local) => Some(local),
        }
    }

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

/// A table of places, read by the kind of place a reader holds: a site, a
/// local, or the IR's own name for a local. One table where there were two
/// (#779), so a reader of a site and a reader of a local read the same
/// memory, each through its own index type.
impl<T> Table<Loc, T> {
    /// Every site the table has, in order.
    pub(super) fn sites(&self) -> impl Iterator<Item = Site> + use<T> {
        (0..self.len() / 2).map(Site::at)
    }

    /// Every local the table has, in order.
    pub(super) fn locals(&self) -> impl Iterator<Item = LocalIdx> + use<T> {
        (0..self.len() / 2).map(LocalIdx::at)
    }
}

impl<T> Index<Site> for Table<Loc, T> {
    type Output = T;
    fn index(&self, site: Site) -> &T {
        &self[Loc::Heap(site)]
    }
}

impl<T> IndexMut<Site> for Table<Loc, T> {
    fn index_mut(&mut self, site: Site) -> &mut T {
        &mut self[Loc::Heap(site)]
    }
}

impl<T> Index<LocalIdx> for Table<Loc, T> {
    type Output = T;
    fn index(&self, local: LocalIdx) -> &T {
        &self[Loc::Local(local)]
    }
}

impl<T> IndexMut<LocalIdx> for Table<Loc, T> {
    fn index_mut(&mut self, local: LocalIdx) -> &mut T {
        &mut self[Loc::Local(local)]
    }
}

impl<T> Index<LocalId> for Table<Loc, T> {
    type Output = T;
    fn index(&self, local: LocalId) -> &T {
        &self[Loc::Local(LocalIdx::of(local))]
    }
}

impl<T> IndexMut<LocalId> for Table<Loc, T> {
    fn index_mut(&mut self, local: LocalId) -> &mut T {
        &mut self[Loc::Local(LocalIdx::of(local))]
    }
}

impl<T> Index<&LocalId> for Table<Loc, T> {
    type Output = T;
    fn index(&self, local: &LocalId) -> &T {
        &self[Loc::Local(LocalIdx::of(local))]
    }
}

impl<T> IndexMut<&LocalId> for Table<Loc, T> {
    fn index_mut(&mut self, local: &LocalId) -> &mut T {
        &mut self[Loc::Local(LocalIdx::of(local))]
    }
}
