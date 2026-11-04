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

use std::borrow::Borrow;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

use crate::ir::LocalId;

/// An index a [`Table`] or a [`Bits`](super::bits::Bits) is kept by.
///
/// [`Idx::at`] is how a table walks its own indices, and how a test names
/// one; nothing else is meant to call it, since an index built from a number
/// is the shape these types exist to stop.
pub(super) trait Idx: Copy + Ord {
    /// The position this index names.
    fn index(self) -> usize;
    /// The index at `position`, for a table walking its own.
    fn at(position: usize) -> Self;
}

/// What the memory check still keeps by a bare position: the elements of a
/// local array, [`Inits`](super::parts::Inits), which are neither sites nor
/// locals.
impl Idx for usize {
    fn index(self) -> usize {
        self
    }
    fn at(position: usize) -> Self {
        position
    }
}

/// An allocation site: the local the call that made it wrote into, as
/// the module comment of `memory` says, but not that local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct Site(usize);

impl Site {
    /// The site an allocation written into `local` is named by.
    pub(super) fn of(local: LocalIdx) -> Site {
        Site(local.0)
    }

    /// The local this site's allocation was written into, which is what
    /// `Known::points_to` says it holds.
    pub(super) fn local(self) -> LocalIdx {
        LocalIdx(self.0)
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

/// A local of the function this check walks, by its position.
///
/// Not the IR's `LocalId`, which cannot be built from a position outside
/// the IR, while this check's tables are walked by position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct LocalIdx(usize);

impl LocalIdx {
    /// The position the IR gave `local`, by value or by reference, since a
    /// walk of `Function::locals` hands its filters a reference.
    pub(super) fn of(local: impl Borrow<LocalId>) -> LocalIdx {
        LocalIdx(local.borrow().index())
    }
}

impl Idx for LocalIdx {
    fn index(self) -> usize {
        self.0
    }
    fn at(position: usize) -> Self {
        LocalIdx(position)
    }
}

/// One kind of place: an allocation site or a local, which this check used
/// to keep in two tables per question and answer twice (#779).
///
/// **Interleaved**, `Heap(s)` at `2s` and `Local(l)` at `2l + 1`, so that a
/// [`Bits`](super::bits::Bits) or a [`Table`] of places needs no count of
/// locals to be indexed and takes the memory the two tables took. A reader
/// that answers the two kinds differently is a `match` on it, so a kind
/// added is `error[E0004]` at each. See ADR-0063.
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

/// A `Vec` indexed by `I` alone.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Table<I, T> {
    items: Vec<T>,
    index: PhantomData<fn(I)>,
}

impl<I: Idx, T> Table<I, T> {
    /// A table of the values `items` holds, the first at index `0`.
    pub(super) fn from_vec(items: Vec<T>) -> Self {
        Table {
            items,
            index: PhantomData,
        }
    }

    /// How many indices the table has.
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    /// Every index the table has, in order.
    pub(super) fn indices(&self) -> impl Iterator<Item = I> + use<I, T> {
        (0..self.items.len()).map(I::at)
    }

    /// Every value, in index order.
    pub(super) fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    /// Every value, in index order, to change.
    pub(super) fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.items.iter_mut()
    }

    /// Every index with its value, in order.
    pub(super) fn enumerated(&self) -> impl Iterator<Item = (I, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(position, item)| (I::at(position), item))
    }

    /// Every index with its value, in order, to change.
    pub(super) fn enumerated_mut(&mut self) -> impl Iterator<Item = (I, &mut T)> {
        self.items
            .iter_mut()
            .enumerate()
            .map(|(position, item)| (I::at(position), item))
    }
}

impl<I: Idx, T: Clone> Table<I, T> {
    /// A table of `len` copies of `item`.
    pub(super) fn filled(item: T, len: usize) -> Self {
        Table::from_vec(vec![item; len])
    }

    /// Set every value to `item`.
    pub(super) fn fill(&mut self, item: T) {
        self.items.fill(item);
    }
}

impl<'a, I: Idx, T> IntoIterator for &'a Table<I, T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'a, I: Idx, T> IntoIterator for &'a mut Table<I, T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}

impl<I: Idx, T> Index<I> for Table<I, T> {
    type Output = T;
    fn index(&self, index: I) -> &T {
        &self.items[index.index()]
    }
}

impl<I: Idx, T> IndexMut<I> for Table<I, T> {
    fn index_mut(&mut self, index: I) -> &mut T {
        &mut self.items[index.index()]
    }
}

/// A local's table read by the IR's own name for it, which cannot be a
/// site's.
impl<T> Index<LocalId> for Table<LocalIdx, T> {
    type Output = T;
    fn index(&self, local: LocalId) -> &T {
        &self[LocalIdx::of(local)]
    }
}

impl<T> IndexMut<LocalId> for Table<LocalIdx, T> {
    fn index_mut(&mut self, local: LocalId) -> &mut T {
        &mut self[LocalIdx::of(local)]
    }
}

impl<T> Index<&LocalId> for Table<LocalIdx, T> {
    type Output = T;
    fn index(&self, local: &LocalId) -> &T {
        &self[LocalIdx::of(local)]
    }
}

impl<T> IndexMut<&LocalId> for Table<LocalIdx, T> {
    fn index_mut(&mut self, local: &LocalId) -> &mut T {
        &mut self[LocalIdx::of(local)]
    }
}

/// A table of places, read by the kind of place a reader holds: a site, a
/// local, or the IR's own name for a local. One table where there were two
/// (#779), so a reader of a site and a reader of a local read the same
/// memory, each through its own index type.
impl<T> Table<Loc, T> {
    /// Every site the table has, in order.
    pub(super) fn sites(&self) -> impl Iterator<Item = Site> + use<T> {
        (0..self.items.len() / 2).map(Site::at)
    }

    /// Every local the table has, in order.
    pub(super) fn locals(&self) -> impl Iterator<Item = LocalIdx> + use<T> {
        (0..self.items.len() / 2).map(LocalIdx::at)
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
