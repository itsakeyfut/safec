//! Which locals have their own address taken, one answer at crate level.
//!
//! It was private to the memory check, while the nullability check answers
//! the same question with a walk of its own that does not skip an address
//! through a dereference. One answer here, so that a later analysis asks
//! this one rather than writing a third. See #895.

use crate::bits::Bits;
use crate::index::LocalIdx;
use crate::ir::{Element, Function, Operation, Projection, Rvalue};

/// Every local whose own address is taken anywhere in `function`: what
/// `memory::assigned` does not watch, since a write through that address is not
/// followed by `memory::assigned`, and what the memory check's `Known::written` asks by
/// name instead, since it does follow one. One answer for both, so that a
/// local whose address is taken is asked by that check alone (#640); one
/// whose address is never taken is asked there first, and by that check
/// only for its doubt (ADR-0075).
pub(crate) fn address_taken(function: &Function) -> Bits<LocalIdx> {
    let mut taken = Bits::new(function.locals().len());
    for block in function.blocks() {
        for element in &block.elements {
            if let Element::Assign(Operation {
                value: Rvalue::Address(place),
                ..
            }) = element
            {
                // **The whole local's address, or an element's.** `&a[0]` is
                // the address an array converts to (#621), and its own start.
                // `&*p` is `p`'s value and not `p`'s address.
                if !place
                    .projection
                    .iter()
                    .any(|step| matches!(step, Projection::Deref))
                {
                    taken.set(LocalIdx::of(place.local));
                }
            }
        }
    }
    taken
}
