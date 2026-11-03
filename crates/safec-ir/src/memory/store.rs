//! What one write of a value into a place is, however it was spelled.
//!
//! An assignment and a library copy both put a value where code may read it,
//! and the memory check asks of each what leaves, what is freed behind it,
//! how wide it is, and what it stores out. Each of those questions once read
//! the element or the call itself and decided on its own which writes it was
//! about, so a kind of write one of them did not know was a write none of
//! the others noticed it missing: the library copy never marked what it
//! carried stored out (#684). Every reader now takes a [`Store`] and matches
//! its [`StoreKind`] with no wildcard, so a kind added later is
//! `error[E0004]` at each of them (#688).

use crate::ir::{Element, Operand, Place, Projection, Rvalue, Terminator};
use crate::library::Callee;
use crate::source::Span;

use super::allocations::Allocations;
use super::known::Known;
use crate::reads::derefs;

/// One write of a value into a place.
pub(super) struct Store<'a> {
    /// Where it lands: an assignment's destination, or `*dest` for a
    /// library copy.
    pub(super) into: Place,
    /// What it writes.
    pub(super) from: Written<'a>,
    /// Which kind of write it is.
    pub(super) kind: StoreKind,
    /// Whether code outside this function may read what it writes: for a
    /// direct write, whether such code names the local; for a write through
    /// at least one dereference and for a copy, [`Known::leaves`] of
    /// [`Self::into`]; and `false` for a projection with no dereference,
    /// which lands in this function's own object and which `Known::leaves`
    /// cannot be asked of.
    pub(super) leaves: bool,
    /// Where it is, which a finding about it points at.
    pub(super) at: Span,
}

/// What a store writes.
pub(super) enum Written<'a> {
    /// An assignment's value.
    Value(&'a Rvalue),
    /// The bytes of the object a library copy's source points at: the
    /// source pointer itself, as the call was handed it.
    Bytes(&'a Place),
}

/// Which kind of write a store is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum StoreKind {
    /// An assignment to a local by name.
    Direct,
    /// An assignment through `derefs` dereferences, or through a projection
    /// with none. No C reaches `derefs: 0` today, since the lowering reaches
    /// an element or a member through its address; it is here for a
    /// frontend that does not, and every reader that asks of a store that
    /// leaves asks for at least one dereference.
    Through {
        /// How many dereferences the destination goes through.
        derefs: usize,
    },
    /// A library copy, `memcpy` or `memmove`, of bytes C defines to copy
    /// (C17 7.24.2.1, 7.24.2.2, 6.2.6.1 p4), and so asked as a doubt and
    /// never as a proof (#652).
    Copy,
}

impl Allocations<'_> {
    /// The store an element is, if it is one: an assignment, of kind
    /// [`StoreKind::Direct`] or [`StoreKind::Through`] and never
    /// [`StoreKind::Copy`].
    pub(super) fn store_in<'e>(&self, element: &'e Element, known: &Known) -> Option<Store<'e>> {
        let Element::Assign(operation) = element else {
            return None;
        };
        let into = operation.place.clone();
        let (kind, leaves) = if into.projection.is_empty() {
            let leaves = known
                .locals_named_by_unknown_code()
                .contains(&into.local.index());
            (StoreKind::Direct, leaves)
        } else {
            let derefs = derefs(&into);
            (
                StoreKind::Through { derefs },
                derefs > 0 && known.leaves(&into),
            )
        };
        Some(Store {
            into,
            from: Written::Value(&operation.value),
            kind,
            leaves,
            at: operation.origin.span(),
        })
    }

    /// The store a terminator is, if it is one: a call to an undefined
    /// [`Callee::Copies`] function handed a place for each of its first two
    /// arguments, always of kind [`StoreKind::Copy`]. A copy whose
    /// destination is no place makes none, so a reader that asks only of the
    /// source does not come here (`report::departed_at_call`). A body defined here is what runs, whatever it is called,
    /// as the departure door reads one.
    pub(super) fn store_at<'t>(
        &self,
        terminator: &'t Terminator,
        known: &Known,
    ) -> Option<Store<'t>> {
        let Terminator::Call {
            callee,
            arguments,
            origin,
            ..
        } = terminator
        else {
            return None;
        };
        if self.unit.function(*callee).is_defined()
            || !matches!(self.callee(*callee), Callee::Copies)
        {
            return None;
        }
        let (Some(Operand::Copy(to)), Some(Operand::Copy(from))) =
            (arguments.first(), arguments.get(1))
        else {
            return None;
        };
        let mut into = to.clone();
        into.projection.push(Projection::Deref);
        let leaves = known.leaves(&into);
        Some(Store {
            into,
            from: Written::Bytes(from),
            kind: StoreKind::Copy,
            leaves,
            at: origin.span(),
        })
    }
}

/// Where one write lands: the allocations and the locals it may write, and
/// whether it may land where this check cannot place it. Computed by
/// [`Allocations::landing`] for a store and for a library copy alike, since
/// a copy into `*dest` is a store at one dereference more, so that the two
/// doors cannot answer one destination differently (#737).
pub(super) struct Landing {
    /// The allocations it may land in, recorded there by `Known::pour`.
    pub(super) containers: Vec<usize>,
    /// The locals it may land in by an edge: the pointer's own edges at
    /// one exact dereference, and the locals the level above may be deeper.
    pub(super) targets: Vec<usize>,
    /// Whether it is one exact dereference through the pointer: then
    /// [`Self::targets`] are the pointer's own edges.
    pub(super) one_step: bool,
    /// The pointer the write goes through, and how many dereferences down
    /// it lands.
    pub(super) pointer: crate::ir::LocalId,
    pub(super) deep: usize,
    /// Which kind of write it is, which the doors' differences are matched on.
    pub(super) kind: StoreKind,
}

impl Landing {
    /// Whether the pointer may hold what this check did not follow, or the
    /// write is deep enough that it is taken as unplaced. See ADR-0044.
    ///
    /// **Asked where each door asks it**, as `Allocations::land_contents`
    /// begins, and not when the landing is made: a copy and a deeper store
    /// have written into their targets by then, which may change the
    /// pointer's own value, and a store through one dereference writes its
    /// targets after.
    ///
    /// **The depth at which a write is unnamed differs between the doors**,
    /// and is kept as it was: a store deeper than one dereference, a copy
    /// deeper than two, which is a copy whose destination itself has more
    /// than one dereference. Found while designing #737, and not shown to
    /// decide a case; this match is where a change to either answers.
    pub(super) fn unnamed(&self, value: &Known) -> bool {
        let deep_enough = match self.kind {
            StoreKind::Direct | StoreKind::Through { .. } => self.deep > 1,
            StoreKind::Copy => self.deep > 2,
        };
        let held = &value.points_to[self.pointer.index()];
        held.loaded || held.lost || held.foreign || deep_enough
    }

    /// Whether it lands nowhere this check names: no allocation and no local.
    ///
    /// **Which locals count differs between the doors**, and is kept as it
    /// was: a store counts only the pointer's own edges, at one exact
    /// dereference, and a copy counts the locals the level above may be as
    /// well. Found while implementing #737, and not shown to decide a case.
    pub(super) fn unplaced(&self) -> bool {
        let no_local = match self.kind {
            StoreKind::Direct | StoreKind::Through { .. } => {
                !self.one_step || self.targets.is_empty()
            }
            StoreKind::Copy => self.targets.is_empty(),
        };
        self.containers.is_empty() && no_local
    }

    /// Whether an allocation it lands in is reachable to code this check
    /// cannot read, which another parameter may name (#489). As of when it
    /// is asked, for the reason [`Self::unnamed`] gives.
    pub(super) fn handed_over(&self, value: &Known) -> bool {
        let reachable = value.reachable_now();
        self.containers
            .iter()
            .any(|container| reachable.contains(container))
    }
}

impl Allocations<'_> {
    /// Where a write of `kind` through `pointer`, `deep` dereferences down,
    /// lands. `one_step` is whether the place is exactly one dereference of
    /// the local; a copy's destination with no projection is that.
    pub(super) fn landing(
        &self,
        value: &Known,
        pointer: crate::ir::LocalId,
        deep: usize,
        one_step: bool,
        kind: StoreKind,
    ) -> Landing {
        let containers: Vec<usize> = if one_step {
            value.sites_of(pointer).collect()
        } else if deep > 1 {
            value.stored_below(pointer, deep - 1).into_iter().collect()
        } else {
            Vec::new()
        };
        let targets: Vec<usize> = if one_step {
            value.written_through(pointer)
        } else if deep > 1 {
            value
                .levels_below(pointer, deep - 1)
                .1
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        Landing {
            containers,
            targets,
            one_step,
            pointer,
            deep,
            kind,
        }
    }
}
