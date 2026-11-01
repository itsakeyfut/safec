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
use crate::source::Span;

use super::derefs;
use super::known::Known;
use super::parts::Callee;
use super::transfer::Allocations;

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
    /// direct write, whether such code names the local; otherwise
    /// [`Known::leaves`] of [`Self::into`].
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
    /// with none.
    Through {
        /// How many dereferences the destination goes through.
        derefs: usize,
    },
    /// A library copy, `memcpy` or `memmove`, of bytes C defines to copy
    /// (C17 7.24.2.1, 6.2.6.1 p4), and so asked as a doubt and never as a
    /// proof (#652).
    Copy,
}

impl Allocations<'_> {
    /// The store an element is, if it is one: an assignment.
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
    /// arguments. A body defined here is what runs, whatever it is called,
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
