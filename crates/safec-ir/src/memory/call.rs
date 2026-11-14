//! The transfer of a call, [`Allocations::call`]: what a call does to what
//! it was handed and what it leaves behind, with the helpers only it uses.
//! A file of its own (#936) because the call is the transfer fixed most
//! often, so a change to it is read without the rest of `transfer.rs`.

use crate::ir::{
    BlockId, FuncId, Function, LocalId, Operand, Origin, Place, Projection, Rvalue, Terminator, Ty,
    TyId,
};
use crate::library::Library;
use crate::source::Span;

use super::allocations::{Allocations, handed_places, innermost_element};
use super::built::{named, replaced_by};
use super::fit::{Fit, fits, read_from_locals, slots_from, slots_leaving};
use super::index::{Idx, Loc, LocalIdx, Site, Table, Target, Why};
use super::init::Wrote;
use super::known::{ArrayWrite, ELEMENTS_KEPT, Known, WrittenBy};
use super::parts::{
    At, Cell, Extent, Freeing, Held, Init, Inits, Levels, PlaceInit, Read, Realloced, SiteState,
    Width,
};
use super::store::{Landing, StoreKind, Strong};
use super::transfer::{CopiedPointers, Pours, ReadAs, believed_width};
use super::{Carried, Carrier};
use crate::library::{Callee, Leaves, Makes, Reach};
use crate::reads::derefs;

impl Allocations<'_> {
    /// A fill of bytes this check does not take for a pointer, landing as
    /// `landing`: every block and local it lands in may now hold a pointer
    /// this check lost, beside what it held. A union, since the fill may land
    /// elsewhere and may cover part of the slots. See #948.
    fn fill_lands_lost(&self, landing: &Landing, value: &mut Known) {
        let mut lost = Held::none(value.points_to.len());
        lost.name_unknown(Why::Forged);
        let poured = Cell::poured(
            value.points_to.len(),
            std::iter::empty(),
            std::iter::empty(),
            &Levels::new(vec![Width::Unbounded]),
            &lost,
        );
        self.land_contents(value, landing, &poured, &[], Pours::Copy { replaces: None });
        for &local in &landing.targets {
            value.points_to[local].name_unknown(Why::Forged);
        }
    }

    /// Whether `dest`, a pointer a fill or a copy writes through, points at
    /// memory whose type holds pointers: bytes written into `int`s or
    /// `char`s are read back as what they are, and only a slot of pointers
    /// reads them as an address.
    fn holds_a_pointer_at(&self, function: &Function, dest: &Place) -> bool {
        self.unit
            .place_ty(function, dest)
            .is_some_and(|ty| self.first_level_holds_a_pointer(ty))
    }

    /// Whether `source`, a pointer a library copy reads through, points at
    /// memory whose type holds no pointer: `char`s or `int`s, or an array of
    /// them. Not `void`, which says nothing of what is there.
    fn holds_no_pointer(&self, function: &Function, source: &Place) -> bool {
        let Some(ty) = self.unit.place_ty(function, source) else {
            return false;
        };
        let Ty::Pointer(pointee) = self.unit.ty(ty) else {
            return false;
        };
        let inner = innermost_element(self.unit, pointee);
        matches!(self.unit.ty(inner), Ty::Char | Ty::Int)
    }

    /// What an argument spelled as `place` hands a call: the local's value
    /// where it is a local; **the one local a dereference certainly reads**,
    /// `fill(*pp)` with `pp` certainly `&p` handing `p`'s value; and
    /// otherwise what a load of it holds, marked as a load, which names what
    /// this check does not follow beside it. Read as a load wherever it is
    /// certain, `fill(*pp)` with `p` certainly `&a` doubted that the call
    /// wrote `a`, and a corpus case kept quiet on purpose was reported.
    /// Found while implementing #949.
    fn handed_value(&self, function: &Function, place: &Place, value: &Known) -> Held {
        let depth = derefs(place);
        if place.projection.is_empty() {
            return value.points_to[place.local].clone();
        }
        if depth == place.projection.len() {
            if let (locals, true) = read_from_locals(function, value, place.local, depth) {
                if let [read] = locals[..] {
                    return value.points_to[read].clone();
                }
            }
        }
        self.read_through(function, place, value)
    }

    /// The transfer of `Terminator::Call`, after
    /// [`Analysis::terminator`](crate::dataflow::Analysis::terminator) has
    /// noted what the call dereferences: `callee` called with `arguments`,
    /// writing `destination`, returning to `then`, at `origin`, which is
    /// `terminator`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call(
        &self,
        function: &Function,
        callee: &FuncId,
        arguments: &[Operand],
        destination: &Option<Place>,
        then: &Option<BlockId>,
        origin: &Origin,
        terminator: &Terminator,
        value: &mut Known,
    ) {
        // **What the call is handed is carried forwards, as a dereference is
        // by `Analysis::terminator` before it hands the call here.** The
        // callee's body reads it, and C17 6.5.2.2 p10 leaves that body indeterminately sequenced with a later call in
        // the same full expression, unless that call encloses this one, whose
        // arguments it orders before itself (ADR-0043, in `used_before`).
        // Before the transfer below, for `PendingRead`'s reason:
        // the sites are the ones held where the call is reached, which
        // `what_a_call_was_handed_is_carried_as_it_was_before_the_call` holds
        // where the call writes into the local it was handed. The
        // arguments are exactly the ones `handed` asks, from one function, so
        // the two readers cannot disagree about which. Dropping this loop
        // silences `a_pointer_handed_to_a_call_the_check_meets_first_is_reported`.
        // See ADR-0042.
        for place in handed_places(self, function, *callee, arguments) {
            let reached = value.handed_reached(place);
            value.meeting(origin.span(), place, Read::Argument, &reached);
        }

        // What the call does to what it was handed, before what it leaves
        // behind, which is the order the two happen in.
        // A `Reached::Lost` moves nothing, because there is nothing to move:
        // what it says is that this call touched something the check was not
        // following, which is a fact about the report rather than about the
        // lattice.
        let kind = self.callee(*callee);
        // `realloc` is asked about its first argument only, as `reported` asks
        // it. No C program shows the difference, since its size holds no
        // allocation; it is written the same way in both places so that the
        // two readings of one rule cannot drift.
        let handed = match kind {
            // **Touches nothing**: what its arguments read was asked above,
            // and nothing after it runs. See `Callee::Ends`.
            Callee::Ends => return,
            Callee::Reallocates => &arguments[..arguments.len().min(1)],
            Callee::Frees
            | Callee::Allocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Opaque => arguments,
        };
        let touched = Self::touching(handed.iter(), value);
        let sites = || named(&touched);

        // **What a later call may have done to an old allocation is not what
        // the `realloc` did**, so a fact naming one this call touches is
        // forgotten, before anything this call records: a free of `p`, a
        // second `realloc` of it, or `g(p)` between the call and the branch
        // left `p` unproven, and the null arm brought it back live. Found by
        // review. See ADR-0039.
        let touched_now: Vec<Site> = sites().collect();
        value.forget_reallocs_touching(|site| touched_now.contains(&site));

        // **An allocation this may free stays fresh.** Clearing it here was
        // tried, to spare a read out of freed memory a second report, and
        // `fresh` is an intersection that nothing sets again: a `realloc`
        // that fails leaves the old block live and unwritten (C17 7.22.3.5
        // p4), and a free of one of two allocations leaves the other, so a
        // read on either path built in silence or as a doubt. Mutation: clear
        // it for a `realloc`, and
        // `a_pointer_read_out_of_the_old_block_after_a_failed_realloc_is_reported`
        // builds. See ADR-0057.

        // **What a `realloc` with a non-zero constant size was handed, while
        // it is all live**, before the transfer below makes it unproven: the
        // branch on the result is what says which it became. **Not for a
        // pointer a call this check cannot read wrote**, which may hold what
        // it does not name, as at a free (#497): `r` is `u` on one arm and `q`
        // on the other, and a fact naming `q` alone proved `free(q)` a double
        // free on the arm where `realloc` was handed `u` (#499). See ADR-0039.
        let realloced: Option<Vec<(Site, Option<Span>)>> = match (kind, arguments) {
            (Callee::Reallocates, [Operand::Copy(old), Operand::Constant(size), ..])
                if *size != 0 && old.projection.is_empty() =>
            {
                let held = &value.points_to[old.local];
                let old: Vec<(Site, Option<Span>)> = held
                    .sites()
                    .filter_map(|site| match value.state[site] {
                        SiteState::Live(made) => Some((site, made)),
                        SiteState::Freed { .. }
                        | SiteState::Unknown
                        | SiteState::Reachable
                        | SiteState::Unheld => None,
                    })
                    .collect();
                (!held.beyond().stopped_following_or_written_unseen()
                    && !old.is_empty()
                    && old.len() == held.sites().count())
                .then_some(old)
            }
            (Callee::Reallocates, _)
            | (
                Callee::Frees
                | Callee::Allocates
                | Callee::ReturnsFirst
                | Callee::Copies
                | Callee::Ends
                | Callee::Opaque,
                _,
            ) => None,
        };

        // **A free of what may be the caller's may free anything the caller
        // can see**, as a call this check cannot read may: `f(p, p)` hands
        // `free(b); return *a;` one allocation twice. **Before the `match`,
        // because the free's own arm returns early** for a pointer read out
        // of memory, which is what a pointer read out of caller memory is:
        // after it, `q = *pp; free(q); return **pp;` went silent. Found by
        // review. See ADR-0040.
        // **A free or a `realloc` of a value this check lost may release any
        // lost value**, so every one is marked as one that may be gone, before
        // the `match` for the reason the caller's rule below gives. A lost
        // local, a load, or a place read through memory, each of which may
        // hold one: `t = *box; free(t);` frees what `free(*box)` frees, and
        // was read in silence after, found by review. What is marked is still
        // only what is lost, not the load: a load may be null, and marking it
        // doubted a read behind `if (r)` of a pointer `free(NULL)` left alone,
        // found by review of #501. See `Known::lost_may_be_gone`.
        // **And a call this check cannot read may release one**, since what
        // it is handed it may free: a lost local or one read out of memory a
        // lost value may be in, or memory that reaches an allocation holding
        // one. `release(u)`, and `*holder = u; release(holder);`, both freed
        // `u` and left `r = *box; v = *r;` read in silence (#505). The same
        // memory the `Callee::Opaque` arm treats as possibly freed, and
        // nothing a call is not handed: `release_all()` is not a trigger,
        // unless an escaped local may hold a lost value, below.
        //
        // **Spelled any way it can be handed.** A load handed as an operand,
        // `release(*box)`, names nothing and is read out of `box`, which is
        // marked rather than anything in it; and a local's address,
        // `release_pp(&u)`, hands what the local holds, so the locals it
        // points at are asked as the argument is, and their allocations are
        // in what it reaches. Each read `*r` in silence. Found by review.
        //
        // **And what an escaped local may hold**, as `Known::reach_of` has a
        // named value's call reach it: the call may have been handed the
        // local's address now, earlier or by another route. `stash_pp(&w);
        // w = u; r = fetch(); free(u);` read `*r` in silence where a named
        // `u` was reported (#515). **And what it points at**, whose closure
        // `reach_of` adds too: `*box = u; s = *box; stash_ppp(&w); w = box;
        // release_all();` freed what `s` holds, read after in silence.
        // Found by review.
        //
        // Computed once, for the trigger here and for the call's result
        // below, so that the two cannot disagree about what it was handed.
        let opaque = match kind {
            Callee::Opaque => true,
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Ends => false,
        };
        let hands_a_lost_value = opaque && {
            let mut handed_memory: Vec<Site> = sites().collect();
            for local in value
                .escaped
                .indices()
                .filter(|&local| value.escaped[local])
            {
                handed_memory.extend(value.points_to[local].sites());
            }
            for argument in handed {
                handed_memory.extend(self.read_out(function, argument, value));
                if let Operand::Copy(place) = argument {
                    let pointed: Vec<LocalIdx> = value.written_through(place.local);
                    for target in pointed {
                        handed_memory.extend(value.points_to[target].sites());
                    }
                }
            }
            let may_hold_lost = |held: &Held| held.beyond().may_hold_a_lost_value();
            handed.iter().any(|argument| match argument {
                Operand::Copy(place) => {
                    may_hold_lost(&value.points_to[place.local])
                        || (!place.projection.is_empty()
                            && value.marked_below(place.local, derefs(place), |contents| {
                                contents.lost()
                            }))
                        || value
                            .written_through(place.local)
                            .into_iter()
                            .any(|target| may_hold_lost(&value.points_to[target]))
                }
                Operand::Constant(_) => false,
            }) || value
                .closure(handed_memory)
                .into_iter()
                .any(|site| value.contents[site].lost())
                || value
                    .escaped
                    .indices()
                    .any(|local| value.escaped[local] && may_hold_lost(&value.points_to[local]))
        };
        if hands_a_lost_value {
            value.lost_may_be_gone();
            value.exposed[Target::Unknown(Why::Lost)] = true;
        }

        let may_free = match kind {
            Callee::Frees | Callee::Reallocates => true,
            Callee::Allocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Ends
            | Callee::Opaque => false,
        };
        if may_free
            && handed.iter().any(|argument| match argument {
                Operand::Copy(place) => {
                    let held = &value.points_to[place.local];
                    !place.projection.is_empty() || held.beyond().any_unknown()
                }
                Operand::Constant(_) => false,
            })
        {
            value.lost_may_be_gone();
        }

        if may_free && self.frees_callers(handed, value) {
            value.callers_memory_may_be_freed();
        }

        // **A fill of part of a pointer writes part of one**, as a copy of
        // that length does: `memset(m, 1, 4)` into memory later held as
        // pointers left a pointer of four set bytes and four others in
        // silence (#913). Only a row that `fills`, whose bytes are its own; a
        // string copy's are a copy's, asked by the copy's rules.
        //
        // **Asked of every row, whatever it does besides, at the length its
        // own row says**: asked inside the arm of the rows that copy, and of
        // the third argument, a row that fills with another `Callee` or its
        // length elsewhere, `bzero(m, 4)`, set the column and stayed
        // silent. Found by review.
        if let Some(length) = self.library(*callee).and_then(Library::fill_length) {
            let covered = self.pointers_covered(*callee, arguments, length);
            // **Bytes it fills are no pointer this check follows**, unless
            // they are all zero: `memset(a, 1, 8)` over a slot of pointers
            // leaves a pointer of eight ones there, and read back it named
            // nothing, so a null test took it for null and the dereference
            // after it fit vacuously; over a slot holding `&x` it left `&x`
            // believed. A slot it covers whole may now hold a pointer this
            // check lost, beside what it held, wherever the fill lands, and
            // a dereference of it is doubted, as every lost value's is.
            // Found by a hunt (#948).
            let zero = matches!(arguments.get(1), Some(Operand::Constant(0)));
            if !zero && !matches!(covered, CopiedPointers::NoneWhole) {
                if let Some(Operand::Copy(dest)) = arguments.first() {
                    let into_depth = derefs(dest);
                    if dest.projection.is_empty() || into_depth > 0 {
                        let landing = self.landing(
                            value,
                            dest.local,
                            into_depth + 1,
                            into_depth == 0,
                            StoreKind::Copy,
                        );
                        if self.holds_a_pointer_at(function, dest) {
                            self.fill_lands_lost(&landing, value);
                        }
                    }
                }
            }
            if !matches!(covered, CopiedPointers::Whole) {
                // Wherever it lands, as a store's write does. See
                // `Self::note_landing_in_part`.
                if let Some(Operand::Copy(dest)) = arguments.first() {
                    let into_depth = derefs(dest);
                    if dest.projection.is_empty() || into_depth > 0 {
                        let landing = self.landing(
                            value,
                            dest.local,
                            into_depth + 1,
                            into_depth == 0,
                            StoreKind::Copy,
                        );
                        self.note_landing_in_part(&landing, origin.span(), value);
                    }
                }
            }
        }

        match kind {
            Callee::Ends => {
                unreachable!("`Allocations::call` returns for it where `handed` is decided")
            }
            Callee::Frees => {
                let reached: Vec<Site> = sites().collect();

                // **A free of a local that may hold something this check
                // cannot name does not say which allocation went.** The sites
                // below are what the local is *thought* to hold, and a call
                // this check cannot read may have written a fresh pointer over
                // it since; writing `Freed` on them would be a proof about an
                // allocation the callee may have swapped out. The site is
                // shared, so that proof is handed to every other local holding
                // it, which is how a program C defines became an `error` no
                // flag suppresses. See ADR-0029.
                //
                // **It supersedes both rules below and skips nothing else.**
                // Neither of them applies once the set is not known to be what
                // was freed: the one is about which member of a set went, and
                // the other about a single member going. An early return can
                // leave the conservative rules behind it unrun, and the rule
                // here is the conservative one.
                //
                // **What it does skip is the destination.** The fall-through
                // path clears the local `free` is written into and this does
                // not, which the `reached.len() > 1` branch below does too.
                // The C frontend gives `free` none, and one another frontend
                // gives it is `void` and holds none of this; measured, putting
                // the clear back inside this branch leaves the whole workspace
                // green.
                if Self::holds_something_unnameable(arguments, value) {
                    for site in reached {
                        // **A free cannot un-free an allocation.** A site an
                        // earlier free this check *could* follow has already
                        // proved is not something this call has anything to
                        // say about, and the site is shared, so blanking it
                        // takes the proof away from every other local holding
                        // it. Found by review;
                        // `a_free_this_check_could_not_follow_leaves_a_proved_free_alone`
                        // is the program and is the guard.
                        if !matches!(value.state[site], SiteState::Freed { .. }) {
                            value.state[site] = SiteState::Unknown;
                        }
                    }

                    return;
                }

                // **A may-set is not a must-set, and this is where the two used
                // to be confused.** Freeing a local that may hold either of two
                // allocations frees exactly one of them; writing `Freed` on
                // both claimed each was certainly freed, and a later free of
                // one of them by name was then a proved double free about a
                // program that frees each exactly once on one of its paths.
                //
                // So nothing is written on the members. They stop being
                // provable, and the fact that one of them went is recorded on
                // the local that named the set, which is the only thing in this
                // lattice that names a set. See ADR-0020.
                if reached.len() > 1 {
                    for site in &reached {
                        value.state[*site] = SiteState::Unknown;
                    }

                    for argument in arguments {
                        // A constant frees nothing and a projection names a
                        // place this check does not follow, which is what
                        // `Allocations::touching` already said about both.
                        //
                        // **No program reaches the second of those, and it is
                        // here anyway.** `free` takes one argument, and an
                        // argument with a projection reaches no site at all,
                        // so this branch is not entered with one. What it
                        // would cost if that changed is a proof recorded
                        // against the pointer rather than against what it
                        // points at, which is the false proof this whole rule
                        // is against.
                        //
                        // Measured, and the narrow claim is the true one:
                        // *removing* the guard breaks nothing, because the
                        // projection here is always empty. Negating it breaks
                        // two named tests, because then nothing is recorded at
                        // all.
                        let Operand::Copy(place) = argument else {
                            continue;
                        };
                        if place.projection.is_empty() {
                            value.points_to[place.local].freed = Some(Freeing::new(origin.span()));
                        }
                    }

                    return;
                }

                for site in reached {
                    // Whatever the site was known to have come from survives
                    // the free: the diagnostic wants to name it.
                    let made = match value.state[site] {
                        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                        SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
                    };
                    // **A free already ordered before here is the one to
                    // keep.** It is what proves anything about what follows,
                    // and it is what the diagnostic has to point at for the
                    // proof to be readable: `free(p); x = (free(p), 0) + *p;`
                    // is a proved use after free because of the first line,
                    // and replacing it with the second would name a free that
                    // is in the same unsequenced expression as the use and
                    // leave the reader with two carets that prove nothing.
                    // Without this the second free took the proof away with
                    // it, which review measured. See ADR-0022.
                    //
                    // It is also the earliest, which is what this variant's
                    // own doc comment has always said it holds.
                    let freed = match value.state[site] {
                        SiteState::Freed { freed, .. } if freed.sequenced => freed,
                        _ => Freeing::new(origin.span()),
                    };
                    value.state[site] = SiteState::Freed { made, freed };
                }
            }
            // It does not free what it is passed, which is the whole of why the
            // name is read.
            Callee::Allocates => {}
            // May have freed it, and has not if it failed. See ADR-0039.
            Callee::Reallocates => {
                for site in sites().collect::<Vec<_>>() {
                    value.state[site].may_be_freed();
                }
            }
            // Frees nothing; what it is handed is out of this check's sight
            // from now on, and a local whose address it is handed may have
            // been written through it, as ADR-0029 says of an opaque call.
            Callee::ReturnsFirst | Callee::Copies => {
                // **The copy door's answer for every field, never `..`**, so
                // a field added to `Known` answers here for what a library
                // copy does with it. See ADR-0091.
                let Known {
                    // Through `Self::landing`, `Known::load_into` and the
                    // walk below: what each side names, and a local the copy
                    // lands in.
                    points_to: _,
                    // Not touched by a copy.
                    state: _,
                    // Through `Known::replaced` and `Known::reach_of`: a local
                    // the copy may land in, and what an escaped one reaches.
                    escaped: _,
                    // Not touched by a copy.
                    handed_away: _,
                    // Through `Known::expose`: what the call is handed, as at
                    // any call of this family.
                    exposed: _,
                    // Through `Known::pour` and the walk: what the
                    // destination's blocks may contain gains what the source's
                    // do.
                    contents: _,
                    // Not touched by a copy.
                    realloced: _,
                    // Through `Self::initialised_by` and the array writes:
                    // what the copy writes, slot by slot where the place keeps
                    // slots, and whether what it carries was written; and
                    // through `Self::land_contents`.
                    written: _,
                    // Through `Known::noticed`, as at any call.
                    pending: _,
                    calls: _,
                    exposed_after_call: _,
                    // Not touched by a copy.
                    // Read: how much the copy covers, and how wide what it
                    // carries is believed.
                    extent: _,
                    handed_at_most: _,
                    // Not touched by a copy.
                    exposed_in_part: _,
                    // Through `Known::pour` and `Known::load_into`.
                    stored_in_part: _,
                    // Through `Self::mark_leaving`: a copy into memory that
                    // leaves marks what it carries (ADR-0085).
                    stored_out: _,
                    left_at: _,
                    // Read, by `Known::held_below`, for how far down that
                    // marking goes.
                    held_levels: _,
                    // Through `Known::note_part_written` and
                    // `Known::carry_part_written`: part of a pointer written,
                    // or carried (ADR-0087).
                    part_written: _,
                    // Not touched by a copy.
                    handed_untyped: _,
                } = &*value;
                // **A copy of an object carries what it contains**: what each
                // allocation the destination holds may contain gains what the
                // source does, which `levels_below` answers for an allocation
                // and, through its edge, for a local's address; and where the source
                // may hold one this check stopped following, so may the
                // destination. Before what follows, which is the whole family's.
                // See ADR-0039 and ADR-0045.
                //
                // **Either argument may be a place of dereferences**, `memcpy(*pp,
                // *ps, 8)`, read as a store and a load through the same place
                // are: the destination's allocations are what `*pp` may point
                // at, and what is copied is one level below what `*ps` may
                // point at. Read only as plain locals, those built in silence.
                // Found by review.
                let copies = match kind {
                    Callee::Copies => true,
                    Callee::Frees
                    | Callee::Allocates
                    | Callee::Reallocates
                    | Callee::ReturnsFirst
                    | Callee::Ends
                    | Callee::Opaque => false,
                };
                if copies {
                    if let [Operand::Copy(dest), Operand::Copy(source), ..] = arguments {
                        let (into_depth, from_depth) = (derefs(dest), derefs(source));
                        let placed = (dest.projection.is_empty() || into_depth > 0)
                            && (source.projection.is_empty() || from_depth > 0);
                        if placed {
                            let (copied, copied_locals) =
                                value.levels_below(source.local, from_depth + 1);
                            // **What a copy that leaves carries is stored out**,
                            // as a store's is, asked before it lands: the sites
                            // a load of the source reads, and everything they
                            // hold. Without it `memcpy(pp, b, 8); free(x);`
                            // with `*b = x` was silent where `*pp = *b;` is
                            // `SC0414` (#684). See `Known::stored_out`.
                            let marks = self.store_at(terminator, value).is_some_and(|store| {
                                match store.kind {
                                    StoreKind::Copy => store.leaves,
                                    // `store_at` makes a copy alone; listed so
                                    // that a kind added is answered here.
                                    StoreKind::Direct | StoreKind::Through { .. } => false,
                                }
                            });
                            if marks {
                                // The more of the two sides, since a pointer
                                // laundered through a `void *` makes either
                                // one's type a claim nothing checks.
                                let levels = self
                                    .copy_levels_holding_pointers(
                                        self.unit.place_ty(function, source),
                                    )
                                    .max(self.copy_levels_holding_pointers(
                                        self.unit.place_ty(function, dest),
                                    ));
                                self.mark_leaving(
                                    value,
                                    copied.iter().copied().collect(),
                                    Vec::new(),
                                    levels,
                                    origin.span(),
                                );
                            }
                            // **What it copies is a load of the source one level
                            // further**, `memcpy(box, pp, 8)` being `*box =
                            // *pp;`, built by the load's own `Allocations::read_at`
                            // rather than by a copy of it to drift (#777): the
                            // sites and addresses one level below the source,
                            // whatever type the source was declared with, since
                            // `memcpy` copies bytes and the usual source is a
                            // `void *`, and every mark a load sets. Among them
                            // whether what was stored there may be lost or gone,
                            // a slot a call could not replace (ADR-0047), what
                            // was read out of caller memory (ADR-0040), and
                            // whether it may be a lost value: `*box = u;
                            // memcpy(box2, box, 8); free(u); r = *box2;` read
                            // `*r` in silence until it was carried. And whether
                            // it reaches an array only by #639's rule, which a
                            // copy written apart once left out where a store
                            // stored such a pointer: `box[0] = m + 1;
                            // memcpy(&g, box, 8); g[0] = &x;` believed `m[0]`
                            // written where `g = box[0];` doubts it. Found by
                            // review, each.
                            let mut read = self.read_at(
                                function,
                                source.local,
                                from_depth + 1,
                                value,
                                ReadAs::Bytes,
                            );
                            // **Bytes out of memory that holds no pointer are
                            // no pointer this check follows**: `memcpy(b, s,
                            // 8)` of a `char s[8]` filled with ones put a
                            // pointer nothing names in `b`'s slot, and read
                            // back, a null test took it for null and the
                            // dereference after it fit vacuously. Where the
                            // source's type holds no pointer and what was
                            // read names nothing, what is copied is a pointer
                            // this check lost. Found by a hunt (#948).
                            if self.holds_no_pointer(function, source)
                                && self.holds_a_pointer_at(function, dest)
                                && read.sites().next().is_none()
                                && read.locals().next().is_none()
                            {
                                read.name_unknown(Why::Forged);
                            }
                            // Where it lands, as a store one dereference
                            // further would. See `store::Landing`.
                            let landing = self.landing(
                                value,
                                dest.local,
                                into_depth + 1,
                                into_depth == 0,
                                StoreKind::Copy,
                            );
                            // **Part of a pointer, recorded on what it lands
                            // in**, for where the block is later held as
                            // pointers to ask (#832). See
                            // `Known::part_written` and ADR-0087.
                            let in_part = match self.copied_pointers(*callee, arguments) {
                                CopiedPointers::LastInPart | CopiedPointers::NoneWhole => true,
                                CopiedPointers::Whole => false,
                            };
                            // **And what the blocks it copies out of
                            // recorded**, whatever its length: a whole copy
                            // of a block a partial copy wrote holds the same
                            // partial pointer, and `memcpy(m2, m, 8); void **q
                            // = m2; *pp = q;` built silent (#905). The blocks
                            // the source names at the copy's depth, which a
                            // load one level further reads. The earlier span
                            // is kept, the partial copy the reader has to
                            // look at. See ADR-0088.
                            //
                            // **Out of and into a local as a block**, the
                            // locals the source names and the ones it lands
                            // in by an edge: through `char buf[8]` the record
                            // was dropped, and a partial copy into one
                            // recorded nothing (#908). See ADR-0090.
                            let (from_sites, from_locals) =
                                value.levels_below(source.local, from_depth);
                            let from: Vec<Loc> = from_sites
                                .into_iter()
                                .map(Loc::Heap)
                                .chain(from_locals.into_iter().map(Loc::Local))
                                .collect();
                            let to: Vec<Loc> = landing
                                .containers
                                .iter()
                                .map(|&site| Loc::Heap(site))
                                .chain(landing.targets.iter().map(|&local| Loc::Local(local)))
                                .collect();
                            // Whether this copy writes a record, by carrying
                            // one or by being partial, for the reset below.
                            let carries = in_part
                                || from
                                    .iter()
                                    .any(|&at| value.part_written[at].now().is_some());
                            value.carry_part_written(self.sources, &from, &to);
                            // **Only where part of a pointer fits**: a local
                            // narrower than a pointer holds no pointer to be
                            // part of, and noted, `memcpy(&k, &i, 4)` of an
                            // `int` into an `int` moved the report off the
                            // copy that wrote half a pointer, found by review.
                            // A later copy of its bytes into wider memory is
                            // a partial copy of its own. A block, or a local
                            // whose size this check cannot read, is noted.
                            if in_part {
                                self.note_less_than_a_pointer(
                                    to.iter().copied(),
                                    origin.span(),
                                    value,
                                );
                            }
                            // **And what was stored through a pointer there is
                            // no longer what a read through it reads**: the
                            // copy wrote the record's bytes over it. Left set,
                            // `*p = x; memcpy(buf, &i, 4); s = *p;` read half a
                            // pointer in silence, for a block as for a local,
                            // found by review. See `Held::stored_through`.
                            let recorded: Vec<Loc> = to
                                .iter()
                                .copied()
                                .filter(|&at| value.part_written[at].now().is_some())
                                .collect();
                            if carries && !recorded.is_empty() {
                                value.written_over(&recorded);
                            }
                            // **Into a local's address, the copy lands in the
                            // local**: `memcpy(&s, &r, n)` stores into `s`
                            // what `r` holds, every mark with it, since C17
                            // 7.24.2.1 p2 copies the bytes. The locals are the
                            // ones the destination's address edges name, or,
                            // for a place of dereferences, the locals the
                            // level above may be, as for a store's deep
                            // targets. How it lands in them is below. See
                            // ADR-0039 and ADR-0040.
                            //
                            // **Whether what it copies may be a pointer that
                            // reaches an array only by #639's rule** is the
                            // load's `array_edge`, carried into a local with the
                            // rest of the value, as a store through the address
                            // gives it, and into memory as the arrays it marks
                            // where it is poured, `Known::stored_in_part`.
                            // Without either `e = m + 1; memcpy(box, &e, 8); f =
                            // box[0]; *f = &x;`, and the same into `&f`, left
                            // `m[0]` believed written where the store form is
                            // doubted.
                            let targets = landing.targets.clone();
                            // **Believed as a load at the local's type would be**,
                            // `s = *(T **)&v`: that type, no wider than the
                            // source's memory recorded, as `believed_read`
                            // answers a load (ADR-0061). Carried with no belief,
                            // `memcpy(&s, &v, 8)` of a whole `int`'s address was
                            // doubted at `*s` where `s = v;` is not (#769).
                            let recorded = value
                                .memory_levels(source.local, from_depth + 1, self.deepest_pointer)
                                .at(0);
                            // **As its type where what it copies is read
                            // fresh**, as `believed_read` believes such a load:
                            // the local then holds what nothing wrote, as one
                            // never assigned does, and a read of it is
                            // reported where it is read, once; nothing was
                            // stored to record a width. Carried with none,
                            // `memcpy(&s, b, 8)` of fresh `b` was doubted again
                            // at `*s` where `memcpy(m, b, 8)` was not (#791).
                            // **At depth zero only**, the one level `*source`
                            // asks. See ADR-0057 and ADR-0061.
                            let fresh = from_depth == 0
                                && self.reads_fresh(
                                    function,
                                    &Place {
                                        local: source.local,
                                        projection: vec![Projection::Deref],
                                    },
                                    value,
                                );
                            // **And it replaces the local where it is the
                            // store's certain one**, as `s = v;` does: one
                            // local and nothing else (`Landing::strong`),
                            // and a constant length of exactly its size. Anywhere
                            // else by union, since `n` may copy
                            // part of a pointer, or none, and a replacement
                            // would prove things about a value nobody wrote
                            // (#395). By union alone, the local's own null was
                            // joined in and took the belief away.
                            let replaces = self.copy_replaces_one_local(
                                function,
                                value,
                                arguments.first(),
                                self.constant_argument(*callee, arguments, 2),
                            );
                            for &target in &targets {
                                let mut each = read.clone();
                                let local = function.locals().nth(target.index());
                                // **By `believed_width`, as a load is**: a
                                // `void *` at what the record says (#929).
                                // Spelled here as a type's width alone,
                                // `memcpy(&w, pv, 8)` proved nothing where
                                // `w = *pv;` does.
                                let typed = local.map_or(Some(0), |local| {
                                    self.typed_width(function, &Place::local(local))
                                });
                                let as_read = typed.unwrap_or(0);
                                let width = believed_width(typed, recorded);
                                each.believe(if fresh {
                                    Some(as_read)
                                } else {
                                    self.read_from_fits(
                                        function,
                                        source.local,
                                        from_depth + 1,
                                        width,
                                        value,
                                    )
                                    .then_some(width)
                                });
                                // The one target, which is all `targets`
                                // holds where the copy replaces it; any
                                // other is a may-write, as a union is.
                                match replaces {
                                    Some(at) if at.local() == target => value.replace(at, each),
                                    // **Naming no element**: a copy that does
                                    // not replace the local may write part of
                                    // a pointer, or none, so what it leaves is
                                    // not the element the source names, and
                                    // taking it made `memcpy(&g, &e, 4); *g =
                                    // &x;` a certain write of `m[1]`, which
                                    // nothing wrote. Found by review.
                                    _ => {
                                        each.element = None;
                                        value.target_union(target, &each);
                                    }
                                }
                            }
                            // **A copy into memory this check cannot place lands
                            // where a store there would**: `memcpy(w, s, 8)`
                            // with `w` a load was believed where `*w = *s;`
                            // was doubted. Found by review. See ADR-0044.
                            // **And the bound on what it copies**, which a copy
                            // carries as a store of a load would: what the
                            // source's memory is believed to hold, from the
                            // type it arrived as or a load at its type, and
                            // what was stored in the memory it points at.
                            // `memcpy(b, m, 8)` with `m` a `void **`
                            // parameter, or `b` filled from memory that held a
                            // `char *` nothing recorded, handed `b` on as an
                            // `int **` with nothing asked (#632). **At every
                            // level**, as a store records it (#634).
                            let mut copied_bound = self.contents_read(function, source, value);
                            copied_bound.bound_by(&value.memory_levels(
                                source.local,
                                from_depth + 1,
                                self.deepest_pointer,
                            ));
                            // **A copy records the width of the bytes it
                            // copies** (#711), so nothing it leaves is without
                            // one, even out of a block a store recorded none
                            // in: marking a copy out of memory nothing wrote
                            // doubted its width beside the `SC0411` already
                            // said there, measured on #767. See `Cell::held`.
                            if copied_bound.at(0) == Width::Nothing {
                                copied_bound.set_first(Width::Unbounded);
                            }
                            let poured = Cell::poured(
                                value.points_to.len(),
                                copied.iter().copied(),
                                copied_locals.iter().copied(),
                                &copied_bound,
                                &read,
                            );
                            let in_part: Vec<LocalIdx> = if read.array_edge {
                                copied_locals.iter().copied().collect()
                            } else {
                                Vec::new()
                            };
                            // Where it lands, unplaced too, with what it
                            // carries, as a store's landing is given. See
                            // `Allocations::land_contents`.
                            //
                            // **And what it replaces**, where it is one
                            // dereference into one block, from its start, of
                            // exactly that block's size (#927). A string copy
                            // replaces nothing, as nothing else here takes
                            // one to write its destination byte for byte.
                            // That is a cost, not a guard: `strncpy` writes
                            // all `n` bytes, padding with nulls, so its block
                            // could be replaced too, and no case holds the
                            // difference.
                            let byte_for_byte = self.library(*callee).is_some_and(|row| {
                                !row.reaches
                                    .iter()
                                    .any(|(_, reach)| matches!(reach, Reach::String))
                            });
                            let replaces = if into_depth == 0 && byte_for_byte {
                                self.replaces(
                                    &landing,
                                    value.points_to[dest.local].offset,
                                    self.constant_argument(*callee, arguments, 2),
                                    value,
                                )
                            } else {
                                None
                            };
                            self.land_contents(
                                value,
                                &landing,
                                &poured,
                                &in_part,
                                Pours::Copy { replaces },
                            );
                        }
                    }
                }
                // What a pointer it was handed may be, read out of memory, as
                // for an opaque call, exposed, and what the copy writes of what
                // is unwritten. See `Allocations::initialised_by` and ADR-0040.
                let reach = self.reach(function, handed, sites(), value);
                self.initialised_by(
                    function,
                    value,
                    Wrote::Copy {
                        dest: arguments.first(),
                        reach,
                        span: origin.span(),
                        length: self.copied_pointers(*callee, arguments),
                        bytes: self.constant_argument(*callee, arguments, 2),
                        source: arguments.get(1),
                        // A source read as far as a string runs is a string
                        // copy, and the table says which of its arguments it
                        // reads so.
                        byte_for_byte: self.library(*callee).is_some_and(|row| {
                            !row.reaches
                                .iter()
                                .any(|(_, reach)| matches!(reach, Reach::String))
                        }),
                    },
                );
            }
            Callee::Opaque => {
                // **What the caller stored is out of sight from here on**, as
                // it is after a free of the caller's pointer. See ADR-0040.
                value.callers_memory_may_be_freed();
                // **What it holds it may have freed**: what it is handed, by
                // name or as a load, the allocations the memory it was handed
                // holds however deep, and what an argument read out of memory
                // may be. Not the locals whose address that memory holds,
                // which `Known::closure` does not follow; what the call
                // reaches through one is answered by `held_out_of_reach`,
                // below. `q = *t; release(q);` and `*d = a; release_in(d);` may
                // each free `a`'s allocation, and neither can replace `a`, so
                // a later call by address asks about it. Not `reach`, which
                // adds every escaped local's sites to every call, so
                // `grow(&a)` would reach `a` there and the in-out idiom would
                // be doubted at the next call by address; what only an address
                // or an earlier exposure reaches stays `Reachable`, below. See
                // ADR-0047.
                //
                // **A proved free stays proved**, whatever the call was
                // handed, because no call un-frees an allocation (ADR-0017).
                // Writing `Unknown` over what an argument named turned a
                // proved use after free into a doubt, which a hatch only lists:
                // `free(p); g(r); return *p;` built in a hatch where `r` may
                // be `p` (#262).
                let mut handed_memory = self.handed_to_opaque_call(function, handed, value);
                for site in value.closure(handed_memory.clone()) {
                    value.state[site].may_be_freed();
                }

                // **Everything this call could reach, and everything reached
                // before it by code this check cannot read**, is unproven
                // after it, because any of it may be what this call frees.
                // What it can reach includes what an argument read out of
                // memory may be, which `reach` says. See ADR-0039.
                let reach = self.reach(function, handed, sites(), value);
                value.expose(self.sources, reach, Some(origin.span()));
                value.unproved_exposed();
                // Pending from here, and not at its own exposure above, which
                // `Known::noticed` leaves out by `by` anyway. See ADR-0042.
                let span = origin.span();
                value
                    .calls
                    .insert((span.file().index(), span.start(), span.end()), span);

                // **A hatch may have reached anything, so every allocation
                // still live is unproven after it.** The loop above is about
                // the sites the arguments name, and this check does not follow
                // a pointer stored into memory it does not model: `*box = p;
                // drop_inner(box);` hands the callee `p`'s allocation one
                // level down, where no argument names it. Everywhere else that
                // gap is answered by the callee's own body being checked, and
                // a hatch's body is the one whose unproven conclusions are
                // listed rather than reported. So what the body cannot answer
                // the caller assumes the worst of, which is ADR-0032's default
                // with nothing declared to narrow it. A proved free stays
                // proved: nothing a callee does un-frees it. See ADR-0038.
                if self.frees_anything(*callee) {
                    for state in &mut value.state {
                        // Reachable: a hatch reaches what it was not handed
                        // only as code this check cannot read. See ADR-0047.
                        state.doubted();
                    }
                }

                // **What it was handed is not all it can reach.** The loop
                // above is about the allocations the arguments name; this is
                // about the *locals* whose addresses are out there, which this
                // call may write a fresh pointer into whether or not it was
                // passed one.
                //
                // **`Callee::Frees` and `Callee::Allocates` are not here**, and
                // what says so is what each is handed rather than a sentence
                // forbidding the write: C17 7.22.3.4 gives `malloc` a size and
                // no address at all, and 7.22.3.3 gives `free` the pointer's
                // *value*, which p2 of that subclause requires to be one an
                // allocation function returned and which 7.22.3 requires to be
                // disjoint from every other object. Neither is ever handed
                // `&p`. That is the whole reason the name is read. `realloc`
                // is handed a pointer's value too, by 7.22.3.5 p2, and is not
                // here either; the library functions that return their first
                // argument can be handed `&p`, and do this in their own arm.
                //
                // **No test holds this**: marking at either arm leaves the
                // whole workspace green, measured. The record says so rather
                // than leaving the next reader to find out by widening it.
                // See ADR-0029.
                //
                // **Every local, because a callee's write has no type this
                // function knows.** The other producer of this fact narrows by
                // the type it writes, which is ADR-0031; a callee's body is not
                // read, so there is nothing here to narrow by. Narrowing this
                // one would need an annotation saying what a callee writes,
                // and `_Nonnull` is not one.
                //
                // **Written, though, only where it can reach**: the locals
                // whose address it was handed, what it can find from there and
                // from everything handed over before, which
                // `Known::expose_locals` walks. `lost` and `foreign` stay on
                // every escaped local, since a route missed there is a free in
                // silence; a route missed here is a report. `int **p = m;
                // tick(); q = *p;` read `m` in silence (#643). See ADR-0057.
                //
                // **No case holds `lost` and `foreign` wider than this set**,
                // measured: giving them the walk's answer as well leaves the
                // whole suite green, since every route the corpus has is one
                // the walk finds. They stay wide because a route it misses
                // would cost a free nobody is told about, not because a case
                // says so.
                //
                // **What an argument may be the address of**: the locals its
                // address edges name, which `written_through` answers for a
                // plain local, or, for a place of dereferences, the locals the
                // level above may be. **An argument naming none hands none**:
                // one this check lost reads as nothing below it. A pointer
                // moved off an array keeps its edge (#639), so `fill(&m[1] -
                // 1)` may write `m`. A report from one naming none is one the
                // reader sees, and widening it would silence every local
                // beside such a call. See ADR-0057.
                let handed_locals = Self::locals_handed_to_opaque_call(handed, value);
                value.expose_locals(handed_locals);
                let may_write: Table<LocalIdx, bool> = Table::from_vec(
                    value
                        .exposed
                        .locals()
                        .map(|local| value.exposed[local])
                        .collect(),
                );
                // And an array handed through a pointer moved off it is
                // written in part, as a store through that pointer is: asked
                // of every local the call may write, since it may write
                // through any pointer it reaches, `fill(&e)` through `e = m +
                // 1`. See `built::write_effects`.
                let mut partial: Vec<(LocalIdx, ArrayWrite)> = Vec::new();
                for argument in handed {
                    if let Operand::Copy(place) = argument {
                        let lands = may_write.indices().filter(|&local| may_write[local]);
                        for (local, write) in super::built::write_effects(
                            self.unit,
                            function,
                            value,
                            place.local,
                            derefs(place) + 1,
                            lands,
                            true,
                            None,
                        ) {
                            if !partial.iter().any(|(each, _)| *each == local) {
                                partial.push((local, write));
                            }
                        }
                    }
                }
                // **And an array handed over in part before is written in
                // part again**: a call this check cannot read may write what
                // an earlier one kept, and only an element of it was in
                // reach. See `Known::exposed_in_part`. Not one this call is
                // handed whole, at its start, which it may write all of:
                // `fill(m)` after `fill(&m[1])`. Found by review of #667.
                let handed_whole = |local: LocalIdx| {
                    handed.iter().any(|argument| match argument {
                        Operand::Copy(place) if place.projection.is_empty() => {
                            let held = &value.points_to[place.local];
                            held.names_local(local) && held.element == Some(0) && !held.array_edge
                        }
                        _ => false,
                    })
                };
                for (local, &may) in may_write.enumerated() {
                    if may
                        && value.exposed_in_part[local]
                        && !partial.iter().any(|(each, _)| *each == local)
                        && !handed_whole(local)
                    {
                        partial.push((local, ArrayWrite::Maybe));
                    }
                }
                partial.retain(|(local, _)| may_write[*local]);
                for (local, _) in &partial {
                    value.exposed_in_part[*local] = true;
                }
                let pending = value.array_writes_before(partial);
                // What each argument hands over, however it is spelled. See
                // `Known::reached_only_beside_another`.
                let holders: Vec<Held> = handed
                    .iter()
                    .filter_map(|argument| match argument {
                        Operand::Copy(place) => Some(self.handed_value(function, place, value)),
                        Operand::Constant(_) => None,
                    })
                    .collect();
                let beside = value
                    .reached_only_beside_another(&holders, |local| self.is_array(function, local));
                value.replaced(
                    |_| true,
                    |local| may_write[local],
                    WrittenBy::Call { beside: &beside },
                );
                value.array_writes_after(pending);
                // **And as what each local it reaches was handed**, which is
                // the most its body can be asked to have written there: a
                // local reached as a `void *` takes a `void *` store, asked
                // its start and no width, and one reached below a `void *`
                // takes whatever the body converts the pointer to. `put(pv,
                // &c)` with `pv` an `int *`'s address converted and `put`
                // taking `void **` left a `char`'s address believed a whole
                // `int` (#632). The walk is the one a departure takes, so the
                // two cannot read a call's arguments differently.
                // An argument past every parameter, to a variadic or an
                // unprototyped callee, is read as its own type, as a
                // departure reads it; a constant hands no local.
                let declared = self.unit.function(*callee);
                let parameters: Vec<LocalId> = declared.parameters().collect();
                for (index, argument) in arguments.iter().enumerate() {
                    let ty = match (parameters.get(index), argument) {
                        (Some(&parameter), _) => declared.local(parameter),
                        (None, Operand::Copy(place)) => match self.unit.place_ty(function, place) {
                            Some(ty) => ty,
                            None => continue,
                        },
                        (None, Operand::Constant(_)) => continue,
                    };
                    let Ty::Pointer(read_as) = self.unit.ty(ty) else {
                        continue;
                    };
                    let (locals, sites, untyped_sites) = slots_leaving(
                        self,
                        function,
                        read_as,
                        &Rvalue::Use(argument.clone()),
                        value,
                    );
                    for (local, read_as) in locals {
                        self.handed(value, local, read_as);
                    }
                    // And the memory it reached, as the locals are. See
                    // `Known::handed_untyped`.
                    for (site, read_as) in sites {
                        self.handed_site(value, site, Some(read_as));
                    }
                    for site in untyped_sites {
                        self.handed_site(value, site, None);
                    }
                }
                // **And below whatever an earlier call was handed untyped**,
                // as it is now (#679): that callee may have kept the address
                // and store through it at this call, so `keep(&v); v = &w;
                // other(); use1(w);` handed a one-byte allocation on in
                // silence. The same walk, from each local and memory at each
                // type it was recorded at. It costs a doubt about
                // whatever is put below a marked local or memory and handed
                // on after any later call, by the user's decision. See
                // `Known::handed_untyped` and ADR-0060.
                let marked_locals: Vec<(LocalId, Option<TyId>)> = function
                    .locals()
                    .flat_map(|local| {
                        value.handed_untyped[local]
                            .iter()
                            .map(move |&read_as| (local, read_as))
                    })
                    .collect();
                let marked_sites: Vec<(Site, Option<TyId>)> = value
                    .handed_untyped
                    .sites()
                    .flat_map(|site| {
                        value.handed_untyped[site]
                            .iter()
                            .map(move |&read_as| (site, read_as))
                    })
                    .collect();
                if !marked_locals.is_empty() || !marked_sites.is_empty() {
                    let (locals, sites, untyped_sites) =
                        slots_from(self, function, value, marked_locals, marked_sites);
                    for (local, read_as) in locals {
                        self.handed(value, local, read_as);
                    }
                    for (site, read_as) in sites {
                        self.handed_site(value, site, Some(read_as));
                    }
                    for site in untyped_sites {
                        self.handed_site(value, site, None);
                    }
                }
                // **And what it wrote there, or into memory it was handed, may
                // be what it was handed**: `get(&r, u)` and `put(box, u)` may
                // store `u`, and `free(u)` then made `*r` a use of freed
                // memory read in silence, for a named allocation as for a
                // lost one (#510). So every escaped local may hold, and every
                // allocation the call was handed may contain, what it was
                // handed. Only that, not everything reachable to code this
                // check cannot read: the reachable set doubted `keep(a);
                // get(&p); *p = 1;`, the output-parameter idiom ADR-0017
                // declines to report, wherever any allocation was exposed. A
                // local that gains a site is a load for its readers, so a use
                // of it is a doubt and never a proof; one that gains none is
                // left as it was, so `get(&u); free(u);` keeps its words. See
                // ADR-0044.
                //
                // **A local's address hands what the local holds**, as it does
                // for the trigger above: `w = u; get(&r, &w);` may store `u`
                // in `r`, and `Known::closure` does not follow a local, so
                // `*r` after `free(u)` was read in silence. Found by review.
                // At any depth, since `pa = &a; get(&r, &pa);` hands `a`'s
                // too and read `*r` in silence the same way (#513).
                let mut deeper: Vec<Site> = Vec::new();
                for argument in handed {
                    if let Operand::Copy(place) = argument {
                        let first = value.written_through(place.local);
                        for &target in &first {
                            handed_memory.extend(value.points_to[target].sites());
                        }
                        for target in value.beneath(first) {
                            deeper.extend(value.points_to[target].sites());
                        }
                    }
                }
                let shallow = value.closure(handed_memory.clone());
                handed_memory.extend(deeper);
                let stored: Vec<Site> = value.closure(handed_memory).into_iter().collect();
                // **What only a deeper level hands is kept out of a local that
                // holds a local's address**: `pp`, in `pp = &p; opaque(&pp);`,
                // points at `p` and not at the allocation `p` holds, and giving
                // it that allocation doubted `*pp = q`, the output-parameter
                // idiom ADR-0017 declines to report; so did giving `pa`, in
                // `two(&pa, &pb);`, what `pb`'s chain hands. Only for those
                // sites, so what one level hands is held as before this rule:
                // kept out of it, `k = 0; r = &k; get(&r, u);` read `*r` after
                // `free(u)` in silence, and so did `void *` spellings in which
                // such a local really may hold it. Read from `Held::locals`
                // rather than from a declared type, which nothing here
                // enforces; so a deeper pointer a callee stores into a local
                // that holds an address, through `void *` or a cast, is
                // missed, as it was before. Found by review. See ADR-0044.
                let deep: Table<Site, bool> = Table::from_vec(
                    value
                        .state
                        .indices()
                        .map(|site| !shallow.contains(&site))
                        .collect(),
                );
                for local in value.escaped.indices() {
                    if !value.escaped[local] {
                        continue;
                    }
                    // **What the callee may write here is believed whole, as
                    // its result is**, since it is a store in the callee's
                    // body, asked there; and only where what the local held
                    // before is whole too, since a callee that writes
                    // nothing leaves that. Believing it whatever it held let
                    // `keep(&a); a = a + 5; other();` pass `a` on as whole.
                    // A belief nothing here can show stays off. See ADR-0060.
                    //
                    // **No wider than it was handed as**, at this call or an
                    // earlier one ([`Known::handed_at_most`], #632).
                    let believed = function.locals().nth(local.index()).and_then(|id| {
                        let width = self.pointee_size(function, &Place::local(id))?;
                        let width = value.handed_at_most[local]
                            .at(0)
                            .bound()
                            .map_or(width, |most| width.min(most));
                        let whole = matches!(
                            fits(self, function, id, width, value, false),
                            None | Some(Fit::Inside)
                        );
                        whole.then_some(width)
                    });
                    let holds_an_address = value.points_to[local].may_be_a_locals_address();
                    // As of this load, and before the borrow below. See
                    // `Held::named_at_read`.
                    let named_now = value.locals_named_by_unknown_code();
                    let handed_contents = value.handed_at_most[local].below(1);
                    let held = &mut value.points_to[local];
                    held.believe(believed);
                    // **And what its memory holds, no more than it was
                    // handed as at every level below** (#636), bounding what
                    // it held, since a callee that writes nothing leaves
                    // that. See `Known::handed_at_most`.
                    held.bound_contents(&handed_contents);
                    for &site in &stored {
                        if !(holds_an_address && deep[site]) {
                            held.hold(site, At::Unknown);
                        }
                    }
                    if !stored.is_empty() {
                        held.read_out_of_memory();
                        for &named in &named_now {
                            held.named_at_read.set(named);
                        }
                    }
                    if hands_a_lost_value {
                        held.read_where_a_lost_value_may_be();
                    }
                }
                for &container in &stored {
                    for &site in &stored {
                        value.contents[container].name_site(site);
                    }
                }
                // **What it wrote there it handed over**, as a callee's
                // result or what a parameter points at is, so the next call
                // this check cannot read may free it: `acquire(&r);
                // release(); return *r;` was believed (#394). Nothing is
                // said until such a call, so `get(&p); *p = 1;`, the
                // output-parameter idiom ADR-0017 declines to report, is
                // not. See ADR-0040.
                for (held, &escaped) in value.points_to.iter_mut().zip(&value.escaped) {
                    if escaped {
                        held.handed_over();
                    }
                }

                // **And what it cannot replace keeps what it may have freed.**
                // The other half of the line above: a local whose address the
                // call cannot reach still holds what the call reached through
                // another one, `b = a; release_ref(&b);`, and is asked about
                // it at the next call by address. Only a local read after
                // this call counts, and after a call that does not return
                // nothing is (ADR-0051). What the call was handed comes
                // first, since a holder it was handed is one it may replace.
                // See ADR-0047.
                value.handed_to_a_call(handed);
                value.held_out_of_reach(|local| {
                    then.is_some_and(|then| self.live_in[then.index()][local.index()])
                });
            }
        }

        let Some(place) = destination else {
            return;
        };
        let Some(at) = Strong::assigned(place) else {
            // `*box = f();` writes an allocation this does not place. See
            // `Known::written`. **No C program reaches this**, measured: the
            // lowering calls into a temporary and stores that, which the
            // store transfer clears for. It is here for a frontend that
            // does not.
            if derefs(place) > 0 {
                value.may_write_all_sites();
            }
            // And an escaped local it may land in, as an unplaced store
            // does. See `Known::written`.
            replaced_by(self.unit, function, place, value);
            return;
        };
        // The call's result writes the local it lands in. See
        // `Known::written`.
        value.written_local(at.into());

        match kind {
            Callee::Ends => {
                unreachable!("`Allocations::call` returns for it where `handed` is decided")
            }
            // `free` returns nothing, and the C frontend gives it no
            // destination. A frontend that gives it one gives it a `void`
            // local, which holds none of this.
            Callee::Frees => {
                value.replace(at, Held::none(value.points_to.len()));
                return;
            }
            // What it returns is its first argument, unmoved: C17 7.24.2 to
            // 7.24.6 say so of each. See ADR-0039.
            Callee::ReturnsFirst | Callee::Copies => {
                let first = match arguments.first() {
                    // **Taken as the parameter it was handed to**, which is
                    // where a pointer to a row becomes a `void *`:
                    // `memcpy(&a[0], b, 4)` returns a pointer that walks
                    // `a[0]`'s row. Found by review (#947). See
                    // `Self::converted`.
                    Some(Operand::Copy(source)) if source.projection.is_empty() => {
                        let mut held = value.points_to[source.local].clone();
                        if let Some(parameter) = self.unit.function(*callee).parameters().next() {
                            self.converted(
                                &mut held,
                                function.local(source.local),
                                self.unit.function(*callee).local(parameter),
                                &value.extent,
                            );
                        }
                        held
                    }
                    // `memset(*tab, 0, 4)` returns a pointer read out of
                    // memory, and says so, as an assignment of `*tab` would.
                    // **Nothing holds this**: the call has just exposed what
                    // `*tab` reaches, so a later call handed the result finds
                    // it exposed already. It is here so that the day this
                    // family stops exposing what it is handed, its result is
                    // not a silence. See ADR-0040.
                    Some(Operand::Copy(source)) => self.read_through(function, source, value),
                    Some(Operand::Constant(_)) | None => Held::none(value.points_to.len()),
                };
                value.replace(at, first);
                return;
            }
            Callee::Allocates | Callee::Reallocates | Callee::Opaque => {}
        }

        // A call leaves behind something this function did not have before, and
        // the local it landed in is what names it. **Live rather than joined
        // with what was there**: a second turn of a loop through the same call
        // is a second allocation, and carrying the first one's `Freed` across
        // would report a double free for code that allocates each time round.
        let result = LocalIdx::of(place.local);
        let site = Site::of(result);
        // Read before the rebirth clears it: a loop through this call writes
        // this site every turn, and what the call returns may be last turn's
        // allocation, exposed, which the site number cannot tell apart.
        let was_exposed = value.exposed[site];
        value.replace(at, Held::none(value.points_to.len()));
        // At its start: the site is whatever the call handed back, so the
        // value is that value and not an offset into it.
        value.points_to[result].hold(site, At::Bytes(0));
        // **The span only where this check saw an allocation.** Every call's
        // destination is a site, because a call this cannot read may hand back
        // anything and a site is how that is tracked. But `allocated here` is a
        // claim, and `void *p = bar();` gives no evidence that `bar` allocated
        // anything. Naming that line was a caret asserting something nothing
        // had established, so a site whose call is not an allocation function
        // this check reads by name, `malloc`, `calloc`, `aligned_alloc` or
        // `realloc`, is `Live(None)` and the diagnostic leaves the label off.
        let made = match kind {
            Callee::Allocates | Callee::Reallocates => Some(origin.span()),
            Callee::Opaque | Callee::Frees | Callee::ReturnsFirst | Callee::Copies => None,
            Callee::Ends => {
                unreachable!("`Allocations::call` returns for it where `handed` is decided")
            }
        };
        value.reborn(site, made);
        // **And may be null**, an allocation's result as much as an opaque
        // call's, unless its callee promises otherwise (#931).
        value.points_to[result].may_be_null = self.unit.function(*callee).promised().is_none();
        value.extent[site] = Some(self.made_extent(function, *callee, kind, arguments, place));
        // **What a call this check cannot read returns is believed whole**,
        // as a parameter is, and bounded by what it may return that this
        // check recorded: an allocation this function handed it is folded in
        // at its size. The `return` in its body is where the belief is asked,
        // where that body is one this check reads. See ADR-0060.
        let opaque = match kind {
            Callee::Opaque => true,
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Ends => false,
        };
        if opaque {
            let believed = self.pointee_size(function, place);
            value.points_to[result].believe(believed);
            // And what its memory holds as its type says, as a parameter's.
            let contents = self
                .unit
                .place_ty(function, place)
                .and_then(|ty| self.type_levels(ty))
                .unwrap_or_default();
            value.points_to[result].set_contents(&contents);
            value.points_to[result].arrived_holds_pointer = self
                .unit
                .place_ty(function, place)
                .is_some_and(|ty| self.first_level_holds_a_pointer(ty));
        }
        // **Its contents are indeterminate** where its row says so, and for
        // `realloc` where it was handed a pointer that is certainly null, as
        // `malloc` for the size (C17 7.22.3.5 p3), a constant `0` or a local
        // that names nothing, or one that names only allocations nothing has
        // written, whose indeterminate bytes it copies (p2). The row says it
        // rather than a name read here, which had read `realloc(NULL, n)` as
        // copying an old object (#689). See `Leaves`.
        //
        // A nonzero constant, or a place reached through a dereference, is
        // read as the old object: neither is null in a defined program, and
        // the null a dereference could read is undefined to read already.
        let fresh = match self.library(*callee).and_then(|row| row.leaves) {
            Some(Leaves::Indeterminate) => true,
            Some(Leaves::TheOld) => match arguments.first() {
                Some(Operand::Constant(0)) => true,
                Some(Operand::Copy(handed)) => {
                    handed.projection.is_empty()
                        && (value.names_nothing(handed.local)
                            || value.names_only_fresh(handed.local, None))
                }
                Some(Operand::Constant(_)) | None => false,
            },
            Some(Leaves::ZeroBits) | None => false,
        };
        // **And the result names the call that made it**, so that a branch on
        // it can say the block does not exist on its null edge. See
        // `branch_on_an_allocations_result` and ADR-0080.
        if fresh {
            value.points_to[result].returned_by = Some(site);
            // **Per element where it is two to `ELEMENTS_KEPT` pointers of a
            // constant size**, and the result at the first: `b[0] = x;`
            // writes one and leaves the rest (#873). See `Known::written` and
            // ADR-0081.
            let target = self.unit.target();
            let pointer = u64::from(target.pointer_bits() / target.char().bits());
            if let Some(Extent::Exactly(bytes)) = value.extent[site] {
                let count = bytes / pointer;
                if bytes % pointer == 0 && (2..=ELEMENTS_KEPT).contains(&count) {
                    let len = usize::try_from(count).unwrap_or(0);
                    value.written[site] = PlaceInit::slots_of(Inits::unwritten(len));
                    value.points_to[result].site_element = Some(0);
                }
            }
        }
        // **And says it carried them where it copied an old block's**, so
        // that a read of them names this call and not an allocation's clause
        // (#752). Not where it was handed nothing to copy, and so made fresh
        // memory as `malloc` does.
        let carried_from = |handed: &Operand, value: &Known| match handed {
            Operand::Copy(handed)
                if handed.projection.is_empty() && !value.names_nothing(handed.local) =>
            {
                Some(Carried {
                    at: origin.span(),
                    from: Some(handed.local),
                    by: Carrier::Realloc,
                })
            }
            Operand::Copy(_) | Operand::Constant(_) => None,
        };
        let carried = match (
            self.library(*callee).and_then(|row| row.leaves),
            arguments.first(),
        ) {
            (Some(Leaves::TheOld), Some(handed)) => carried_from(handed, value),
            (Some(Leaves::TheOld | Leaves::Indeterminate | Leaves::ZeroBits) | None, _) => None,
        };
        if fresh {
            value.written[site].set(Init::Unwritten);
            if let Some(carried) = carried {
                value.written[site].carried_by(carried);
            }
        } else if let (Some(Leaves::TheOld), Some(Operand::Copy(handed))) = (
            self.library(*callee).and_then(|row| row.leaves),
            arguments.first(),
        ) {
            // **And may be indeterminate where the old block may have been**,
            // fresh or maybe so beside what else it names: it copies those
            // bytes as well (#724). See `Known::written`. **Or where what it was
            // handed may be null**, on a path where it copies nothing and
            // returns fresh memory (C17 7.22.3.5 p3): a join unions what the
            // paths name, so the null path names nothing to tell it by, and
            // `Held::may_be_null` says it (#708, ADR-0093 rule 5).
            if handed.projection.is_empty()
                && (value.may_name_fresh(handed.local, None)
                    || value.points_to[handed.local].may_be_null)
            {
                value.written[site].doubt();
                // Carried only where the old block may be unwritten itself:
                // a doubt the null path alone raises is of fresh memory, which
                // nothing was carried into.
                if let Some(carried) = carried.filter(|_| value.may_name_fresh(handed.local, None))
                {
                    value.written[site].carried_by(carried);
                }
            }
            self.past_the_old_size(function, value, site, result, handed, carried);
        }
        // **`realloc`'s new object holds what the old one held**, C17 7.22.3.5
        // p2, so what a pointer stored in the old one may hold, the new one
        // may. Reborn with nothing in it, a table grown by `realloc` and then
        // handed to a call exposed nothing it held. See ADR-0039.
        match kind {
            Callee::Ends => {
                unreachable!("`Allocations::call` returns for it where `handed` is decided")
            }
            Callee::Reallocates => {
                // **The `realloc` door's answer for every field, never
                // `..`**, so a field added to `Known` answers here for what
                // a `realloc` does with it: what the new block takes from the
                // old. The new block is a site made again first, which
                // `Known::reborn` answers for. See ADR-0091.
                let Known {
                    // The result names the new block, and below it the bound
                    // it was handed as, here.
                    points_to: _,
                    // The old block may be freed, at the call
                    // (`SiteState::may_be_freed`); the new one is live, by
                    // `Known::reborn`.
                    state: _,
                    // Not touched by `realloc`.
                    escaped: _,
                    handed_away: _,
                    // The new block's is cleared by `Known::reborn`, and
                    // nothing is carried from the old.
                    exposed: _,
                    // The new block holds what the old held, by `Known::pour`
                    // here (C17 7.22.3.5 p2).
                    contents: _,
                    // What this call remembers of the old block, here
                    // (ADR-0039).
                    realloced: _,
                    // The new block is fresh, or doubted where the old one
                    // may not have been written (#724) or it may have been
                    // handed a null (#708), above; kept per slot where it
                    // grew to a size that allows it, its slots past the old
                    // size unwritten (`Self::past_the_old_size`).
                    written: _,
                    // Not touched by `realloc`'s own arms.
                    pending: _,
                    calls: _,
                    exposed_after_call: _,
                    // The new block's size, where its site is made.
                    extent: _,
                    // A local's; not touched by `realloc`.
                    handed_at_most: _,
                    exposed_in_part: _,
                    // Through `Known::pour`.
                    stored_in_part: _,
                    // Not carried from the old block: the site made again
                    // answers for each in `Known::reborn`.
                    stored_out: _,
                    left_at: _,
                    held_levels: _,
                    // Carried from the old block, by
                    // `Known::carry_part_written` (ADR-0087).
                    part_written: _,
                    // Not touched by `realloc`.
                    handed_untyped: _,
                } = &*value;
                let old: Vec<Site> = handed
                    .iter()
                    .filter_map(|argument| match argument {
                        Operand::Copy(source) if source.projection.is_empty() => Some(source.local),
                        Operand::Copy(_) | Operand::Constant(_) => None,
                    })
                    .flat_map(|local| value.sites_of(local).collect::<Vec<_>>())
                    .filter(|&old| old != site)
                    .collect();
                // **And the bound on what it holds**, from the pointer it was
                // handed as from the old allocation below: a `void **`
                // parameter grown by `realloc` and read through an `int **`
                // was believed four bytes wide (#632).
                for argument in handed {
                    if let Operand::Copy(source) = argument {
                        if source.projection.is_empty() {
                            let handed = value.points_to[source.local].contents_at_most();
                            value.contents[site].bound_stored(&handed);
                        }
                    }
                }
                // **What the old allocation held, the new one holds after the
                // copy**: its record poured whole, the marks with the rows,
                // since what the old allocation may hold of a reborn site is
                // what the new one holds (ADR-0045), the caller's mark for the
                // same reason (ADR-0040), and whether a lost value was stored
                // in it, or a free of one after `box = realloc(box, 16);`
                // finds nothing in the new allocation to mark, found by
                // review (a cell's `lost`). See `Known::pour`.
                for old in old {
                    // **And whether a copy wrote part of a pointer into
                    // it**, the bytes it moves: `n = realloc(m, 8); void
                    // **q = n; *pp = q;` built silent (#905). See ADR-0088.
                    value.carry_part_written(self.sources, &[Loc::Heap(old)], &[Loc::Heap(site)]);
                    let held = value.contents[old].clone();
                    value.pour(&[site], &held, &[]);
                }
                if let Some(old) = realloced {
                    value.realloced[site] = Some(Realloced {
                        at: origin.span(),
                        old,
                    });
                    value.points_to[result].returned_by = Some(site);
                }
            }
            Callee::Frees
            | Callee::Allocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Opaque => {}
        }
        // **Or any allocation code this check cannot read may reach**, which a
        // call it cannot read may hand back: `stash(p); q = fetch();` may make
        // `q` be `p`. At an offset nobody said, since what comes back may point
        // into one. These are the allocations the call has just unproven, so
        // the result is as doubtful as what it may be and never less, which is
        // what a widened set has to be, since widening one can make this check
        // say less. See ADR-0039.
        match kind {
            Callee::Ends => {
                unreachable!("`Allocations::call` returns for it where `handed` is decided")
            }
            Callee::Opaque => {
                // **What it returned may hold a pointer the callee kept**, the
                // same fact as a parameter's memory from the other side of a
                // call, so the next free of the caller's or call this check
                // cannot read stales it. Here rather than where the function
                // starts, because the call that returns it cannot have freed
                // through it: marked from the start, this very call staled its
                // own result. **In a loop it does anyway**, from the second
                // turn: the site names last turn's result as well, the mark
                // from then is still on it, and `reborn` keeps `stale_read`
                // because a site's allocations are not told apart. So a load
                // out of each turn's result is doubted, a false report the
                // reader can see. See ADR-0040.
                value.contents[site].mark_from_caller();
                // **And it may be the lost value it was handed**, `id(*box)`
                // returning `u` (#508), **or one an earlier call was handed**,
                // as it holds every exposed site: `stash(u); r = fetch();`
                // (#511). The trigger above has already set the fact if this
                // call was handed one. **Lost at once**, as a named value the
                // call may have freed is doubted at once. A mark that waited
                // for a later free of a lost value, `Held::lost_read`, left
                // `stash(u); free(u); r = fetch(); *r` and `release_all()` in
                // place of the free read in silence, since nothing after the
                // call made it lost. Found by review. See ADR-0045.
                if value.exposed[Target::Unknown(Why::Lost)] {
                    value.points_to[result].lost_its_allocation();
                }
                let exposed: Vec<Site> = value
                    .exposed
                    .sites()
                    .filter(|&other| other != site && value.exposed[other])
                    .collect();
                for other in exposed {
                    value.points_to[result].hold(other, At::Unknown);
                }
                // **And any escaped local's address**, for the same reason
                // and with the same reach: `id(&x)` may return `&x`, and
                // `stash(&x); fetch()` may too. `Held::locals` is how a free
                // learns it, through `Held::may_be_a_locals_address`, and how
                // a write through the result is followed into the locals it
                // may land in. See ADR-0039.
                for local in value
                    .escaped
                    .indices()
                    .filter(|&local| value.escaped[local])
                {
                    value.points_to[result].name_local(local);
                }
                if was_exposed {
                    value.state[site] = SiteState::Unknown;
                }
                // **And what it returns is exposed**: the callee had the pointer,
                // and may have kept it where the next call can reach it.
                value.expose(self.sources, [site], Some(origin.span()));
            }
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies => {}
        }
        // After the state, because this is what takes it away again. No C
        // reaches here with an escaped destination: the lowering writes every
        // call that returns something into a fresh temporary and copies it
        // out, and one that returns `void` nowhere, so the copy above is
        // what a C program goes through. Another frontend need not, and
        // `a_call_into_a_local_whose_address_escaped` builds the shape by hand.
        value.unproved(result);
    }

    /// How many bytes the object a call's result names takes, as far as this
    /// check can read it.
    ///
    /// **Exactly the size an allocation function was asked for, where it was
    /// asked with constants**: `malloc(n)` and `aligned_alloc(a, n)` make `n`
    /// bytes (C17 7.22.3.4 p2, 7.22.3.1 p2), `calloc(m, n)` makes `m * n`
    /// (7.22.3.2 p2), and `realloc(p, n)` makes `n` (7.22.3.5 p2). A size
    /// this check cannot read, a negative constant among them, which converts
    /// to a `size_t` nothing could allocate, is nothing read. Any other call's
    /// result is believed to point at one whole object of the type it
    /// returns, as a parameter is, and a `void *` at nothing. See ADR-0060.
    fn made_extent(
        &self,
        function: &Function,
        callee: FuncId,
        kind: Callee,
        handed: &[Operand],
        destination: &Place,
    ) -> Extent {
        let constant = |index: usize| self.constant_argument(callee, handed, index);
        match kind {
            Callee::Opaque => {
                return Extent::AtLeast(self.pointee_size(function, destination).unwrap_or(0));
            }
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Ends => {}
        }
        let asked = match self.library(callee).and_then(|row| row.makes) {
            Some(Makes::Argument(index)) => constant(index),
            Some(Makes::Product(m, n)) => constant(m)
                .zip(constant(n))
                .and_then(|(m, n)| m.checked_mul(n)),
            None => None,
        };
        asked.map_or(Extent::AtLeast(0), Extent::Exactly)
    }

    /// Bound what a call this check cannot read may write into `local`,
    /// reached on the walk a departure takes as `read_as`, at its own level
    /// and below, and record it where that type is untyped somewhere below,
    /// so a later call walks there again ([`Known::handed_untyped`]). One
    /// place for the call's own arguments and for what an earlier call kept
    /// (#679), so the two cannot read a local differently.
    fn handed(&self, value: &mut Known, local: LocalId, read_as: Option<TyId>) {
        // Reached as a pointer, the body stores one of that type; reached as
        // `void`, through a `void *`, it may store any pointer, so nothing is
        // believed.
        //
        // **Reached as bytes, a local is `void`'s case** (#675): `memcpy` puts
        // any pointer in bytes, and is asked nothing of their type, so
        // `put(pv)` with `put` taking `char (*)[8]` left `v` believing its
        // declared type at every level, and a one-byte allocation was handed on
        // in silence. Whatever the local is declared as: a `char` buffer filled
        // so and read back as a pointer through a `void *` was silent too,
        // where a `void *` callee was doubted. Found by review. See ADR-0060.
        //
        // **And reached below a type that stops being a pointer, the same**
        // (#677): a callee handed `&v` as bytes or through a `void *` may read
        // `&w` back out of `v` and store any pointer into `w`, so `use1(w)`
        // handed a one-byte allocation on in silence while `use1(*v)` was
        // doubted. The walk does not say whether the type was reached through a
        // typed pointer, so one reached below memory a typed callee holds an
        // `int` in, which a may-set says may hold a local's address, is doubted
        // too, and so is one a later departure reaches only through such a
        // may-set, named in a report of its own, by the user's decision. See
        // ADR-0060.
        let Some(read_as) = read_as else {
            remember(&mut value.handed_untyped[local], None);
            value.handed_at_most[local].bound_by(&Levels::new(vec![
                Width::AtMost(0);
                self.deepest_pointer + 1
            ]));
            return;
        };
        // Recorded, so a later call walks below it again. See
        // `Known::handed_untyped`.
        if self.untyped_below(read_as) {
            remember(&mut value.handed_untyped[local], Some(read_as));
        }
        let (width, levels) = self.handed_bounds(Some(read_as));
        value.handed_at_most[local]
            .bound_by(&levels.stored_under(Width::AtMost(width), usize::MAX));
    }

    /// What a call this check cannot read may store in memory it reached as
    /// `read_as`: how wide the pointer it stores there points, and what that
    /// pointer's own memory holds, level by level. `0` at every level where
    /// it was reached as `void` or as bytes, `None`, since such a callee may
    /// store any pointer. One answer for a local ([`Self::handed`]) and for a
    /// site ([`Self::handed_site`]), so the two cannot read a type
    /// differently.
    ///
    /// No case reaches the `None` arm through a site, nor tells the levels
    /// below a site's own from nothing: only the width a site is handed at is
    /// held, by `a_pointer_read_out_of_memory_a_call_was_handed_is_not_reported_as_indeterminate`.
    fn handed_bounds(&self, read_as: Option<TyId>) -> (u64, Levels) {
        let nothing = || Levels::new(vec![Width::AtMost(0); self.deepest_pointer]);
        let Some(read_as) = read_as else {
            return (0, nothing());
        };
        // An array of pointers holds pointers, as `type_levels` reads it.
        let width = match self.unit.ty(innermost_element(self.unit, read_as)) {
            Ty::Pointer(pointee) => self.unit.size_of(pointee).unwrap_or(0),
            Ty::Void | Ty::Int | Ty::Char => 0,
            Ty::Array { .. } => {
                unreachable!("`innermost_element` strips every array")
            }
        };
        // **And below it, at every level**, as the type it was reached as says:
        // through a `void *`, nothing at any level. See
        // `Known::handed_at_most`. Through an array to its element, as
        // the width above reads it: `fill(&a)` with `a` an `int **[2]` took `a`
        // as reached through a `void *`. Found by review.
        let levels = self
            .type_levels(innermost_element(self.unit, read_as))
            .unwrap_or_else(nothing);
        (width, levels)
    }

    /// Record memory a call this check cannot read reached as `read_as`,
    /// where that type is untyped somewhere below. See
    /// [`Known::handed_untyped`].
    ///
    /// **And bound what it may store there**, as [`Self::handed`] bounds a
    /// local: what the callee stores is believed as it was handed, and a
    /// site records nothing else, so `init(box); int *t = *box;` with `init`
    /// taking `int **` believed nothing once a load believes nothing that no
    /// width was recorded for (ADR-0061).
    fn handed_site(&self, value: &mut Known, site: Site, read_as: Option<TyId>) {
        if read_as.is_none_or(|ty| self.untyped_below(ty)) {
            remember(&mut value.handed_untyped[site], read_as);
        }
        let (width, levels) = self.handed_bounds(read_as);
        value.contents[site]
            .bound_stored(&levels.stored_under(Width::AtMost(width), self.deepest_pointer));
    }

    /// Whether a callee handed a pointer to `ty` may store any pointer
    /// somewhere below it: `ty` holds no pointer, or is a pointer whose chain
    /// ends in `void` or `char`. Not one that ends in `int`, which a
    /// well-typed `int **` out-parameter does. See [`Known::handed_untyped`].
    ///
    /// Written out rather than as a `matches!`, so a kind of type added later
    /// is asked what it is.
    fn untyped_below(&self, ty: TyId) -> bool {
        match self.unit.ty(innermost_element(self.unit, ty)) {
            Ty::Pointer(pointee) => {
                let mut below = innermost_element(self.unit, pointee);
                while let Ty::Pointer(next) = self.unit.ty(below) {
                    below = innermost_element(self.unit, next);
                }
                match self.unit.ty(below) {
                    Ty::Void | Ty::Char => true,
                    Ty::Int => false,
                    Ty::Pointer(_) | Ty::Array { .. } => {
                        unreachable!("the loop strips every pointer and array")
                    }
                }
            }
            Ty::Void | Ty::Int | Ty::Char => true,
            Ty::Array { .. } => unreachable!("`innermost_element` strips every array"),
        }
    }

    /// Whether a free or `realloc` handed these arguments may free what the
    /// caller owns: an argument holding an exposed parameter's site, or read
    /// out of what one points at, or a place of dereferences whose load reads
    /// caller memory, `free(*pp)`. Not an allocation this function made,
    /// which the caller cannot have handed it. See ADR-0040.
    fn frees_callers(&self, handed: &[Operand], value: &Known) -> bool {
        handed.iter().any(|argument| {
            let Operand::Copy(place) = argument else {
                return false;
            };
            if place.projection.is_empty() {
                let held = &value.points_to[place.local];
                held.beyond().is_from_caller()
                    || self
                        .exposed_parameters
                        .iter()
                        .any(|&parameter| held.has_site(Site::of(LocalIdx::of(parameter))))
            } else {
                let depth = derefs(place);
                depth > 0 && self.reads_caller_memory(place.local, depth, value)
            }
        })
    }

    /// Whether this call is handed a local that may hold an allocation this
    /// check cannot name.
    ///
    /// **A second question about the same arguments**, and it cannot be folded
    /// into [`Allocations::touching`]: that answers [`Reached::Lost`](super::parts::Reached::Lost) for an
    /// escaped local as well, which is ADR-0017, and the two facts are not
    /// interchangeable here. Freeing an escaped local that no call has run past
    /// still frees what this check thinks it holds, and reading the folded
    /// answer instead would give up the proof
    /// `a_free_through_an_escaped_local_is_seen_by_a_sharer` holds. One rule
    /// written in two places drifts apart, so the argument walk is
    /// spelled the way its twin above spells it and the reason there are two is
    /// written here. See ADR-0029.
    ///
    /// **Neither arm is observable, and both are here anyway.** `free` takes
    /// one argument, so the argument this walk skips is the only argument
    /// there is, and the branch that reads this then writes on nothing:
    /// answering `true` for a constant, and dropping the projection test, each
    /// leave the whole workspace green, measured. The `reached.len() > 1`
    /// branch says the same thing about the same premise. What they would cost the day something
    /// reaches them is a free refusing to prove because of a row belonging to
    /// a pointer rather than to what it points at. The arm above them is the
    /// one that decides anything.
    fn holds_something_unnameable(arguments: &[Operand], known: &Known) -> bool {
        arguments.iter().any(|argument| match argument {
            // A constant holds no allocation, for the reason `touching` gives.
            Operand::Constant(_) => false,
            // A projection names a place rather than a local, and this check
            // follows locals: `free(*pp)` is already a `Reached::Lost` above,
            // and there is no row here to ask.
            // A load holds what was stored where it was read from, which may
            // not be all it holds, so a free of it does not say which went
            // either. See ADR-0045.
            //
            // A pointer that may be a local's address may have freed none of
            // its sites, for the reason `touching` gives. Asked here too, or
            // a later free of one of those sites is proved a double free on
            // the path that took `&x`, where nothing freed it.
            Operand::Copy(place) => {
                let held = &known.points_to[place.local];
                place.projection.is_empty()
                    && (held.beyond().any_unknown() || held.may_be_a_locals_address())
            }
        })
    }

    /// **What `realloc` leaves past the old block's size**: those bytes are
    /// indeterminate (C17 7.22.3.5 p2), and the new block is otherwise what
    /// the old one held. Believed written whole, `p = malloc(8); *p = x; q =
    /// realloc(p, 16); r = q[1];` read `q[1]` in silence (#918's review).
    ///
    /// **How large the old block was** is the least any block it may have
    /// been handed is known or believed to be: an allocation's constant
    /// size, exactly; a parameter's or a call's result, at least its type
    /// (ADR-0060); and with no block recorded, what the pointer handed is
    /// believed to point at. Exact sizes leave every slot past the old size
    /// unwritten; a lower bound leaves them possibly so, since the old block
    /// may have been larger.
    ///
    /// **Per slot where the new size allows**, 2 to `ELEMENTS_KEPT` pointers:
    /// the slots below the old size keep what the old block's record says,
    /// so `*q` of a block written before it grew is not doubted, and one the
    /// old size ends inside may be either. **Doubted whole** anywhere else a
    /// grown block may be, a new size this check cannot read among them. Not
    /// touched where the new block is shown no larger than the old.
    fn past_the_old_size(
        &self,
        function: &Function,
        value: &mut Known,
        site: Site,
        result: LocalIdx,
        handed: &Place,
        carried: Option<Carried>,
    ) {
        let target = self.unit.target();
        let pointer = u64::from(target.pointer_bits() / target.char().bits());
        let old: Vec<Site> = if handed.projection.is_empty() {
            value.sites_of(handed.local).collect()
        } else {
            Vec::new()
        };
        // The least the old block was, and whether that is its size exactly.
        let (least, exact) = if old.is_empty() {
            let believed = value.points_to[handed.local]
                .believed()
                .map(|width| {
                    if width == u64::MAX {
                        self.pointee_size(function, handed).unwrap_or(0)
                    } else {
                        width
                    }
                })
                .unwrap_or(0);
            (believed, false)
        } else {
            old.iter().fold((u64::MAX, true), |(least, exact), &old| {
                match value.extent[Loc::Heap(old)] {
                    Some(Extent::Exactly(size)) => (least.min(size), exact),
                    Some(Extent::AtLeast(size)) => (least.min(size), false),
                    None => (0, false),
                }
            })
        };
        let new = match value.extent[Loc::Heap(site)] {
            Some(Extent::Exactly(size)) => Some(size),
            Some(Extent::AtLeast(_)) | None => None,
        };
        if new.is_some_and(|new| new <= least) {
            return;
        }
        let slots = new
            .filter(|new| new % pointer == 0)
            .map(|new| new / pointer)
            .filter(|count| (2..=ELEMENTS_KEPT).contains(count))
            .and_then(|count| usize::try_from(count).ok());
        let Some(len) = slots.filter(|_| !value.points_to[handed.local].may_be_null) else {
            value.written[site].doubt();
            return;
        };
        // What the slots below the old size hold: the one old block's
        // record, or what the new block already says where there are
        // several or none.
        let before = match old.as_slice() {
            [old] => value.written[*old].clone(),
            _ => value.written[site].clone(),
        };
        let past = if exact { Init::Unwritten } else { Init::Maybe };
        let whole_slots = usize::try_from(least / pointer).unwrap_or(usize::MAX);
        let mut inits = Inits::spread(past, len);
        for slot in 0..len.min(whole_slots) {
            inits.set(slot, before.at(u64::try_from(slot).ok()));
        }
        if whole_slots < len && least % pointer != 0 {
            inits.set(whole_slots, Init::Maybe);
        }
        value.written[site] = PlaceInit::slots_of(inits);
        // **Where its bytes came from is this call**, below the old size
        // and past it alike, which the note about a `realloc` says of both
        // (C17 7.22.3.5 p2).
        if let Some(carried) = carried {
            value.written[site].carried_by(carried);
        }
        value.points_to[result].site_element = Some(0);
    }

    /// How many levels of the blocks a library copy carries out the type
    /// `side_ty` of one side of it, its source or its destination, reads as
    /// holding pointers: what is copied is a value one level below it, and
    /// the blocks are what that value points at, asked as a store asks the
    /// type it stores into ([`Self::levels_holding_pointers`]). `int **b;
    /// memcpy(out, &b, 8)` copies an `int **`, whose blocks hold pointers;
    /// `int *x; memcpy(pp, &x, 8)` copies an `int *`, whose block holds
    /// `int`s.
    ///
    /// **The copy is asked where either side says so.** Each type is a
    /// claim nothing checks once a `void *` stands between: with `out` an
    /// `int ***`, `void *v = &b; int **w = v; memcpy(out, w, 8);` copies
    /// `b` through a source that reads it as an `int *`, while the
    /// destination says what the caller reads, as a store's does. Found by
    /// review.
    ///
    /// **A side whose type says nothing reads every level as holding them**,
    /// `usize::MAX`: one
    /// that copies a value not typed as a pointer, a `void *`, `char *` or
    /// `int *` side. `memcpy` copies bytes, and the usual source is a
    /// `void *`, so leaving those unasked would leave the common spelling of
    /// #890 silent. Nor does an `int *` source prove it copies an `int`:
    /// `void *v = &b; int *i = v; memcpy(out, i, 8);` copies `b` through it,
    /// and C defines that, since `memcpy` reads bytes. Found by review.
    /// Where the source truly holds no pointer, the copy carries no block
    /// and asking costs nothing; what it costs elsewhere is a doubt the
    /// reader sees, at a block of `int`s copied out through a `void *`
    /// source.
    ///
    /// An untyped side, and one not typed as a pointer at all, are read the
    /// same way. No program this frontend reads reaches either, and
    /// nothing guards them.
    fn copy_levels_holding_pointers(&self, side_ty: Option<TyId>) -> usize {
        let Some(side_ty) = side_ty else {
            return usize::MAX;
        };
        let copied = match self.unit.ty(innermost_element(self.unit, side_ty)) {
            Ty::Pointer(copied) => innermost_element(self.unit, copied),
            Ty::Int | Ty::Char | Ty::Void => return usize::MAX,
            Ty::Array { .. } => unreachable!("`innermost_element` strips every array"),
        };
        match self.unit.ty(copied) {
            Ty::Pointer(_) => self.levels_holding_pointers(copied),
            Ty::Int | Ty::Char | Ty::Void => usize::MAX,
            Ty::Array { .. } => unreachable!("`innermost_element` strips every array"),
        }
    }
}

/// Add `ty` to the types `kept` records, once. See `Known::handed_untyped`.
fn remember(kept: &mut Vec<Option<TyId>>, ty: Option<TyId>) {
    if !kept.contains(&ty) {
        kept.push(ty);
    }
}
