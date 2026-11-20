//! What a write leaves initialised, and whether a read is of memory nothing
//! wrote: the unwritten belief of ADR-0057, applied by [`Allocations::initialised_by`]
//! and asked by [`Allocations::reads_fresh`] and [`Allocations::reads_partly`].
//!
//! In a file of its own so that one description of where a write lands can
//! be read beside the one belief that applies it. See ADR-0063.

use crate::ir::{Rank, Sign};
use std::collections::BTreeSet;

use crate::ir::{Function, Operand, Place, Shape, Ty};
use crate::source::Span;

use super::allocations::{Allocations, innermost_element};
use super::built::write_effects;
use super::index::{LocalIdx, Site, Storage, Table};
use super::known::{Known, WrittenBy};
use super::parts::{At, Held, Init, PlaceInit, Whole};
use super::store::{Landing, StoreKind, Strong, written_alone};
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
        /// How many bytes it writes, which `Write` says once for the store
        /// rather than this rule asking the place's type again.
        width: Option<u64>,
        /// What it wrote: a whole pointer only where its type is a pointer
        /// (#940).
        whole: Whole,
    },
    /// A library copy handed `dest` first and `source` second, whose length
    /// covers the pointers it writes as
    /// `length` says. `byte_for_byte` where it copies `bytes` of the source
    /// as they are, `memcpy` and `memmove`, and not a string copy, which
    /// stops at a null and may pad with zeros (C17 7.24.2.3 to 7.24.3.1).
    Copy {
        dest: Option<&'a Operand>,
        span: Span,
        length: CopiedPointers,
        bytes: Option<u64>,
        source: Option<&'a Operand>,
        byte_for_byte: bool,
        /// What it is believed to write ([`Allocations::copy_writes`]).
        whole: Whole,
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

    /// **Which blocks a write through `place` marks written**, a store's and
    /// a library copy's alike (#1005): the fresh and may-be-fresh marks of
    /// the allocations it lands in, as its landing says. One method for
    /// both doors, so that a block the two would mark differently cannot be
    /// written twice. The copy marked them once by exposing everything it
    /// was handed, as a call this check cannot read is, and putting back
    /// every block outside a set it computed itself; that set answered
    /// apart from the store's, which was #993 and #994. See ADR-0063.
    ///
    /// Returns whether no one block certainly took a write its landing does
    /// not place, which is where the store marks what it points at as
    /// written (`Held::stored_through`). What a write does to the locals it
    /// lands in is still each door's own.
    ///
    /// **What a write lands in certainly is each kind's**, a `match` on
    /// [`StoreKind`] with no wildcard: a store by the element it lands at,
    /// a copy by how much of the block its length covers
    /// ([`Self::copied_into`]).
    fn blocks_written(
        &self,
        value: &mut Known,
        place: &Place,
        landing: &Landing,
        width: Option<u64>,
        whole: Whole,
    ) -> bool {
        let pointer = place.local;
        let one_step = landing.one_step;
        let offset = value.points_to[pointer].position.at;
        let covers_block =
            |value: &Known, site: Site| value.covers_whole(Storage::Heap(site), offset, width);
        let held = &value.points_to[pointer];
        if one_step && !held.beyond().any_unknown() {
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
                    match landing.kind {
                        StoreKind::Direct | StoreKind::Through { .. } => {
                            let at_element = site_element.is_some_and(|element| {
                                value.certainly_written_at(&at, element, whole)
                            });
                            // **At an element not known, a may-write of
                            // a block kept per slot**: it may have
                            // missed every slot (#893, ADR-0094 rule 3).
                            // **And of a block kept whole, by a store
                            // narrower than it**, which may have missed
                            // the rest (`covers_block`).
                            let kept = value.written[at.site()].slots().is_some();
                            let may_have_missed = if kept {
                                site_element.is_none()
                            } else {
                                !covers_block(value, at.site())
                            };
                            if may_have_missed {
                                value.written[at.site()].may_write(whole);
                            } else if !at_element {
                                value.certainly_written(at, whole);
                            }
                        }
                        StoreKind::Copy => self.copied_into(value, at.site(), offset, width, whole),
                    }
                }
                None => {
                    for &site in &sites {
                        value.written[site].may_write(whole);
                    }
                }
            }
            false
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
            let mut may_land: BTreeSet<Site> = landing.containers.iter().copied().collect();
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
            //
            // **What a store wrote is said only of the blocks its landing
            // names** (#940): what code this check cannot read can name
            // beside them keeps what it held. A store of a value that is
            // not a pointer landing in memory of pointers is undefined
            // itself (C17 6.5p7), so a defined program never makes it hold
            // one, and saying it did doubted every pointer read out of
            // such memory after an `int` stored through a parameter, by the
            // owner's decision. **Not a library copy's or a fill's**, whose
            // bytes C17 6.5p6 lets set what allocated memory holds, so the
            // later read is what is undefined. **Held by no case,
            // measured**: answering as a store does fails nothing, and no
            // program built for it by review reached it without other
            // reports about the pointer it went through.
            // See ADR-0098.
            for site in may_land {
                let copy = matches!(landing.kind, StoreKind::Copy);
                let wrote = if copy || landing.containers.contains(&site) {
                    whole
                } else {
                    Whole::Pointer
                };
                value.written[site].may_write(wrote);
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
                match landing.kind {
                    StoreKind::Direct | StoreKind::Through { .. } => {
                        // **`place.local` describes the block only one step
                        // below it** (#1009). Deeper, what reaches the block
                        // was read out of memory, which keeps no position
                        // (#997), so its element is not known and `**box =
                        // *x;` wrote all of `b` by `box`'s own element. A
                        // block kept whole is asked from its start, as a load
                        // of that pointer believes it: a write as wide as the
                        // block that starts anywhere else runs past its end,
                        // which is the bounds rule's to report, so the start
                        // makes certain no write that is not whole.
                        let kept = value.written[at.site()].slots().is_some();
                        let element_known =
                            one_step && value.points_to[place.local].site_element.is_some();
                        let reached_at = if one_step { offset } else { At::Bytes(0) };
                        let unplaced = (!element_known && kept)
                            || (!kept
                                && !value.covers_whole(
                                    Storage::Heap(at.site()),
                                    reached_at,
                                    width,
                                ));
                        if unplaced {
                            value.written[at.site()].may_write(whole);
                            uncertain = true;
                        } else {
                            value.certainly_written(at, whole);
                        }
                    }
                    // Its position counted only where `*dest` is one step
                    // below a local, whose position it is.
                    StoreKind::Copy => {
                        let at_dest = if one_step { offset } else { At::Unknown };
                        self.copied_into(value, at.site(), at_dest, width, whole);
                    }
                }
            }
            uncertain
        }
    }

    /// **What a library copy writes of the one block it certainly lands
    /// in** (#925): whole where it covers the block, a block's size a
    /// constant and the length at least that; the slots it covers whole
    /// where the block is kept per slot, from a position counted and
    /// aligned to a pointer with a length known, the rest keeping what they
    /// held; and a may-write anywhere else.
    ///
    /// **Not the store's element rule**, which takes one write for one
    /// slot: a store is never wider than a pointer in what this frontend
    /// lowers, and a copy is any length. Taken for a copy, a length this
    /// check cannot read, or a string copy that may stop after one byte,
    /// certainly wrote the slot it started at, and a read of it after
    /// `memcpy(b, &x, n)` was silent where it is a doubt (#1005's review);
    /// and `memcpy(m2, v2, 16)` into a block of two slots wrote only the
    /// first, reporting a correct program (the route row
    /// `fresh_memcpy_part_d1_both`).
    fn copied_into(&self, value: &mut Known, site: Site, at: At, width: Option<u64>, whole: Whole) {
        let target = self.unit.target();
        let pointer =
            u64::from(target.pointer_bits() / target.integer(Rank::Char, Sign::Plain).bits());
        if value.covers_whole(Storage::Heap(site), at, width) {
            value.written[site].set(Init::Written(whole));
            return;
        }
        let covered = match (at, width) {
            (At::Bytes(start), Some(bytes)) if start % pointer == 0 => {
                Some((start / pointer, (start + bytes) / pointer))
            }
            _ => None,
        };
        let place = &mut value.written[site];
        match (place.slots_mut(), covered) {
            (Some(slots), Some((first, end))) => {
                for slot in first..end {
                    if let Ok(slot) = usize::try_from(slot) {
                        if slot < slots.len() {
                            slots.set(slot, Init::Written(whole));
                        }
                    }
                }
            }
            (Some(_) | None, _) => place.may_write(whole),
        }
    }

    /// Distrust every escaped local a write through this place may have reached.
    ///
    /// ADR-0029's fact with the writer inside the function: whoever holds an
    /// escaped address may have been handed it through a projection this check
    /// does not follow, so a write it cannot pin down may replace what an escaped
    /// local holds, and writing `SiteState::Freed` on that local's sites later is
    /// a proof about an allocation this write may have swapped out. See ADR-0031.
    ///
    /// **Only a local declared with the type this write writes.** C17 6.5 p7 gives
    /// an object an effective type and lets an lvalue of another type access it
    /// only where that type is a character type, so a write of an `int` cannot
    /// replace a pointer and the rule costs what it should rather than every
    /// escaped local at every `*p = 1`. `docs/c-family.md` carries what that asks
    /// of a frontend, and getting it wrong keeps a proof rather than losing one,
    /// which is a false positive and not a silence.
    ///
    /// **A character type reaches everything, which is the exception that clause
    /// carries**, and no cast is needed to reach it: C17 6.3.2.3 p1 and 6.5.16.1
    /// p1 make the `void *` round trip implicit both ways, so `void *v = &p;
    /// char *c = v;` is a conforming program this frontend accepts, and copying
    /// one pointer's object representation through `c` is defined. Review found
    /// that one, and compiled and ran the C under a sanitiser to show the program
    /// has no use after free in it.
    ///
    /// [`TranslationUnit::place_ty`](crate::ir::TranslationUnit::place_ty) answers `None` for a `Deref` of something
    /// that is not a pointer, which the lowering does not build and a hand-built
    /// unit can. Every local then, because a write whose type this cannot name is
    /// a write it cannot narrow.
    ///
    /// **What it may have written is a second, narrower answer**, about
    /// `Known::written` alone: a local of that type which
    /// `Known::locals_a_write_may_land_in` says the place may land in. A local
    /// the write cannot reach keeps its mark, so a read of it nothing wrote is
    /// still `SC0411`. See ADR-0057.
    ///
    /// **A store and a library copy alike** (#1006), the other half of what
    /// one write marks written beside [`Self::blocks_written`] (ADR-0063). The
    /// copy spelled this again in its own arm; where the two differ is a
    /// `match` on `kind` with no wildcard. `copied` is a copy's constant
    /// length, not read for a store, whose width is its type's.
    pub(super) fn locals_written(
        &self,
        function: &Function,
        value: &mut Known,
        place: &Place,
        kind: StoreKind,
        copied: Option<u64>,
        whole: Whole,
    ) {
        let unit = self.unit;
        let written_ty = unit.place_ty(function, place);
        // **A copy is bytes**, as a write of a character type is, so every
        // escaped local may hold what it wrote.
        let everything = match kind {
            StoreKind::Direct | StoreKind::Through { .. } => {
                match written_ty.map(|ty| unit.ty(ty)) {
                    None
                    | Some(Ty::Integer {
                        rank: Rank::Char,
                        sign: _,
                    }) => true,
                    Some(Ty::Integer {
                        rank: Rank::Int,
                        sign: _,
                    }) => false,
                    Some(Ty::Integer {
                        rank: rank @ (Rank::Short | Rank::Long | Rank::LongLong),
                        sign: _,
                    }) => unreachable!("{rank:?} is #1112's to answer, and nothing builds it yet"),
                    Some(Ty::Void | Ty::Pointer(_) | Ty::Array { .. }) => false,
                }
            }
            StoreKind::Copy => true,
        };
        // A row of bytes per local, against a value that is already square in
        // them, because the predicate is asked per index and `LocalId` cannot be
        // built from one.
        //
        // **An array holds its elements**, so `*p = &x;` with `p` an `int **`
        // may land in an `int *m[1]`: compared against the local's own type, the
        // array was believed unwritten after it, and a read of `m[0]` was reported
        // as indeterminate (#640).
        let may_hold: Table<LocalIdx, bool> = Table::from_vec(
            function
                .locals()
                .map(|local| {
                    everything || written_ty == Some(innermost_element(unit, function.local(local)))
                })
                .collect(),
        );

        // Written: a local of the type stored that the place may land in.
        let mut lands: Table<LocalIdx, bool> = Table::filled(false, may_hold.len());
        for local in value.locals_a_write_may_land_in(
            place.local,
            match place.shape() {
                Shape::Deref(depth) => depth,
                Shape::Local | Shape::Indexed => 0,
            },
        ) {
            lands[local] = true;
        }
        // **Through a pointer moved off an array, an element of it**, as the
        // placed store answers: still a write `replaced` makes of every local it
        // may land in, then partly written where it was unwritten before. See
        // `Known::written`.
        let writes = write_effects(
            unit,
            function,
            value,
            place.local,
            match place.shape() {
                Shape::Deref(depth) => depth,
                Shape::Local | Shape::Indexed => 0,
            },
            lands
                .indices()
                .filter(|&local| lands[local] && may_hold[local]),
            matches!(kind, StoreKind::Copy),
            match kind {
                StoreKind::Direct | StoreKind::Through { .. } => {
                    written_ty.and_then(|ty| unit.size_of(ty))
                }
                StoreKind::Copy => copied,
            },
        );
        // **A store this check reads that may land in more than one place may
        // have missed each local it may land in**, which may still be unwritten,
        // as a site the landing names beside another may (#721): with `g`
        // either `&m` or `&n`, or either `m + 1` or an allocation, `*g = &x;`
        // let a read of `m` or `m[1]` nothing wrote go silent. Found by review.
        // **It writes a local for certain only where it lands there alone**,
        // through a pointer that follows what it names (`store::written_alone`),
        // and is otherwise a may-write, `WrittenBy::StoreOrCopy`: it is no call, and
        // borrowing a call's belief that it writes what it reaches left a
        // pointer read out of a caller's memory, naming one local and landing
        // elsewhere, taken as certain (#803). A pointer read out of memory this
        // function recorded is believed to reach its one local, as #726 believes
        // a block's. See ADR-0074.
        let landing: Vec<LocalIdx> = lands
            .indices()
            .filter(|&local| lands[local] && may_hold[local])
            .collect();
        // **A copy writes its one local for certain only where it covers the
        // local exactly**: a length this check cannot read
        // may be zero, and a shorter one leaves part of the local as it was, so
        // `memcpy(&p, &src, n); return p;` returned a pointer nothing may have
        // written in silence (#942). A longer one is the bounds rule's to report.
        // See ADR-0095. That it lands in no block beside the local is
        // `written_alone`'s own condition, for either kind. The copy asked
        // it again in its arm, and that second asking refused more: whether
        // code this check cannot read can name any block at all, which one
        // pointer parameter makes so, doubting `a` after `memcpy(*t, &s, 8)`
        // with `*t` only `&a`, where the store `**t = s;` does not (#1006's
        // review).
        let alone = written_alone(
            value,
            place.local,
            match place.shape() {
                Shape::Deref(depth) => depth,
                Shape::Local | Shape::Indexed => 0,
            },
            &landing,
        )
        .filter(|at| match kind {
            StoreKind::Direct | StoreKind::Through { .. } => true,
            StoreKind::Copy => self.covers_exactly(function, at.local(), copied),
        });
        // **Put back as a may-write only for a store** (#721, the comment
        // above): a copy's `write_effects` already answers which elements of an
        // array it covers, and putting each local back as a may-write wrote
        // over that with a may-write of the whole array. Measured on #1006: put
        // back for a copy too, three proofs became doubts and two correct
        // programs were reported, among them
        // `an_array_of_two_pointers_a_copy_of_one_wrote_read_at_the_first_is_not_reported`.
        let put_back = match kind {
            StoreKind::Direct | StoreKind::Through { .. } => alone.is_none(),
            StoreKind::Copy => false,
        };
        let before: Vec<(LocalIdx, PlaceInit)> = if !put_back {
            Vec::new()
        } else {
            landing
                .iter()
                .map(|&local| (local, value.written[local].clone()))
                .collect()
        };
        let pending = value.array_writes_before(writes);
        value.replaced(
            |local| may_hold[local],
            |local| may_hold[local] && lands[local],
            WrittenBy::StoreOrCopy(whole),
        );
        if let Some(at) = alone {
            value.written_local(at, whole);
        }
        value.array_writes_after(pending, whole);
        for (local, mut written) in before {
            written.may_write(whole);
            value.written[local] = written;
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
                width,
                whole,
            } => {
                let pointer = place.local;
                let one_step = landing.one_step;
                // **And a local it may write past the end of**, at every depth:
                // `**ppp = q;` with `*ppp = &z + 7` proved a use after free
                // once the one-step store was marked and the deeper one was
                // not. Asked before the write changes what the pointer reaches.
                // See `Allocations::past_end_targets`.
                let past_end = self.past_end_targets(value, pointer, landing.deep, one_step);
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
                // **A store covers a block kept whole only where it is as
                // wide as the block**, an allocation of a constant size: any
                // narrower store leaves bytes of it as they were, and the
                // block's one record cannot say which. Written whole, `b =
                // malloc(800); b[0] = x; r = b[5];` read `b[5]` in silence
                // once a counted position proved it inside the block (#703),
                // where an unknown position had doubted it. **Nor a block of
                // no constant size**, which no store is known to be as wide
                // as: written whole, `b = malloc(n); *b = x; return b;` was
                // believed written, and only the doubt whether a read past
                // its start fits stood in for the slots nothing wrote, the
                // shape #703 exposed (#932). See ADR-0094 rule 3.
                if (match place.shape() {
                    Shape::Deref(depth) => depth,
                    Shape::Local | Shape::Indexed => 0,
                }) > 0
                {
                    let held = &value.points_to[pointer];
                    let placed = one_step && !held.beyond().any_unknown();
                    // The locals it may land in, taken before the blocks are
                    // marked, as they were.
                    let edges: Vec<LocalIdx> = held.locals().collect();
                    let uncertain = self.blocks_written(value, place, landing, width, whole);
                    if placed {
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
                            width,
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
                            value.written_local(at, whole);
                            value.array_writes_after(pending, whole);
                        } else {
                            for local in edges {
                                value.written[local].may_write(whole);
                            }
                        }
                        // And what it points at is written now, whichever
                        // element that is. See `Held::stored_through`.
                        if match place.shape() {
                            Shape::Deref(1) => true,
                            Shape::Local | Shape::Deref(_) | Shape::Indexed => false,
                        } {
                            value.points_to[pointer].stored_through = true;
                        }
                    } else {
                        // And what it points at was written, so that a read
                        // back through it is of what was stored, as the
                        // one-step store's is above: without it the lowering's
                        // own read of `*v` after `*v = x;` was doubted. See
                        // `Held::stored_through`.
                        if uncertain
                            && (match place.shape() {
                                Shape::Deref(1) => true,
                                Shape::Local | Shape::Deref(_) | Shape::Indexed => false,
                            })
                        {
                            value.points_to[place.local].stored_through = true;
                        }
                    }
                }
                if !(match place.shape() {
                    Shape::Local => true,
                    Shape::Deref(_) | Shape::Indexed => false,
                }) && landing.strong(value).is_none()
                {
                    self.locals_written(function, value, place, landing.kind, None, whole);
                    for target in past_end {
                        self.written_past_its_end(function, value, target, unwritten_before);
                    }
                }
            }
            Wrote::Copy {
                dest,
                span,
                length,
                bytes,
                source,
                byte_for_byte,
                whole,
            } => {
                // **And a local it may write past the end of**, as a store
                // through the same pointer marks it: lost, and in doubt where
                // it was unwritten. `pp = &z + 1; memcpy(pp, &px, 8); q = z;`
                // reported only the out-of-bounds copy, where `*pp = &x;`
                // also doubts the read of `z` (#729). Which locals is the
                // store's own answer, `Allocations::past_end_targets`.
                let unwritten_before = value.unwritten_locals();
                // Asked only of a destination reached by dereferences alone,
                // which is the one shape `may_land` below takes a copy's
                // destination to have.
                let past_end: Vec<LocalIdx> = match dest {
                    Some(Operand::Copy(dest))
                        if match dest.shape() {
                            Shape::Local | Shape::Deref(_) => true,
                            Shape::Indexed => false,
                        } =>
                    {
                        self.past_end_targets(
                            value,
                            dest.local,
                            (match dest.shape() {
                                Shape::Deref(depth) => depth,
                                Shape::Local | Shape::Indexed => 0,
                            }) + 1,
                            match dest.shape() {
                                Shape::Local => true,
                                Shape::Deref(_) | Shape::Indexed => false,
                            },
                        )
                    }
                    Some(Operand::Copy(_) | Operand::Constant(_)) | None => Vec::new(),
                };
                // **What it writes, only where its destination may land**: the
                // blocks its landing names and, where it may land where this
                // check cannot place it, what code this check cannot read can
                // name, as a store's are since #726. It exposes nothing it was
                // handed: what it carries is exposed where it lands, by
                // `land_contents`, as a store's is (#1014, ADR-0096).
                //
                // **Where the landing names more than one place**, sites and
                // locals together and through a place of dereferences too, it
                // may have missed any of its blocks, so each that may still be
                // fresh stays so, as a store's does (#721, #727), and a
                // destination that is a local has written what it points at,
                // `Held::stored_through`, so that a read back through it is of
                // the block it wrote. See
                // `Known::written`.
                let may_land: Option<(BTreeSet<Site>, bool, bool, Landing, Place)> = match dest {
                    Some(Operand::Copy(dest))
                        if match dest.shape() {
                            Shape::Local | Shape::Deref(_) => true,
                            Shape::Indexed => false,
                        } =>
                    {
                        let deep = (match dest.shape() {
                            Shape::Deref(depth) => depth,
                            Shape::Local | Shape::Indexed => 0,
                        }) + 1;
                        let landing = self.landing(
                            value,
                            dest.local,
                            deep,
                            match dest.shape() {
                                Shape::Local => true,
                                Shape::Deref(_) | Shape::Indexed => false,
                            },
                            StoreKind::Copy,
                        );
                        let mut set: BTreeSet<Site> = landing.containers.iter().copied().collect();
                        // **Not because a call was handed the block it lands
                        // in**: the copy writes where its destination lands,
                        // as a store does, and the store never widened by it.
                        // Widened, `keep(b); memcpy(b, &yy, 8); r = *m2;`
                        // doubted the read of `*m2` the store spelling `*b =
                        // yy;` proves, since `m2` stored in `*b` is a block
                        // that call can name (#993); and a copy out through a
                        // parameter doubted what it let leave (#994). What
                        // it carries that nothing wrote into a place another
                        // parameter may also name is reported at the copy
                        // itself (`Kind::CopiedUnwritten`), and that report,
                        // not a doubt at a later read, is what holds such a
                        // program (`a_copy_of_unwritten_bytes_through_a_parameter_that_may_alias_another_is_reported_at_the_copy`).
                        if landing.unnamed(value) || landing.unplaced() {
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
                            && (match dest.shape() {
                                Shape::Local => true,
                                Shape::Deref(_) | Shape::Indexed => false,
                            })
                            && matches!(value.points_to[dest.local].position.at, At::Bytes(0));
                        // The place it writes, `*dest`, as a store through
                        // the same pointer writes it.
                        let mut written = (*dest).clone();
                        written = written.deref();
                        Some((set, several, alone, landing, written))
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
                // **Of a block kept per slot, the slots it reads** (#928): asked
                // of the whole, `b[0] = x; memcpy(&r, b, 8);` doubted `r`
                // where `r = b[0];` reads the slot written.
                let (source_fresh, source_maybe) = match source {
                    Some(Operand::Copy(source))
                        if (match source.shape() {
                            Shape::Local => true,
                            Shape::Deref(_) | Shape::Indexed => false,
                        }) =>
                    {
                        let held = &value.points_to[source.local];
                        let slots = self.slots_read(held, bytes);
                        let plain = !(held.beyond().any_unknown() || held.inner_address);
                        let local_slots = self.slots_read_of_locals(held, bytes);
                        let locals_unwritten = plain
                            && held.sites().next().is_none()
                            && held.may_be_a_locals_address()
                            && held
                                .locals()
                                .all(|l| value.written[l].over(local_slots).is_unwritten());
                        let locals_maybe = held
                            .locals()
                            .any(|l| value.written[l].over(local_slots).may_be_unwritten());
                        (
                            value.names_only_fresh(source.local, slots) || locals_unwritten,
                            value.may_name_fresh(source.local, slots) || locals_maybe,
                        )
                    }
                    // **A source read through dereferences is asked too**, of
                    // what the pointer it reads may name, the walk a load of it
                    // takes: `memcpy(n, *ppp, 8)` with `**ppp` nothing wrote
                    // read `*n` in silence, where the same through a named
                    // `s = *ppp` was doubted. A doubt alone, since what such a
                    // pointer names is not all it may name. Found by review.
                    Some(Operand::Copy(source))
                        if match source.shape() {
                            Shape::Local | Shape::Deref(_) => true,
                            Shape::Indexed => false,
                        } =>
                    {
                        let (sites, locals) = value.levels_below(
                            source.local,
                            match source.shape() {
                                Shape::Deref(depth) => depth,
                                Shape::Local | Shape::Indexed => 0,
                            },
                        );
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
                        Some(Operand::Copy(source))
                            if (match source.shape() {
                                Shape::Local => true,
                                Shape::Deref(_) | Shape::Indexed => false,
                            }) =>
                        {
                            Some(source.local)
                        }
                        Some(Operand::Copy(_) | Operand::Constant(_)) | None => None,
                    },
                    by: Carrier::Copy,
                };
                let lands_in_no_block = may_land
                    .as_ref()
                    .is_none_or(|(blocks, _, _, _, _)| blocks.is_empty());
                let before = value.written.clone();
                if let Some((may_land, several, alone, landing, place)) = may_land {
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
                    // lands in takes a may-write in `Self::blocks_written`,
                    // which leaves one that
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
                    let dest_at = match dest {
                        Some(Operand::Copy(dest))
                            if (match dest.shape() {
                                Shape::Local => true,
                                Shape::Deref(_) | Shape::Indexed => false,
                            }) =>
                        {
                            value.points_to[dest.local].position.at
                        }
                        Some(Operand::Copy(_) | Operand::Constant(_)) | None => At::Unknown,
                    };
                    let covers_whole = |value: &Known, site: Site| {
                        value.covers_whole(Storage::Heap(site), dest_at, bytes)
                    };
                    // **The blocks it writes are the store's to mark**
                    // (#1005): the copy writes where its landing lands, whole
                    // where it covers a block and the slots it covers where
                    // one is kept per slot, as a store through `*dest` does.
                    // Written whole, `memcpy(b, &x, 8)` into a block of two
                    // slots left `b[1]` read in silence where `b[0] = x;` is
                    // `SC0411` (#925). A copy that writes no pointer whole
                    // writes none of them.
                    if !unwritten {
                        self.blocks_written(value, &place, &landing, bytes, whole);
                    }
                    for site in before.sites() {
                        let was = &before[site];
                        if may_land.contains(&site) && !unwritten {
                            if doubted && was.whole().may_be_unwritten() {
                                value.written[site].doubt();
                            }
                            // **Certain only where it covers the whole block**,
                            // its size a constant and the length at least that:
                            // a copy of 8 bytes into 16, or of a length this
                            // check cannot read, leaves what it did not cover
                            // as it was, and that may be written. Found by
                            // review.
                            let covers = covers_whole(value, site);
                            if source_fresh
                                && alone
                                && covers
                                && matches!(length, CopiedPointers::Whole)
                            {
                                value.written[site].set(Init::Unwritten(whole));
                                value.written[site].carried_by(copied);
                                carried = true;
                            } else if source_maybe {
                                value.written[site].doubt();
                                // **Said to be the copy's only where it
                                // covers the whole block from its start**,
                                // so that every byte the read may reach came
                                // from it: a string copy that stops at a
                                // null, or a copy of one byte, reached part
                                // of the block, and a slot it never reached
                                // was named as carried by it (#752, #918's
                                // review). Nor a string copy, whose bytes
                                // past a null are zeros it wrote.
                                if alone && covers && byte_for_byte {
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
                            if match dest.shape() {
                                Shape::Local => true,
                                Shape::Deref(_) | Shape::Indexed => false,
                            } {
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
                        if match dest.shape() {
                            Shape::Local | Shape::Deref(_) => true,
                            Shape::Indexed => false,
                        } =>
                    {
                        Some(value.locals_a_write_may_land_in(
                            dest.local,
                            (match dest.shape() {
                                Shape::Deref(depth) => depth,
                                Shape::Local | Shape::Indexed => 0,
                            }) + 1,
                        ))
                    }
                    // A constant writes nothing anyone can read back, and no
                    // other place reaches a call's argument from C.
                    _ => None,
                };
                // And through a destination moved off an array, an element of
                // it, as a store through it answers. See
                // `built::write_effects`.
                if let Some(at) = self.copy_replaces_one_local(function, value, dest, bytes) {
                    let writes = match (dest, &written) {
                        (Some(Operand::Copy(dest)), Some(lands)) => write_effects(
                            self.unit,
                            function,
                            value,
                            dest.local,
                            (match dest.shape() {
                                Shape::Deref(depth) => depth,
                                Shape::Local | Shape::Indexed => 0,
                            }) + 1,
                            lands.iter().copied(),
                            true,
                            bytes,
                        ),
                        _ => Vec::new(),
                    };
                    let pending = value.array_writes_before(writes);
                    // **Not where it replaced its one local**, as a store's
                    // certain write is not (ADR-0028): the copy wrote that local
                    // whole, and `replaced` marking it as a call may have written
                    // it took away the belief the copy gave, so `memcpy(&s, &v,
                    // 8)` doubted `*s` where `s = v;` did not (#769).
                    // It wrote that local, which is what `replaced` would have said
                    // of it.
                    value.written_local(at.into(), whole);
                    value.array_writes_after(pending, whole);
                } else if let (Some(Operand::Copy(dest)), Some(_)) = (dest, &written) {
                    // **The locals it may land in are the store's to answer**
                    // (#1006), for the place `*dest`, with the copy's own arms.
                    let mut place = (*dest).clone();
                    place = place.deref();
                    self.locals_written(function, value, &place, StoreKind::Copy, bytes, whole);
                } else {
                    // A destination that is no place of dereferences may land
                    // anywhere, as a call's argument may.
                    value.replaced(|_| true, |_| true, WrittenBy::StoreOrCopy(whole));
                }
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
                    let at_start =
                        (match dest.shape() {
                            Shape::Local => true,
                            Shape::Deref(_) | Shape::Indexed => false,
                        }) && matches!(value.points_to[dest.local].position.at, At::Bytes(0));
                    for &local in lands {
                        let covers = value.covers_whole(
                            Storage::Local(local),
                            value.points_to[dest.local].position.at,
                            bytes,
                        );
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
                            value.written[local].set(Init::Unwritten(whole));
                            value.written[local].carried_by(copied);
                        } else if source_maybe {
                            value.written[local].doubt();
                            // Said to be the copy's only where it covers the
                            // whole local from its start, as into a block.
                            if lands.len() == 1
                                && lands_in_no_block
                                && at_start
                                && covers
                                && byte_for_byte
                            {
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
    /// is reported as certain. **`loaded`, among the reasons
    /// `Beyond::any_unknown` asks, is held** by
    /// `a_pointer_no_call_can_reach_is_not_written_by_a_store_this_check_cannot_place`,
    /// which gains a false certain report without it, and by
    /// `a_pointer_read_out_of_memory_that_names_a_block_nothing_wrote_is_doubted_where_read_through`,
    /// whose doubt becomes certain: such a pointer is a doubt, in
    /// `reads_partly` (#728). **`lost` and `foreign` there have no case**,
    /// measured: a local loses its sites where it is made lost, and no
    /// program found gives a foreign pointer a fresh site that an exposure
    /// has not already cleared.
    /// The slot of a block a read through `through` lands in: the element
    /// it names, or **where that is not known, the slot its counted
    /// position is the start of**. A join of a block kept per slot with one
    /// that is not leaves no element, while both sides are at `Bytes(0)`:
    /// read as no element, `m = c ? realloc(m, 16) : m; r = *m;` asked
    /// every slot of the grown block and doubted `*m` (#918's review). A
    /// position counts bytes from the start of the object the value names
    /// (ADR-0092 rule 1), so it is the slot where it is a whole number of
    /// pointers; anywhere else it names none.
    fn slot_read(&self, through: &Held) -> Option<u64> {
        let target = self.unit.target();
        let pointer =
            u64::from(target.pointer_bits() / target.integer(Rank::Char, Sign::Plain).bits());
        through.site_element.or(match through.position.at {
            At::Bytes(at) if at % pointer == 0 => Some(at / pointer),
            At::Bytes(_) | At::Within { .. } | At::Moved | At::Unknown => None,
        })
    }

    /// The slots a read of `bytes` bytes through `through` touches, a byte
    /// of each at least: from the slot its element or its counted position
    /// is in, as [`Self::slot_read`] finds it, to the one its last byte is
    /// in. `None` where either is not known, or the read is empty.
    ///
    /// Mutation: start at slot 0, and
    /// `a_copy_out_of_a_slot_nothing_wrote_beside_a_written_one_is_reported_where_read`
    /// goes silent; end at the last slot it reads whole, and
    /// `a_copy_reaching_into_a_slot_nothing_wrote_leaves_what_it_lands_in_in_doubt`
    /// does.
    pub(super) fn slots_read(&self, through: &Held, bytes: Option<u64>) -> Option<(u64, u64)> {
        self.slots_read_from(through.site_element, through.position.at, bytes)
    }

    /// [`Self::slots_read`] of a local the value names, from the element it
    /// names there (`Held::element`) or its counted position: **the one
    /// rule the copy's transfer and its report both read a source by** (the
    /// memory model rebuilt on #924, stage 2). Asked of a local at any
    /// element by the transfer and slot by slot by the report,
    /// `a[0] = x; memcpy(out, a, 8); r = *out;` was doubted at `*out` and
    /// asked nothing at the copy.
    pub(super) fn slots_read_of_locals(
        &self,
        through: &Held,
        bytes: Option<u64>,
    ) -> Option<(u64, u64)> {
        self.slots_read_from(through.element, through.position.at, bytes)
    }

    fn slots_read_from(
        &self,
        slot: Option<u64>,
        offset: At,
        bytes: Option<u64>,
    ) -> Option<(u64, u64)> {
        let target = self.unit.target();
        let pointer =
            u64::from(target.pointer_bits() / target.integer(Rank::Char, Sign::Plain).bits());
        let start = match slot {
            Some(element) => element.checked_mul(pointer)?,
            None => match offset {
                At::Bytes(at) => at,
                At::Within { .. } | At::Moved | At::Unknown => return None,
            },
        };
        let end = start.checked_add(bytes.filter(|&bytes| bytes > 0)?)?;
        Some((start / pointer, end.div_ceil(pointer)))
    }

    pub(super) fn reads_fresh(&self, function: &Function, source: &Place, value: &Known) -> bool {
        let through = &value.points_to[source.local];
        (match source.shape() {
            Shape::Deref(1) => true,
            Shape::Local | Shape::Deref(_) | Shape::Indexed => false,
        }) && self.may_be_pointer(function, source)
            && (through.sites().next().is_some() || through.may_be_a_locals_address())
            && !(through.beyond().any_unknown() || through.inner_address)
            && through.sites().all(|site| {
                value
                    .site_init_at(site, self.slot_read(through))
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
        let depth = match source.shape() {
            Shape::Deref(depth) => depth,
            Shape::Local | Shape::Indexed => 0,
        };
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
        // **At its start unless it is known to be off it**: a pointer read
        // out of memory whose position no one said may be at a local's
        // start, which is where an edge to a local points. Asked only at a
        // known start, `memcpy(&**m, &v_, 8); r = **m; *r` read a `char`
        // buffer written at one byte in silence, where `**m = v_;` doubts it:
        // the copy then exposed the block it wrote, the walk below the first
        // level added what was exposed, and `r`'s position joined to `Unknown`
        // (#686, measured by the route matrix). A doubt, so it can only add
        // one. Mutation: ask at `At::Bytes(0)` alone again;
        // `a_long_local_array_written_at_one_element_is_doubted_where_read_at_another`
        // and the route rows `part_local_restored_d2_both` and
        // `part_local_ret_d1_one` fail, among others. The case written for
        // #686 no longer depends on it since the copy stopped exposing what
        // it is handed (#1014).
        let at_start = !matches!(through.position.at, At::Moved);
        let maybe = |local: LocalIdx| value.may_be_unwritten(local, through.element, at_start);
        // **Any block it may read that may still be fresh is enough**,
        // fresh or maybe so, beside blocks that were written: `p = a; if (c)
        // p = b;` with `b` written reads `a` on one path, which is undefined
        // there whatever the value is read for (#724). `reads_fresh` answers
        // first, and asks every block.
        let site_maybe = |site: Site| {
            value
                .site_init_at(site, self.slot_read(through))
                .may_be_unwritten()
        };
        (match source.shape() { Shape::Deref(1) => true, Shape::Local | Shape::Deref(_) | Shape::Indexed => false })
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
    /// it replaces one: [`Allocations::replaces_whole`], the store's rule,
    /// with a constant length of exactly its size (ADR-0028). Not a longer
    /// one, which writes past the local and is the bounds rule's to report.
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
        if !match dest.shape() {
            Shape::Local | Shape::Deref(_) => true,
            Shape::Indexed => false,
        } {
            return None;
        }
        let landing = self.landing(
            value,
            dest.local,
            (match dest.shape() {
                Shape::Deref(depth) => depth,
                Shape::Local | Shape::Indexed => 0,
            }) + 1,
            match dest.shape() {
                Shape::Local => true,
                Shape::Deref(_) | Shape::Indexed => false,
            },
            StoreKind::Copy,
        );
        self.replaces_whole(function, value, &landing, bytes)
    }
}
