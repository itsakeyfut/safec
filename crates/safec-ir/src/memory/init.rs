//! What a write leaves initialised, and whether a read is of memory nothing
//! wrote: the unwritten belief of ADR-0057, applied by [`Allocations::initialised_by`]
//! and asked by [`Allocations::reads_fresh`] and [`Allocations::reads_partly`].
//!
//! In a file of its own so that one description of where a write lands can
//! be read beside the one belief that applies it. See ADR-0063.

use std::collections::BTreeSet;

use crate::ir::{Function, Operand, Place, Projection};
use crate::reads::derefs;
use crate::source::Span;

use super::allocations::Allocations;
use super::built::replaced_by;
use super::index::{Idx, LocalIdx, Site, Table};
use super::known::{Known, WrittenBy};
use super::parts::{At, Extent, Init};
use super::store::{Certain, Landing, StoreKind, Strong, written_alone};
use super::transfer::CopiedPointers;
use super::{Carried, Carrier};

/// What [`Allocations::initialised_by`] is told of one write.
pub(super) enum Wrote<'a> {
    /// An assignment to `place`, any assignment: one with no dereference
    /// changes nothing here, and the arm asks that itself. `landing` is where
    /// it lands, as the store computed it before writing, the one
    /// description of a write both doors read (#778). `unwritten_before` is
    /// `Known::written` as it was before the store, for a local it may
    /// write past.
    Store {
        place: &'a Place,
        landing: &'a Landing,
        unwritten_before: &'a Table<LocalIdx, bool>,
    },
    /// A library copy handed `dest` first and `source` second, which
    /// exposes `reach`, and whose length covers the pointers it writes as
    /// `length` says.
    Copy {
        dest: Option<&'a Operand>,
        reach: Vec<Site>,
        span: Span,
        length: CopiedPointers,
        bytes: Option<u64>,
        source: Option<&'a Operand>,
    },
}

impl Allocations<'_> {
    /// **A local that is not an array, written through a pointer that reached
    /// it only by the rule that keeps an edge through a known move, is lost,
    /// and in doubt where it was unwritten** (#647): the write may be of the
    /// local, `&slot + 1 - 1`, or past it, `pp[1] = q;`, which is undefined
    /// and the bounds rule's to report. Kept as a may-write alone, a local
    /// that held nothing held only what was stored, so `free(z)` after
    /// `*(&z + 7) = q;` certainly freed `q`'s allocation and `*q` was a proved
    /// use after free; and a read of it after a store past it, which C leaves
    /// indeterminate, was silent where it had been a certain `SC0411`, found
    /// by review. Measured, both. See `Held::moved_by_arithmetic`.
    pub(super) fn written_past_its_end(
        &self,
        function: &Function,
        value: &mut Known,
        target: LocalIdx,
        unwritten_before: &Table<LocalIdx, bool>,
    ) {
        if self.is_array(function, target) {
            return;
        }
        value.points_to[target].lost_its_allocation();
        if unwritten_before[target] {
            value.written[target].doubt();
        }
    }

    /// **What a write does to what this check knows is unwritten**, at the
    /// store and at the library copy: the fresh and may-be-fresh marks of the
    /// allocations it may write, the locals it writes or may write, an array
    /// written in part, a local written past its end, and that a pointer has
    /// written what it points at. One function with one arm per kind of
    /// write, so that a rule changed for one is read beside the other: each
    /// arm is what that door did, and their differences are #726, #729 and
    /// #739 (#737).
    ///
    /// **Called where each door called it**, which is before what the store
    /// pours and after what the copy pours: the copy's exposure of what it
    /// was handed marks what is reachable, which its landing asks before it.
    /// Mutation: call it after the store's `land_contents`, and
    /// `a_live_pointer_through_a_locals_address_stored_in_an_allocation_is_read_in_silence`
    /// fails. Found by review.
    ///
    /// **What is still each door's own**: what a write puts into the locals
    /// it lands in, which a store replaces where it is certain and a copy
    /// only unions (#395), and `Known::named_after_a_store`, which only the
    /// store records. A kind of write added to `StoreKind` is `E0004` at
    /// `Landing`'s two matches; it reaches this function and
    /// `Self::land_contents` only through a variant of `Wrote` and `Pours`
    /// written for it.
    pub(super) fn initialised_by(&self, function: &Function, value: &mut Known, wrote: Wrote<'_>) {
        match wrote {
            Wrote::Store {
                place,
                landing,
                unwritten_before,
            } => {
                let pointer = place.local;
                let one_step = landing.one_step;
                // **A write through a pointer may write an allocation**, of
                // any type, and what it wrote is no longer indeterminate.
                // Placed only for one dereference through a pointer that
                // names exactly its sites; anything else may land in any
                // allocation. See `Known::written`.
                //
                // **A store of an indeterminate pointer too.** The pointer was
                // reported where it was read, and keeping the slot fresh
                // reported the copy the lowering takes of the assignment's
                // value (C17 6.5.16 p3) as a read of memory nothing wrote, at
                // the store that wrote it. Mutation: keep the slot fresh for
                // such a store, and
                // `a_pointer_read_out_of_fresh_memory_stored_and_read_back_is_reported_once`
                // gains two. See ADR-0057.
                if derefs(place) > 0 {
                    let held = &value.points_to[pointer];
                    // **And a local it may land in**, the local its edges name
                    // where the store is placed: `m[0] = p;` writes `m`. Where
                    // it is not placed, `replaced_by` below clears the
                    // locals it may land in. See `Known::written`.
                    if one_step && !held.beyond().unrecorded_address() {
                        let edges: Vec<LocalIdx> = held.locals().collect();
                        let sites: Vec<Site> = held.sites().collect();
                        let site_element = held.site_element;
                        // **A store that may land in several allocations may
                        // have missed any one**, so one that may still be
                        // fresh stays so, as a may-fact: `*p = x;` with `p`
                        // either of two blocks, one of them written on one
                        // arm only, cleared the doubt about it. Found by
                        // review. See `Known::written` (#721). **A local
                        // it names counts as one of them**: `p = c ? m : &loc;
                        // *p = x;` may have written `loc` and missed `m`
                        // (#739).
                        // One site and no edge is the landing's one place.
                        match landing.certain() {
                            // **At its element, where that is known**, so a
                            // block kept per element is written there alone
                            // (#873). Anywhere else the block is written
                            // whole, `b[i] = x;` among them, as a local array
                            // is. See ADR-0081.
                            Some(at) => {
                                let at_element = site_element.is_some_and(|element| {
                                    value.certainly_written_at(&at, element)
                                });
                                // **At an element not known, a may-write of
                                // a block kept per slot**: it may have
                                // missed every slot (#893, ADR-0094 rule 3).
                                let kept = value.written[at.site()].slots().is_some();
                                if site_element.is_none() && kept {
                                    value.written[at.site()].may_write();
                                } else if !at_element {
                                    value.certainly_written(at);
                                }
                            }
                            None => {
                                for &site in &sites {
                                    value.written[site].may_write();
                                }
                            }
                        }
                        // **Through a pointer moved off an array, an element
                        // of it**, which may not be the one read later. See
                        // `built::write_effects`.
                        let writes = super::built::write_effects(
                            self.unit,
                            function,
                            value,
                            place.local,
                            1,
                            edges.iter().copied(),
                            false,
                            self.unit
                                .place_ty(function, place)
                                .and_then(|ty| self.unit.size_of(ty)),
                        );
                        // **A local is written for certain only where it is
                        // the one place the store lands**, as a site is
                        // (`Landing::certain`): with `g` either `&m` or `&n`,
                        // `*g = &x;` wrote one of them and may have missed
                        // the other, and writing both let `r = m;` read
                        // what nothing wrote in silence. A local it may land
                        // in beside another place may still be unwritten,
                        // and each element of it. Found by review.
                        // See ADR-0074.
                        if let Some(at) = written_alone(value, pointer, 1, &edges) {
                            let pending = value.array_writes_before(writes);
                            value.written_local(at);
                            value.array_writes_after(pending);
                        } else {
                            for local in edges {
                                value.written[local].may_write();
                            }
                        }
                        // And what it points at is written now, whichever
                        // element that is. See `Held::stored_through`.
                        if place.projection.as_slice() == [Projection::Deref] {
                            value.points_to[pointer].stored_through = true;
                        }
                    } else {
                        // **Only where it may land**: what its landing names
                        // and what code this check cannot read can reach,
                        // which is where its contents land too
                        // (`Known::landed_where_unknown_code_points`). Every
                        // allocation let `*w = x;` through a pointer read out
                        // of a parameter clear one nothing could reach, read
                        // after it in silence where its `memcpy` spelling is
                        // `SC0411` (#726). Taken as written where it lands in
                        // one place, the may-be-fresh mark not kept, since
                        // keeping it doubted a write through a pointer read out
                        // of memory that certainly reached its block, by the
                        // user's decision. **And
                        // what such code can name through a local whose
                        // address it holds**, `Known::sites_named_by_unknown_code`,
                        // and not only what is reachable from exposed memory:
                        // `keep(&p); p = m; **t = x;` with `t` read out of a
                        // parameter may write `m` through `p`, and reading it
                        // as reachable only refused that correct program with
                        // a certain `SC0411`. Found by review. See ADR-0044 and
                        // ADR-0057.
                        let mut may_land: BTreeSet<Site> =
                            landing.containers.iter().copied().collect();
                        may_land.extend(value.sites_named_by_unknown_code());
                        // **But a store whose landing names more than one place
                        // may have missed any of its blocks**, as one through a
                        // single dereference may: `*h = c ? m : o; **h = x;`
                        // read `*m` in silence (#739), and so did `v = *t; *v =
                        // x;` with `*t` either of two blocks (#754). Whatever
                        // the pointer is: #726's decision is of a store that
                        // certainly reaches its one block, and a landing of two
                        // reaches neither certainly, by the user's decision.
                        //
                        // **Only the landing's one place is written**: what
                        // code this check cannot read may name beside it may
                        // have been written by this store or not, so `**t =
                        // y;` through a parameter that cannot reach `*m` left
                        // `*m` read in silence where it was `SC0411` without
                        // the store (#778).
                        for site in may_land {
                            value.written[site].may_write();
                        }
                        let certain = landing.certain();
                        let mut uncertain = certain.is_none();
                        // **At an element not known, a may-write of a block
                        // kept per slot**, as through a pointer this function
                        // placed: `c = *box; *c = x;` names no element of
                        // the block `c` reaches, and writing it whole left
                        // `b[1]` read in silence (#893, ADR-0094 rule 3).
                        // #726's decision stands for a block kept whole.
                        if let Some(at) = certain {
                            let unplaced = value.points_to[place.local].site_element.is_none()
                                && value.written[at.site()].slots().is_some();
                            if unplaced {
                                value.written[at.site()].may_write();
                                uncertain = true;
                            } else {
                                value.certainly_written(at);
                            }
                        }
                        // And what it points at was written, so that a read
                        // back through it is of what was stored, as the
                        // one-step store's is above: without it the lowering's
                        // own read of `*v` after `*v = x;` was doubted. See
                        // `Held::stored_through`.
                        if uncertain && place.projection.as_slice() == [Projection::Deref] {
                            value.points_to[place.local].stored_through = true;
                        }
                    }
                }
                if !place.projection.is_empty() && landing.strong(value).is_none() {
                    replaced_by(self.unit, function, place, value);
                    if one_step && value.points_to[pointer].array_edge {
                        for &target in &landing.targets {
                            self.written_past_its_end(function, value, target, unwritten_before);
                        }
                    }
                }
            }
            Wrote::Copy {
                dest,
                reach,
                span,
                length,
                bytes,
                source,
            } => {
                // **And a local it may write past the end of**, as a store
                // through the same pointer marks it: lost, and in doubt where
                // it was unwritten. `pp = &z + 1; memcpy(pp, &px, 8); q = z;`
                // reported only the out-of-bounds copy, where `*pp = &x;`
                // also doubts the read of `z` (#729). Through one dereference
                // where the destination itself was moved off a local, and
                // deeper where a level above was, as the store asks it. See
                // `Allocations::written_past_its_end`.
                let unwritten_before = value.unwritten_locals();
                let past_end: Vec<LocalIdx> = match dest {
                    Some(Operand::Copy(dest))
                        if dest
                            .projection
                            .iter()
                            .all(|step| matches!(step, Projection::Deref)) =>
                    {
                        let deep = derefs(dest) + 1;
                        if deep == 1 {
                            if value.points_to[dest.local].array_edge {
                                value.written_through(dest.local)
                            } else {
                                Vec::new()
                            }
                        } else if value
                            .levels_below(dest.local, deep - 2)
                            .1
                            .into_iter()
                            .any(|local| value.points_to[local].array_edge)
                        {
                            value
                                .levels_below(dest.local, deep - 1)
                                .1
                                .into_iter()
                                .collect()
                        } else {
                            Vec::new()
                        }
                    }
                    Some(Operand::Copy(_) | Operand::Constant(_)) | None => Vec::new(),
                };
                // What a pointer it was handed may be, read out of memory, as
                // for an opaque call, exposed. See ADR-0040.
                //
                // **What it writes, only where its destination may land**: the
                // blocks its landing names and, where it may land where this
                // check cannot place it, what code this check cannot read can
                // name, as a store's are since #726. The exposure wrote the fresh
                // mark of everything it reached, the copy's source and what its
                // destination only holds among them: `memcpy(d, m, 8); r = *m;`
                // and `*o = m; memcpy(o, pp, 8); r = *m;` read a block nothing
                // wrote in silence, where the store forms are certain (#739).
                // So the exposure stays, and the mark it had is put back on every
                // block outside that set.
                //
                // **Where the landing names more than one place**, sites and
                // locals together and through a place of dereferences too, it
                // may have missed any of its blocks, so each that may still be
                // fresh stays so, as a store's does (#721, #727), and a
                // destination that is a local has written what it points at,
                // `Held::stored_through`, so that a read back through it is of
                // the block it wrote. See
                // `Known::written`.
                let may_land: Option<(BTreeSet<Site>, bool, bool, Option<Site>)> = match dest {
                    Some(Operand::Copy(dest))
                        if dest
                            .projection
                            .iter()
                            .all(|step| matches!(step, Projection::Deref)) =>
                    {
                        let deep = derefs(dest) + 1;
                        let landing = self.landing(
                            value,
                            dest.local,
                            deep,
                            dest.projection.is_empty(),
                            StoreKind::Copy,
                        );
                        let mut set: BTreeSet<Site> = landing.containers.iter().copied().collect();
                        if landing.unnamed(value)
                            || landing.unplaced()
                            || landing.handed_over(value)
                        {
                            set.extend(value.sites_named_by_unknown_code());
                        }
                        let several = landing.containers.len() + landing.targets.len() > 1;
                        // One block alone, counted after the widening above:
                        // a copy that may land wherever code this check cannot
                        // read can name lands in no one block, and taking its
                        // container alone made every such block certainly fresh,
                        // a written one among them. Found by review.
                        // And from its start, so that a copy that covers the
                        // block's size covers the block.
                        let alone = set.len() == 1
                            && landing.targets.is_empty()
                            && dest.projection.is_empty()
                            && matches!(value.points_to[dest.local].offset, At::Bytes(0));
                        let certain = landing.certain().as_ref().map(Certain::site);
                        Some((set, several, alone, certain))
                    }
                    // A destination that is no place keeps what the exposure
                    // says, as it did.
                    Some(Operand::Copy(_) | Operand::Constant(_)) | None => None,
                };
                // **What it copies may be indeterminate**: bytes nothing wrote
                // in the source are bytes nothing wrote where they land, so a
                // read of the destination is a read of them. Where the source
                // names only blocks nothing wrote, and the copy lands in one
                // block alone with whole pointers, that block is as fresh as
                // `realloc`'s result is (`Known::names_only_fresh`); where it
                // may name one, each block it may land in may be (#724).
                // Asked before the exposure below, which clears the source's
                // own mark. A source read through a dereference is not asked.
                // **And a local it names as much as a block** (#750): a local
                // array nothing wrote, copied into a block, put bytes nothing
                // wrote there, and `memcpy(n, a, 8); r = *n;` read them in
                // silence. Certainly so where it names only locals nothing
                // wrote and nothing it cannot name, as a block's is by
                // `Known::names_only_fresh`; maybe, where any it names may be,
                // at any element, since this keeps one mark per object.
                let (source_fresh, source_maybe) = match source {
                    Some(Operand::Copy(source)) if source.projection.is_empty() => {
                        let held = &value.points_to[source.local];
                        let plain = !(held.beyond().unrecorded_address() || held.inner_address);
                        let locals_unwritten = plain
                            && held.sites().next().is_none()
                            && held.may_be_a_locals_address()
                            && held.locals().all(|l| value.certainly_unwritten(l, None));
                        let locals_maybe = held.locals().any(|l| {
                            value.certainly_unwritten(l, None)
                                || value.may_be_unwritten(l, None, true)
                        });
                        (
                            value.names_only_fresh(source.local) || locals_unwritten,
                            value.may_name_fresh(source.local) || locals_maybe,
                        )
                    }
                    // **A source read through dereferences is asked too**, of
                    // what the pointer it reads may name, the walk a load of it
                    // takes: `memcpy(n, *ppp, 8)` with `**ppp` nothing wrote
                    // read `*n` in silence, where the same through a named
                    // `s = *ppp` was doubted. A doubt alone, since what such a
                    // pointer names is not all it may name. Found by review.
                    Some(Operand::Copy(source))
                        if source
                            .projection
                            .iter()
                            .all(|step| matches!(step, Projection::Deref)) =>
                    {
                        let (sites, locals) = value.levels_below(source.local, derefs(source));
                        let maybe = sites
                            .iter()
                            .any(|&site| value.written[site].whole().may_be_unwritten())
                            || locals.iter().any(|&l| {
                                value.certainly_unwritten(l, None)
                                    || value.may_be_unwritten(l, None, true)
                            });
                        (false, maybe)
                    }
                    Some(Operand::Copy(_) | Operand::Constant(_)) | None => (false, false),
                };
                // Whether the destination may land in no block, nor anywhere
                // code this check cannot read can name: a local is the only
                // place it writes, so it alone takes what the copy carries.
                // **What it carried, for the report about a read of it**
                // (#752): this copy, and the local it read through.
                let copied = Carried {
                    at: span,
                    from: match source {
                        Some(Operand::Copy(source)) if source.projection.is_empty() => {
                            Some(source.local)
                        }
                        Some(Operand::Copy(_) | Operand::Constant(_)) | None => None,
                    },
                    by: Carrier::Copy,
                };
                let lands_in_no_block = may_land
                    .as_ref()
                    .is_none_or(|(blocks, _, _, _)| blocks.is_empty());
                let before = value.written.clone();
                value.expose(self.sources, reach, Some(span));
                if let Some((may_land, several, alone, certain)) = may_land {
                    // **And only as far as its length writes a pointer
                    // whole**: a copy shorter than one writes no pointer in
                    // the block, so a block that was fresh is still fresh,
                    // and a read of it is certain; one that ends part way
                    // through a later pointer wrote some and not others, and
                    // the mark, one per block, cannot say which a read is of,
                    // so it is a doubt, as a copy that may have missed the
                    // block is (#745). See `CopiedPointers`.
                    //
                    // **`several` decides nothing in the `Whole` arm, and
                    // nothing holds it**: a landing of several places has no
                    // `certain` one (`Landing::certain`), so every block it
                    // lands in takes `may_write` below, which leaves one that
                    // may be unwritten `Maybe` already, and `doubt` moves only
                    // `Written`. Measured on #785: answering `false` there
                    // fails no case, no route row and no probe written to tell
                    // them apart. It is kept so that a landing that one day
                    // has a certain place among several still doubts it.
                    let (unwritten, doubted) = match length {
                        CopiedPointers::Whole => (false, several),
                        CopiedPointers::LastInPart => (false, true),
                        CopiedPointers::NoneWhole => (true, false),
                    };
                    let mut carried = false;
                    for site in before.sites() {
                        let was = &before[site];
                        if !may_land.contains(&site) || unwritten {
                            value.written[site] = was.clone();
                        } else {
                            // **Only the landing's one place is written**, as a
                            // store's is (#778): a block code this check cannot
                            // read may name beside it may have been written by
                            // the copy or not, and taking the exposure's belief
                            // for it read `*m` in silence after `memcpy(*t, y,
                            // 8);` through a parameter, where `**t = *y;` is
                            // `SC0411`. A library copy is no code this check
                            // cannot read.
                            if Some(site) != certain {
                                value.written[site] = was.clone();
                                value.written[site].may_write();
                            }
                            if doubted && was.whole().may_be_unwritten() {
                                value.written[site].doubt();
                            }
                            // **Certain only where it covers the whole block**,
                            // its size a constant and the length at least that:
                            // a copy of 8 bytes into 16, or of a length this
                            // check cannot read, leaves what it did not cover
                            // as it was, and that may be written. Found by
                            // review.
                            let covers = matches!(
                                (value.extent[site], bytes),
                                (Some(Extent::Exactly(size)), Some(bytes)) if bytes >= size
                            );
                            if source_fresh
                                && alone
                                && covers
                                && matches!(length, CopiedPointers::Whole)
                            {
                                value.written[site].set(Init::Unwritten);
                                value.written[site].carried_by(copied);
                                carried = true;
                            } else if source_maybe {
                                value.written[site].doubt();
                                // The copy's only where the block was
                                // written before it: one that may already
                                // have been unwritten may be so for its
                                // own reason (#752).
                                if was.whole() == Init::Written {
                                    value.written[site].carried_by(copied);
                                }
                                carried = true;
                            }
                        }
                    }
                    // **Only a destination that is a local itself**: the flag
                    // says what the local points at was written, and a copy
                    // through `*h` writes `**h`, not `*h`, so set on `h` it would
                    // stop doubting a read of `*h` nothing wrote. The store sets
                    // it at one dereference for the same reason.
                    // **Not where it carried bytes nothing may have written**,
                    // and those it carried them into are read as such again:
                    // the flag says a read back through a pointer is of what a
                    // store put there, and what this copy put there may be
                    // indeterminate. `*n = x; memcpy(n, m, 8); r = *n;` with `m`
                    // written on one path read `*n` in silence. Found by review.
                    if carried {
                        for held in &mut value.points_to {
                            if held.sites().any(|site| may_land.contains(&site)) {
                                held.stored_through = false;
                            }
                        }
                    } else if several {
                        if let Some(Operand::Copy(dest)) = dest {
                            if dest.projection.is_empty() {
                                value.points_to[dest.local].stored_through = true;
                            }
                        }
                    }
                }
                // **It writes only through its first argument**: the clause of
                // each function read by name, C17 7.24.2.1 to 7.24.3.2 and
                // 7.24.6.1, says what is written, and it is that one. So
                // a local is written where that argument may point:
                // `Known::locals_a_write_may_land_in`, one dereference below
                // it, which is what a store through it asks too, so the two
                // cannot answer one pointer differently. `memcpy(&a, &b, 4)`
                // cleared every local array's mark, and so did `memcpy(*pa,
                // &b, 4)` (#643), and so did a copy through a pointer read out
                // of the caller's memory (#645). It keeps nothing, so it
                // exposes no local. See `Known::written` and ADR-0057.
                let written: Option<Vec<LocalIdx>> = match dest {
                    Some(Operand::Copy(dest))
                        if dest
                            .projection
                            .iter()
                            .all(|step| matches!(step, Projection::Deref)) =>
                    {
                        Some(value.locals_a_write_may_land_in(dest.local, derefs(dest) + 1))
                    }
                    // A constant writes nothing anyone can read back, and no
                    // other place reaches a call's argument from C.
                    _ => None,
                };
                // And through a destination moved off an array, an element of
                // it, as a store through it answers. See
                // `built::write_effects`.
                let writes = match (dest, &written) {
                    (Some(Operand::Copy(dest)), Some(lands)) => super::built::write_effects(
                        self.unit,
                        function,
                        value,
                        dest.local,
                        derefs(dest) + 1,
                        lands.iter().copied(),
                        true,
                        bytes,
                    ),
                    _ => Vec::new(),
                };
                let pending = value.array_writes_before(writes);
                let mut may_write: Table<LocalIdx, bool> =
                    Table::filled(written.is_none(), value.points_to.len());
                for local in written.iter().flatten() {
                    may_write[*local] = true;
                }
                // **Not where it replaced its one local**, as a store's
                // certain write is not (ADR-0028): the copy wrote that local
                // whole, and `replaced` marking it as a call may have written
                // it took away the belief the copy gave, so `memcpy(&s, &v,
                // 8)` doubted `*s` where `s = v;` did not (#769).
                // It wrote that local, which is what `replaced` would have said
                // of it.
                if let Some(at) = self.copy_replaces_one_local(function, value, dest, bytes) {
                    value.written_local(at.into());
                } else {
                    // **A may-write of every local it may land in**, and a
                    // certain one only where it lands in one local alone, as a
                    // store's is: borrowing the belief for a call this check
                    // cannot read wrote a local the copy may have missed,
                    // `memcpy(g, &px, 8)` with `g` either `&m` or `&n` (#803).
                    // See ADR-0074.
                    // Asked before `replaced`, which marks every escaped local
                    // the copy may write as one code this check cannot read
                    // may have written, so that a pointer the copy goes
                    // through, `p` in `memcpy(*pp, ..)`, would no longer
                    // follow what it names. Found while implementing.
                    let alone = match (dest, &written) {
                        (Some(Operand::Copy(dest)), Some(lands)) => {
                            written_alone(value, dest.local, derefs(dest) + 1, lands)
                                .filter(|_| lands_in_no_block)
                        }
                        _ => None,
                    };
                    value.replaced(|_| true, |local| may_write[local], WrittenBy::StoreOrCopy);
                    if let Some(at) = alone {
                        value.written_local(at);
                    }
                }
                value.array_writes_after(pending);
                // **What it carries into a local it writes**, as into a block
                // above (#724): `memcpy(a, m, 8); r = a[0];` with `m` nothing
                // wrote read `a[0]` in silence, where `a[0] = *m;` is certain
                // (#750). Certain where the source names only what nothing
                // wrote and the copy covers this one local whole from its
                // start; a doubt about the whole local, every element of it,
                // where the source may name such a thing, since a copy of part
                // of it may be of the part that was written, by the user's
                // decision.
                if let (Some(lands), Some(Operand::Copy(dest))) = (&written, dest) {
                    let at_start = dest.projection.is_empty()
                        && matches!(value.points_to[dest.local].offset, At::Bytes(0));
                    for &local in lands {
                        let size = function
                            .locals()
                            .nth(local.index())
                            .and_then(|id| self.unit.size_of(function.local(id)));
                        let covers =
                            matches!((size, bytes), (Some(size), Some(bytes)) if bytes >= size);
                        // One local alone, and no block beside it: `d` either
                        // `a` or a block `k` was certain about `a[0]` though
                        // the copy may have landed in `k`. Found by review.
                        if source_fresh
                            && lands.len() == 1
                            && lands_in_no_block
                            && at_start
                            && covers
                            && matches!(length, CopiedPointers::Whole)
                        {
                            value.written[local].set(Init::Unwritten);
                            value.written[local].carried_by(copied);
                        } else if source_maybe {
                            let was = value.written[local].whole();
                            value.written[local].doubt();
                            if was == Init::Written {
                                value.written[local].carried_by(copied);
                            }
                        }
                    }
                }
                for target in past_end {
                    self.written_past_its_end(function, value, target, &unwritten_before);
                }
            }
        }
    }

    /// Per local, whether it is made unwritten where the function starts:
    /// every local but the parameters, which the caller wrote. See
    /// [`Known::written`].
    ///
    /// **Whatever its type**: a `char` buffer nothing wrote, read as a pointer
    /// or copied out into memory that leaves, is as indeterminate as a pointer
    /// local nothing wrote. What asks it asks only a read as a pointer: a load
    /// (ADR-0069 rule 1) and a library copy into memory that leaves (ADR-0071
    /// rule 5), while a read of an integer by name is unasked
    /// (`report::unwritten_read`). See ADR-0073 rule 2.
    pub(super) fn unwritten_at_entry(&self) -> Vec<bool> {
        let function = self.unit.function(self.function);
        function
            .locals()
            .map(|local| !self.parameters.contains(&local))
            .collect()
    }

    /// Whether reading `source` reads a pointer out of memory nothing wrote,
    /// certainly: one dereference through a pointer that names allocations
    /// and locals and nothing else, every allocation fresh and every local
    /// unwritten. The report asks it at every read of such a place, and
    /// [`Self::read_through`] asks it for [`Held::fresh_read`](super::parts::Held::fresh_read), which is
    /// sound only while the two agree. See ADR-0057.
    ///
    /// **A pointer, by its type.** A byte or an integer read out of fresh
    /// memory is not followed.
    ///
    /// **A local reached by its address is memory too** ([`Known::written`]):
    /// `int *q = m[0];` with `m` a local array nothing wrote reads an
    /// indeterminate pointer, as `*box` does out of `malloc`'s block (#640).
    /// `writes_elsewhere` is not asked. `Address(a[0])` sets it for every
    /// array, and a path where the pointer held nothing is a null, which is
    /// the nullability check's as a path holding no site already is for an
    /// allocation. `inner_address` is asked, since an edge that may name
    /// something inside another local names the wrong memory. **Nothing
    /// guards that clause**: no C program reaches a read through such an
    /// edge with every other clause holding.
    ///
    /// Mutation: weaken `all` to `any` over the sites, and
    /// `a_pointer_read_through_either_of_two_allocations_one_written_is_doubted`
    /// is reported as certain. **`loaded`, among the marks
    /// `Beyond::unrecorded_address` asks, is held** by
    /// `a_pointer_no_call_can_reach_is_not_written_by_a_store_this_check_cannot_place`,
    /// which gains a false certain report without it, and by
    /// `a_pointer_read_out_of_memory_that_names_a_block_nothing_wrote_is_doubted_where_read_through`,
    /// whose doubt becomes certain: such a pointer is a doubt, in
    /// `reads_partly` (#728). **`lost` and `foreign` there have no case**,
    /// measured: a local loses its sites where it is made lost, and no
    /// program found gives a foreign pointer a fresh site that an exposure
    /// has not already cleared.
    pub(super) fn reads_fresh(&self, function: &Function, source: &Place, value: &Known) -> bool {
        let through = &value.points_to[source.local];
        source.projection.as_slice() == [Projection::Deref]
            && self.may_be_pointer(function, source)
            && (through.sites().next().is_some() || through.may_be_a_locals_address())
            && !(through.beyond().unrecorded_address() || through.inner_address)
            && through.sites().all(|site| {
                value
                    .site_init_at(site, through.site_element)
                    .is_unwritten()
            })
            && through
                .locals()
                .all(|local| value.certainly_unwritten(local, through.element))
    }

    /// Whether a load may read an array that may still be unwritten where it
    /// reads, [`Known::written`]: `reads_fresh`'s question, answered as
    /// a doubt. A fresh
    /// allocation beside it is not known written, and is no reason for
    /// silence. Found by review of #667. **Or an allocation that may still be
    /// fresh**, `Known::written` unwritten or maybe so, which needs no local
    /// beside it (#721), and whatever allocations beside it were written
    /// (#724). **Any place it may read that may be unwritten is enough**, a
    /// block or a local, whatever else it may read was written: the read is of
    /// that place on some path, and undefined there whatever it is read for
    /// (#749). See ADR-0057.
    ///
    /// **At the element the pointer is known to point at**, or, where that is
    /// not known, at any element (`Known::may_be_unwritten`). Not through a
    /// pointer a store has written
    /// through since it was given its value: that read is of the element
    /// just written, the lowering's own read of an assignment's value among
    /// them. See `Held::stored_through` and ADR-0057.
    pub(super) fn reads_partly(&self, function: &Function, source: &Place, value: &Known) -> bool {
        // **A read below the first dereference is asked too**, of what the
        // pointer it goes through last may name, as a load reads it
        // (`Known::levels_below`): `int *r = **m;` of memory nothing wrote
        // was asked nothing, where `t = *m; r = *t;` was `SC0411` (#768).
        // **A doubt, never a proof**: that pointer was read out of memory,
        // so what the walk shows is a lower bound on what it may name, and
        // it is asked as a pointer read out of memory is below, a block or
        // a local that is not an array, at no known element, and at a local's
        // start, which is where an edge to a local points: asked as off its
        // start, a local written on one path only was not doubted, where its
        // two-step spelling is. See ADR-0057.
        // `derefs` is zero for a place with anything but dereferences in it.
        let depth = derefs(source);
        if depth > 1 {
            if !self.may_be_pointer(function, source) {
                return false;
            }
            let (sites, locals) = value.levels_below(source.local, depth - 1);
            return sites
                .into_iter()
                .any(|site| value.written[site].whole().may_be_unwritten())
                || locals.into_iter().any(|local| {
                    !self.is_array(function, local)
                        && (value.certainly_unwritten(local, None)
                            || value.may_be_unwritten(local, None, true))
                });
        }
        let through = &value.points_to[source.local];
        let at_start = matches!(through.offset, At::Bytes(0));
        let maybe = |local: LocalIdx| value.may_be_unwritten(local, through.element, at_start);
        // **Any block it may read that may still be fresh is enough**,
        // fresh or maybe so, beside blocks that were written: `p = a; if (c)
        // p = b;` with `b` written reads `a` on one path, which is undefined
        // there whatever the value is read for (#724). `reads_fresh` answers
        // first, and asks every block.
        let site_maybe = |site: Site| {
            value
                .site_init_at(site, through.site_element)
                .may_be_unwritten()
        };
        source.projection.as_slice() == [Projection::Deref]
            && self.may_be_pointer(function, source)
            && (through.may_be_a_locals_address() || through.sites().next().is_some())
            && !(through.beyond().stopped_following_or_written_unseen()
                || through.inner_address || through.stored_through)
            // **A pointer read out of memory is asked about its blocks
            // alone**: it may name what it does not show, so what it shows is
            // no proof (`reads_fresh` refuses it), and a block it names that
            // may still be fresh is a doubt. `u = *t; r = *u;` with `*t = m`
            // read `m` nothing wrote in silence (#728). **And about every
            // local it names**, unwritten or maybe so: `u = *pp; r = *u;` with
            // `*pp = &loc` read `loc` in silence (#756). **An array too**, by
            // the user's decision on #919, reversing the one on #728 that
            // left arrays out: that moved twelve cases then and one now, the
            // case that held the silence, and it left a `char` array written
            // at one byte silent once its address passed through memory,
            // where ADR-0073 rule 2 asks it. Mutation: leave such an array
            // out again; `a_pointer_read_out_of_memory_that_names_an_unwritten_local_array_is_doubted`
            // goes silent, and so do the route matrix's `part_local_*` rows
            // through memory.
            && (through.sites().any(site_maybe)
                || through.locals().any(|local| {
                    value.certainly_unwritten(local, through.element) || maybe(local)
                }))
    }

    /// The one local a library copy into `dest` of `bytes` replaces, where
    /// it replaces one: one exact local and nothing else
    /// ([`Landing::strong`]), and a constant length of exactly its
    /// size, as a store's certain write replaces its target (ADR-0028). Not
    /// a longer one, which writes past the local and is the bounds rule's to
    /// report.
    /// Anything less is a union, since a copy may write part of a pointer,
    /// or none (#395). **One answer for the value the copy leaves and for
    /// whether it is marked as a call's write**, so that the two cannot
    /// disagree: a copy that replaced `s` and then marked it as one a call
    /// may have written doubted `*s` where `s = v;` did not (#769).
    pub(super) fn copy_replaces_one_local(
        &self,
        function: &Function,
        value: &Known,
        dest: Option<&Operand>,
        bytes: Option<u64>,
    ) -> Option<Strong> {
        let Some(Operand::Copy(dest)) = dest else {
            return None;
        };
        if !dest
            .projection
            .iter()
            .all(|step| matches!(step, Projection::Deref))
        {
            return None;
        }
        let landing = self.landing(
            value,
            dest.local,
            derefs(dest) + 1,
            dest.projection.is_empty(),
            StoreKind::Copy,
        );
        let size = |target: LocalIdx| {
            function
                .locals()
                .nth(target.index())
                .and_then(|local| self.unit.size_of(function.local(local)))
        };
        landing
            .strong(value)
            .filter(|at| bytes.is_some() && bytes == size(at.local()))
    }
}
