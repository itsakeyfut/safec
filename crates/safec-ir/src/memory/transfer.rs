//! The analysis, [`Allocations`], and its transfer functions: what each element,
//! terminator and edge of a function does to the [`Known`] value, and what the
//! analysis asks of the IR to answer that.

use std::collections::{BTreeMap, BTreeSet};

use crate::dataflow::Analysis;
use crate::ir::{
    BlockId, Element, FuncId, Function, LocalId, Operand, Place, Projection, Rvalue, Terminator,
    TranslationUnit, Ty,
};
use crate::source::{SourceMap, Span};

use super::built::{built_from, named, replaced_by};
use super::known::Known;
use super::parts::{
    Callee, Freeing, Held, Offset, PendingRead, Reached, Read, Realloced, SiteState,
};
use super::{dereferenced_in_element, dereferenced_in_terminator, derefs, handed_places};

/// The analysis: where an allocation is, and whether it has been freed.
pub(super) struct Allocations<'a> {
    pub(super) sources: &'a SourceMap,
    pub(super) unit: &'a TranslationUnit,
    /// The function this is the analysis of, which every finding names.
    pub(super) function: FuncId,
    /// How many locals the function has, which is how many sites there can be.
    pub(super) locals: usize,
    /// The locals a caller filled, which are sites because an allocation can
    /// arrive through one.
    pub(super) parameters: Vec<LocalId>,
    /// The parameters that hold a pointer, whose allocations are exposed
    /// where the function starts: the caller, which this check cannot read,
    /// had the pointer, and may have left it where a call can free it.
    ///
    /// **Not every parameter.** Every one is a site, for the reason the module
    /// comment gives, and an integer parameter can hold an allocation only
    /// through a conversion this frontend does not accept yet: a cast, which
    /// does not parse, or an implicit one, which C17 6.5.16.1 p1 forbids and
    /// #154 is about. Exposing one made every opaque call's result reach it.
    /// The day casts parse, the type below stops being enough.
    ///
    /// **Not `main`'s either.** Its caller is the host, and C17 5.1.2.2.1 p2
    /// has `argv` and its strings keep their values until the program ends:
    /// no allocation function returned them, so no call can free them
    /// without the behaviour 7.22.3.3 p2 leaves undefined. Exposing them
    /// refused `log_line(); char *name = argv[0];`. A program that calls
    /// `main` itself hands it arguments this does not see. See ADR-0040.
    pub(super) exposed_parameters: Vec<LocalId>,
    /// Per block, the locals read from its entry before they are written:
    /// what [`live_in`] answers, once per function. See ADR-0048.
    pub(super) live_in: Vec<Vec<bool>>,
}

impl Allocations<'_> {
    /// What this check can read in the name of the function being called.
    pub(super) fn callee(&self, id: FuncId) -> Callee {
        match self.sources.snippet(self.unit.function(id).name) {
            "free" => Callee::Frees,
            "malloc" | "calloc" | "aligned_alloc" => Callee::Allocates,
            "realloc" => Callee::Reallocates,
            "memcpy" | "memmove" => Callee::Copies,
            "memset" | "strcpy" | "strncpy" | "strcat" | "strncat" => Callee::ReturnsFirst,
            _ => Callee::Opaque,
        }
    }

    /// What a write through a pointer carries: the sites the written value may
    /// hold.
    ///
    /// One answer for every reader of it: the write that lands in a followed
    /// local, and what a write into memory records as inside an allocation or exposes.
    /// One rule in two places drifts.
    ///
    /// **A `match` rather than an `if let`, so a fifth kind of rvalue has to
    /// answer here too.** Every other reader of `Rvalue` in this crate is
    /// exhaustive and `error[E0004]` is what asks them; this one was the
    /// exception, and what a missed arm would mean is that a write through a
    /// pointer silently carries nothing, which is a silence rather than a build
    /// error.
    fn carried(&self, function: &Function, written_value: &Rvalue, value: &Known) -> Held {
        match written_value {
            // `*pp = q + 1;` arrives here as a copy, not as the arithmetic:
            // the lowering puts the addition in a temporary and copies it
            // out, and the direct assignment in `Allocations::element` has
            // already given that temporary `q`'s sites.
            Rvalue::Use(Operand::Copy(source)) if source.projection.is_empty() => {
                value.points_to[source.local.index()].clone()
            }
            // The arithmetic written straight into the place, which no C
            // reaches for the reason above and another frontend may. The
            // operands the types say contributed, for the reason the direct
            // assignment's `Rvalue::Binary` arm gives, and the same question
            // about the edge: this asked none of it until review built the
            // shape by hand, and carried an edge through `qq + 7` that the
            // direct assignment had just been taught to drop.
            Rvalue::Binary { op, lhs, rhs } => {
                let mut reached = built_from(
                    *op,
                    [lhs, rhs],
                    value,
                    |local| self.is_pointer(function, local),
                    |place| self.may_be_pointer(function, place),
                    |place| self.reads_a_byte(function, place),
                    |local, depth| self.reads_caller_memory(local, depth, value),
                );
                // What the arithmetic leaves of the edge, as the direct
                // assignment's `Rvalue::Binary` arm asks. See ADR-0019.
                reached.moved_by_arithmetic();
                reached
            }
            // A read through a projection is not a pointer this check follows
            // to an allocation, and the target is given no site for it; but
            // it may be a pointer read out of memory, and says so. See
            // `Allocations::read_through`.
            Rvalue::Use(Operand::Copy(source)) => self.read_through(function, source, value),
            // A constant, a unary operator, an address. None is a pointer
            // this check follows to an allocation, so the target is given
            // nothing: a write this check cannot follow is not evidence that
            // the old contents are gone.
            Rvalue::Use(Operand::Constant(_)) | Rvalue::Unary { .. } | Rvalue::Address(_) => {
                Held::none(value.points_to.len())
            }
        }
    }

    /// Whether this local holds a pointer.
    ///
    /// What tells the pointer operand of an addition from the integer beside
    /// it, which is the question [`built_from`] asks and C17 6.5.6 p8 answers.
    /// See ADR-0030.
    ///
    /// **Written out rather than as a `matches!`, because the answer for a kind
    /// nobody has added yet is not `false`.** A type this does not recognise is
    /// dropped from an addition that has a pointer beside it, and a dropped
    /// operand is a site nothing reports: `error[E0004]` here is what asks a
    /// fourth kind of type whether it is one. `E0004` makes somebody look and
    /// that is all it makes them do, and this is the case where a reader has
    /// something to decide rather than a line to fill in.
    pub(super) fn is_pointer(&self, function: &Function, local: LocalId) -> bool {
        match self.unit.ty(function.local(local)) {
            Ty::Pointer(_) => true,
            Ty::Int | Ty::Char | Ty::Void => false,
        }
    }

    /// Whether a place may hold a pointer, by its type.
    ///
    /// **`None` answers yes.** [`TranslationUnit::place_ty`] has no type for a
    /// `Deref` of something that is not a pointer, which the lowering does not
    /// build and a hand-built unit can, and a place whose type this cannot
    /// name is one it cannot narrow: [`replaced_by`] reads it the same way.
    ///
    /// **A `char` answers no**, though C17 6.5 p7 lets one copy a pointer a
    /// byte at a time and [`replaced_by`] reads it as reaching everything for
    /// that reason. The copy is followed where the byte is read, by
    /// [`Self::read_through`]; answering yes here would also read every
    /// character handed to a call as everything stored anywhere, which refused
    /// correct string code. See ADR-0046.
    pub(super) fn may_be_pointer(&self, function: &Function, place: &Place) -> bool {
        match self
            .unit
            .place_ty(function, place)
            .map(|ty| self.unit.ty(ty))
        {
            Some(Ty::Pointer(_)) | None => true,
            Some(Ty::Int | Ty::Char | Ty::Void) => false,
        }
    }

    /// Whether a place is a byte: of character type, which C17 6.5 p7 lets read
    /// any object, a pointer included.
    ///
    /// Written out rather than as a `matches!`, so a kind of type added later
    /// is asked whether it can copy a pointer too. See ADR-0046.
    pub(super) fn reads_a_byte(&self, function: &Function, place: &Place) -> bool {
        match self
            .unit
            .place_ty(function, place)
            .map(|ty| self.unit.ty(ty))
        {
            Some(Ty::Char) => true,
            Some(Ty::Pointer(_) | Ty::Int | Ty::Void) | None => false,
        }
    }

    /// What a value read through a projection holds: no site, and whether it
    /// may be a pointer, or a byte of one, read out of memory.
    ///
    /// One answer for the three places a load is given to something: an
    /// assignment, a write through a pointer, and what a library copy returns
    /// when handed one. One rule in several places drifts apart.
    /// See ADR-0040.
    fn read_through(&self, function: &Function, source: &Place, value: &Known) -> Held {
        let mut held = Held::none(value.points_to.len());
        // **A byte is a load too.** C17 6.5 p7 lets a character type read any
        // object, so a pointer copied a byte at a time is copied, and the byte
        // carries what was stored where it was read from, marked as a load is
        // so that it proves nothing about what it names. See ADR-0046.
        held.loaded = self.may_be_pointer(function, source) || self.reads_a_byte(function, source);
        // **And what was stored where it was read from**, at an offset nobody
        // said, so that a dereference of it can be asked about what it may
        // point at. `loaded` stays set, which is what marks the set as
        // possibly incomplete for every reader. See ADR-0045.
        let depth = derefs(source);
        if held.loaded && depth > 0 {
            let (sites, locals) = value.levels_below(source.local, depth);
            for site in sites {
                held.hold(site, Offset::Unknown);
            }
            // **And the locals whose address it may be**, as edges, so a
            // write through it lands there: `m = *t3; *m = r;` with `*t3 =
            // &slot`. Not all of them, since the set is a lower bound.
            // See ADR-0045.
            for target in locals {
                held.writes_to[target] = true;
                held.writes_elsewhere = true;
            }
            // Read out of an allocation that may hold one that is gone.
            // See ADR-0045.
            if value.stale_below(source.local, depth) {
                held.lost = true;
                held.stale_read = true;
            }
            // Read through a pointer to a local this check lost, carrying
            // whether what it lost may be gone, as the local does. See
            // ADR-0045.
            if value.lost_through(source.local, depth) {
                held.lost = true;
                held.stale_read |= value.stale_through(source.local, depth);
            }
            if self.reads_caller_memory(source.local, depth, value) {
                held.from_caller = true;
            }
        }
        held
    }

    /// Whether a load `depth` dereferences through `local` reads memory a
    /// pointer parameter points at: whether `local` holds the site of one, or
    /// was itself read out of such memory, or the load reads through an
    /// allocation one was stored in, at any level. The second is the same read
    /// one level further in, `q = **ppp` spelled `pp = *ppp; q = *pp;`, and
    /// without it `q` held nothing a call could make lost; the third is the
    /// same read after a store, `*box = q; r = *box;` or `r = **bb;`, which
    /// [`Known::from_caller`] marks. Only the parameters `exposed_parameters` names,
    /// so what the host hands `main` is not this, as it is not exposed. One
    /// answer for a load assigned and a load as an operand, so that the two
    /// cannot disagree. See ADR-0040.
    fn reads_caller_memory(&self, local: LocalId, depth: usize, value: &Known) -> bool {
        let held = &value.points_to[local.index()];
        // At least the local's own allocations, which a load with no
        // dereference in its place, an element, still reads through.
        held.from_caller
            || value.marked_below(local, depth.max(1), &value.from_caller)
            || self
                .exposed_parameters
                .iter()
                .any(|parameter| held.sites[parameter.index()])
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
                let held = &value.points_to[place.local.index()];
                held.from_caller
                    || self
                        .exposed_parameters
                        .iter()
                        .any(|parameter| held.sites[parameter.index()])
            } else {
                let depth = derefs(place);
                depth > 0 && self.reads_caller_memory(place.local, depth, value)
            }
        })
    }

    /// What an operand may reach beyond the sites it holds, because it may be
    /// a pointer read out of memory.
    ///
    /// Read through one `Deref` of a local holding sites, and neither a load
    /// nor a pointer it lost besides, what those allocations may contain. Read
    /// any other way, or held by a local that may hold a load or a pointer it
    /// lost, what is stored anywhere, [`Known::stored`]. **A local merely holding no site answers
    /// nothing**: `int *z = 0;` is one, and is not a load. What the report
    /// reads is not this, and is ADR-0017's. See ADR-0040.
    fn read_out(&self, function: &Function, operand: &Operand, known: &Known) -> Vec<usize> {
        let Operand::Copy(place) = operand else {
            return Vec::new();
        };
        // **A byte read out of memory handed to a call reaches what was stored
        // where it was read**, as a byte assigned first does: a callee can keep
        // the bytes of a pointer and put it back together. Never everything
        // stored anywhere, which is what refused a character handed to a call
        // in correct string code. See ADR-0046.
        let depth = derefs(place);
        if depth > 0 && self.reads_a_byte(function, place) {
            return known
                .levels_below(place.local, depth)
                .0
                .into_iter()
                .collect();
        }
        if !self.may_be_pointer(function, place) {
            return Vec::new();
        }
        let held = &known.points_to[place.local.index()];
        if place.projection.is_empty() {
            return if held.loaded || held.lost {
                known.stored()
            } else {
                Vec::new()
            };
        }
        if place.projection.as_slice() == [Projection::Deref]
            && !held.lost
            && !held.loaded
            && held.sites().next().is_some()
        {
            return held
                .sites()
                .flat_map(|container| {
                    known.inside[container]
                        .iter()
                        .enumerate()
                        .filter_map(|(site, &in_it)| in_it.then_some(site))
                })
                .collect();
        }
        known.stored()
    }

    /// Everything a call handed these arguments can reach by itself: the sites
    /// they name, what every escaped local holds, and what an argument read
    /// out of memory may be. Not closed over what those allocations hold.
    ///
    /// One function for the two arms of the transfer that expose it and for
    /// [`used_before`](super::report::used_before), which asks a read carried to the call about it, so
    /// that what a call is asked about and what it is taken to have reached
    /// cannot disagree. See ADR-0039 and ADR-0040.
    pub(super) fn reach(
        &self,
        function: &Function,
        handed: &[Operand],
        named: impl Iterator<Item = usize>,
        known: &Known,
    ) -> Vec<usize> {
        let mut reach = known.reach_of(named);
        // **And what an argument read out of memory may be.** It names no
        // site, and "no site" is what the report is told; what the callee can
        // reach through it is whatever was stored where it was read from.
        // `release(*tab);` frees what `tab` held. See ADR-0040.
        for argument in handed {
            reach.extend(self.read_out(function, argument, known));
        }
        reach
    }

    /// Whether a call to this function may have freed any allocation still
    /// live, whatever it was handed: a hatch, whose unproven conclusions are
    /// listed rather than reported. See ADR-0038.
    pub(super) fn frees_anything(&self, callee: FuncId) -> bool {
        self.unit.function(callee).hatch()
    }

    /// What the arguments of a call reach, in the order they were written.
    ///
    /// Shared with [`findings`](super::findings), so that the walk which reports and the walk which
    /// computes cannot disagree about what a call touches.
    ///
    /// **What they may disagree about is which arguments are handed here**, and
    /// exactly one thing does it: `asked` leaves out a pointer the nullability
    /// check established is null, because C says such a call does nothing, and
    /// only the walk that reports asks that. See ADR-0027, and `asked` for why
    /// the transfer is deliberately not told.
    pub(super) fn touching<'o>(
        arguments: impl IntoIterator<Item = &'o Operand>,
        known: &Known,
    ) -> Vec<Reached> {
        let mut reached = Vec::new();

        for argument in arguments {
            let place = match argument {
                // Not a pointer that went missing. `free(0)` is the case, and
                // C17 7.22.3.3 p2 makes it do nothing, so it is written on
                // purpose and is not something this check lost track of.
                //
                // **The arm is wider than the clause**, which covers the null
                // pointer and makes every other address undefined. `free(17)`
                // is skipped here too and this check says nothing about it.
                // What keeps that from mattering is not this line: C17
                // 6.5.2.2 p2 makes the call a constraint violation, so a
                // conforming implementation has to diagnose it before any
                // analysis runs, and the reason this one does not is the
                // missing assignment-constraint check that #154 is about.
                // `a_free_of_a_null_constant` pins the half the clause
                // supports; the other half is held by nobody here and is not
                // this check's to hold.
                //
                // **The same clause reaches a local**, where the constant was
                // given a name first, and that is `asked`'s rather than this
                // arm's: what it takes to recognise is a nullness the other
                // check established, which nothing here can see. See ADR-0027.
                Operand::Constant(_) => continue,
                Operand::Copy(place) => place,
            };

            if !place.projection.is_empty() {
                // `free(*pp)` frees whatever `pp` points at, and this check
                // follows locals rather than what they point at.
                reached.push(Reached::Lost);
                continue;
            }

            let before = reached.len();
            reached.extend(known.reached_by(place.local));
            // **A load is something this check cannot fully name**, whatever
            // sites it holds, so a free of it stays the doubt it was rather
            // than a free of those sites alone: answering them without this
            // made `free(q)` of a load say less. See ADR-0045.
            if reached.len() == before || known.points_to[place.local.index()].loaded {
                reached.push(Reached::Lost);
            }
        }

        reached
    }

    /// Whether this call is handed a local that may hold an allocation this
    /// check cannot name.
    ///
    /// **A second question about the same arguments**, and it cannot be folded
    /// into [`Allocations::touching`]: that answers [`Reached::Lost`] for an
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
            Operand::Copy(place) => {
                let held = &known.points_to[place.local.index()];
                place.projection.is_empty() && (held.lost || held.loaded)
            }
        })
    }

    /// The sites `arguments` name through a local that is not a load: what a
    /// call is certainly handed, rather than what it may be.
    fn named_outright(arguments: &[Operand], known: &Known) -> Vec<usize> {
        let outright = arguments.iter().filter(|argument| match argument {
            Operand::Copy(place) => !known.points_to[place.local.index()].loaded,
            Operand::Constant(_) => true,
        });
        named(&Self::touching(outright, known)).collect()
    }

    /// **A branch on what a `realloc` returned says what became of what it was
    /// handed.** On the arm where the pointer is null the call failed and the
    /// old allocations are live again; on the other it succeeded and they were
    /// freed at the call. Only where the tested pointer holds exactly the one
    /// allocation the fact is about, and is not one this check stopped
    /// following. Which local the branch tested is the nullability check's
    /// answer, shared, so the two checks read one branch alike. See ADR-0039.
    fn branch_on_reallocs_result(
        &self,
        function: &Function,
        block: BlockId,
        terminator: &Terminator,
        index: usize,
        value: &mut Known,
    ) {
        let Terminator::Branch {
            condition: Operand::Copy(condition),
            ..
        } = terminator
        else {
            return;
        };
        let Some((local, on_then)) =
            crate::nullability::tested_against_null(self.unit, function, block, condition)
        else {
            return;
        };
        let held = &value.points_to[local.index()];
        let mut sites = held.sites();
        let (Some(returned), None) = (sites.next(), sites.next()) else {
            return;
        };
        if held.lost || held.returned_by != Some(returned) {
            return;
        }
        let Some(fact) = value.realloced[returned].clone() else {
            return;
        };
        // `Terminator::successors` pushes `then` and then `otherwise`, so
        // index 1 is the other arm, as the nullability check reads it.
        let null = (index == 0) == (on_then == crate::nullability::Nullness::Null);
        for (site, made) in fact.old {
            if null {
                if matches!(value.state[site], SiteState::Unknown) {
                    value.state[site] = SiteState::Live(made);
                }
            } else {
                value.state[site] = SiteState::Freed {
                    made,
                    freed: Freeing {
                        at: fact.at,
                        sequenced: true,
                    },
                };
            }
        }
    }
}

impl Analysis for Allocations<'_> {
    /// **Two things happen on an edge, in this order.** A branch on what a
    /// `realloc` returned says what became of what it was handed,
    /// [`Self::branch_on_reallocs_result`]; and a local nothing reads past the
    /// edge holds nothing, so that where the arms meet a site nothing live
    /// still names on one of them takes its state from the other. See
    /// ADR-0048. The `realloc` branch first, because it reads the local the
    /// branch tested, which is often read nowhere after it: cleared first,
    /// three `realloc` cases lost their answer, measured. A local whose
    /// address was taken is never cleared, since [`live_in`] counts it live
    /// everywhere: `Rvalue::Address` is the only way an address is taken.
    fn edge(
        &self,
        function: &Function,
        block: BlockId,
        terminator: &Terminator,
        index: usize,
        value: &mut Self::Value,
    ) {
        self.branch_on_reallocs_result(function, block, terminator, index, value);
        let mut successors = Vec::new();
        terminator.successors(&mut successors);
        let Some(successor) = successors.get(index) else {
            return;
        };
        let live = &self.live_in[successor.index()];
        for local in function.locals() {
            if !live[local.index()] {
                value.clear(local);
            }
        }
    }

    type Value = Known;

    fn height(&self, function: &Function) -> usize {
        let locals = function.locals().len();
        // Each local's set of sites only grows, so it takes at most one step
        // per site, and a site is a local. Each site's state walks `Live` to
        // `Freed` or to `Reachable`, and on to `Unknown`, which is still two
        // steps since the join of those two is `Unknown` (ADR-0047), and a
        // third below them, since a state nothing on one side holds is beneath
        // every state the join can meet it with (ADR-0048); its
        // `freed` span can only move to an earlier
        // one, which it can do at most once per site that frees; and its `made`
        // span can only fall from `Some` to `None`, once. A local's `escaped`
        // bit goes from `false` to `true` and never back, once each, and its
        // `lost` bit costs one more of the same. Each local's set of locals a
        // write through it may reach is a second square table that only grows,
        // so it takes at most one step per pair, and whether that table is all
        // of what a write through it may reach goes from `false` to `true`
        // once per local, for one more step each. Where the set it named was
        // freed goes from `None` to `Some` once per local and a join only takes
        // it away, so it costs one more step each. Whether a free has been
        // sequenced is one bit per site: the transfer sets it and only a join
        // takes it back, which a join can do once, so it is one more step per
        // site and the number below is not changed for it. That is slack being
        // spent rather than a bound being re-derived, and the paragraph below
        // is why that is acceptable here. Where a local points in its sites
        // moves from `Zero` or `NonZero` to `Unknown` at a join and never back,
        // once per local, and the per-local term below counts that step.
        // Whether a site is exposed is one more bit per site that a join only
        // sets, so one more step each; what each site may contain is a third
        // square table that only grows at a join, one step per pair, and the
        // first term gains a third square for it. See ADR-0039. Whether a
        // local may hold a pointer read out of memory is one more bit per
        // local that a join only sets, one more step each. See ADR-0040.
        // Whether a call this check cannot read may have written into a local
        // is one more bit per local that a join only sets, one more step each.
        // See ADR-0044. Whether a site may contain an allocation that is gone
        // is one more bit per site that a join only sets, one more step each,
        // and whether a local was read out of such a site one more per local.
        // See ADR-0045. Whether a local was read out of what a parameter
        // points at is one more per local, and whether a site may hold such a
        // pointer one more per site, which a join only sets. See ADR-0040.
        // What a `realloc` remembered on a site is one more per site: set by
        // the call, and a join only takes it away; and which site a local is
        // exactly the result of is one more per local, the same. See ADR-0039.
        // Which locals' addresses each site may hold is a fourth square table
        // that only grows, one step per pair, and the first term gains a
        // fourth square for it. See ADR-0045.
        //
        // **The bit is not monotone in the transfer, and does not have to be.**
        // `Held::clear` puts it back at every fresh assignment. What this
        // number bounds is how often a *block's entry value* can move, and an
        // entry value moves only through `join`, which unions. A transfer that
        // takes facts away inside a block cannot make the entry value descend,
        // so it cannot make the solver oscillate. Generous
        // rather than tight, which is the direction `Analysis::height` says to
        // err in: answering too low stops a correct analysis.
        //
        // **What has been read since the last sequence point is a set of
        // positions, so it is counted in positions rather than in locals.** A
        // block's entry value can gain one entry per place read at one element
        // or terminator of the function, and each entry's set of sites can gain
        // one site per local, and its set of sites reachable to code this check
        // cannot read one more, which is ADR-0042's and was left uncounted
        // until ADR-0044 recounted this, and the calls pending with them one
        // step per position more. The same paragraph above applies to
        // it: the transfer empties it at every marker and only a join makes an
        // entry value grow.
        let positions: usize = function
            .blocks()
            .map(|block| block.elements.len() + 1)
            .sum();

        // **Nothing holds this number.** Measured: answering `locals` instead
        // leaves the whole suite passing, because no function here takes more
        // visits to a block than it has locals, and building one that did
        // would be building a program for the bound rather than for the check.
        // What a wrong answer costs is what that method promises: too low is a
        // panic naming `Analysis::height`, which is a build that stops with
        // something to read rather than a wrong answer about a program.
        locals * locals * 4 + locals * (locals + 18) + positions * (2 * locals + 3) + locals
    }

    fn on_entry(&self) -> Self::Value {
        let mut known = Known {
            points_to: vec![Held::none(self.locals); self.locals],
            // Nothing has allocated into any of these yet, so none of them can
            // say where it came from. A parameter stays this way: its
            // allocation happened somewhere this check cannot see.
            state: vec![SiteState::Live(None); self.locals],
            // Nothing holds a local's address where a function starts, a
            // parameter included: what a caller holds is its own local.
            escaped: vec![false; self.locals],
            exposed: vec![false; self.locals],
            inside: vec![vec![false; self.locals]; self.locals],
            stale: vec![false; self.locals],
            inside_locals: vec![vec![false; self.locals]; self.locals],
            realloced: vec![None; self.locals],
            from_caller: vec![false; self.locals],
            // Nothing has been read yet, so there is nothing a free could be
            // unordered against.
            pending: BTreeMap::new(),
            calls: BTreeMap::new(),
            exposed_after_call: BTreeSet::new(),
        };

        // A parameter holds whatever the caller passed, which is a thing this
        // function can free and did not make. The module comment says why one
        // has to be a site of its own.
        //
        // At its start, because the site stands for whatever the caller passed
        // and not for an allocation behind it. ADR-0036 says why a free of
        // `p + 1` is still proved on that reading.
        for &parameter in &self.parameters {
            known.points_to[parameter.index()].hold(parameter.index(), Offset::Zero);
        }

        // **And exposed**, so that `release_all(); return *p;` is unproven:
        // the caller may have stashed `p` where `release_all` frees it. What
        // this costs is that a pointer parameter read after any call this
        // check cannot read is unproven too. See ADR-0040.
        known.expose(
            self.exposed_parameters.iter().map(|local| local.index()),
            None,
        );

        known
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        // Before anything below changes either side. See ADR-0048.
        let held_here = into.held_sites();
        let held_there = from.held_sites();
        // **Every field named, never `..`.** A lattice value whose join
        // forgets a field reaches a fixpoint over a value nobody is joining,
        // and nothing else in the build says so: the field is read, the walk
        // ends, and the answer is wrong on exactly the programs a join is for.
        // This is the same as `..` letting a field walk past a match that was
        // otherwise exhaustive, one type over.
        let Known {
            points_to,
            state,
            escaped,
            exposed,
            inside,
            stale,
            from_caller,
            inside_locals,
            realloced,
            pending,
            calls,
            exposed_after_call,
        } = into;
        // Kept only where both arms remember the same `realloc`: a fact one
        // arm lacks is one a branch after the join cannot act on. See ADR-0039.
        for (here, there) in realloced.iter_mut().zip(&from.realloced) {
            // And where one arm left an old allocation in another state: a
            // free on one arm leaves it `Unknown` after the join, which the
            // null arm would read as the `realloc`'s. Found by review.
            let disagree = here.as_ref().is_some_and(|fact| {
                fact.old
                    .iter()
                    .any(|&(old, _)| state[old] != from.state[old])
            });
            if here != there || disagree {
                *here = None;
            }
        }
        // A local's address stored on one arm may be there where the arms
        // meet, as a site is. See ADR-0045.
        for (here, there) in inside_locals.iter_mut().zip(&from.inside_locals) {
            for (here, there) in here.iter_mut().zip(there) {
                *here = *here || *there;
            }
        }
        // Marked on one arm is marked where the arms meet. See ADR-0045.
        for (here, there) in stale.iter_mut().zip(&from.stale) {
            *here = *here || *there;
        }
        // And so is what may hold the caller's. See ADR-0040.
        for (here, there) in from_caller.iter_mut().zip(&from.from_caller) {
            *here = *here || *there;
        }

        for (here, there) in points_to.iter_mut().zip(&from.points_to) {
            here.joined(there);
        }

        // **A site nothing on one side can still name takes the other side's
        // state.** On the arm of `if (c) { free(p); p = 0; }` that freed it,
        // nothing read again holds the allocation, so whether it was freed
        // there is about no execution's next use of it, and joining it in
        // made the other arm's live `p` unproven. Held on both sides, or on
        // neither, the states join as they always did. See ADR-0048.
        for (site, (here, there)) in state.iter_mut().zip(&from.state).enumerate() {
            *here = match (held_here[site], held_there[site]) {
                (true, false) => *here,
                (false, true) => *there,
                (true, true) | (false, false) => here.joined(*there),
            };
        }

        // A local whose address escaped on one arm has escaped where the arms
        // meet: the other arm did not un-take it.
        for (here, there) in escaped.iter_mut().zip(&from.escaped) {
            *here = *here || *there;
        }

        // Exposed on one arm is exposed where the arms meet, and what an
        // allocation may hold on either arm it may hold after them. A union,
        // for the reason `escaped` is one. See ADR-0039.
        for (here, there) in exposed.iter_mut().zip(&from.exposed) {
            *here = *here || *there;
        }
        for (row, other) in inside.iter_mut().zip(&from.inside) {
            for (here, there) in row.iter_mut().zip(other) {
                *here = *here || *there;
            }
        }

        // **A union, because a read on either arm is a read some execution
        // performed.** What this costs when it is wrong is a report about a
        // read that did not happen, which the reader sees; the other direction
        // loses the read that did. The arms themselves do not meet before their
        // join, which is what keeps a read on one arm from being reported
        // against a free on the other: each arm is walked from the value that
        // reached it and not from this one.
        for (key, entry) in &from.pending {
            let here = pending.entry(key.clone()).or_insert(PendingRead {
                at: entry.at,
                sites: BTreeSet::new(),
                // Whichever arm got here first, since the key's place
                // decides the kind and two arms cannot disagree about it.
                // Answering `Read::Dereference` here fails
                // `a_pointer_handed_to_a_call_inside_one_arm_before_a_free_survives_the_join`.
                read: entry.read,
                reachable: BTreeSet::new(),
                after_call: false,
            });
            here.sites.extend(&entry.sites);
            here.after_call |= entry.after_call;
            // Reachable on one arm is reachable where they meet, for the
            // reason `exposed` is a union.
            here.reachable.extend(&entry.reachable);
        }
        for (key, span) in &from.calls {
            calls.entry(*key).or_insert(*span);
        }
        exposed_after_call.extend(&from.exposed_after_call);

        // And applying it, which the union alone does not do. [`Known::settle`]
        // says why a join needs this and the assignments do not cover it.
        into.settle();
    }

    fn element(&self, function: &Function, element: &Element, value: &mut Self::Value) {
        // What is read here is read where this element runs, against what held
        // before it, which is the same question `findings` asks one line
        // earlier and has to get the same answer to.
        //
        // **Before the arms, so that no arm's early return can skip it, and
        // nothing observes that today.** Measured: moving it below the match
        // changes no answer, because the one arm that returns early is the
        // write through a pointer, and this frontend reads an assignment's
        // value back into a temporary, so the read is recorded by that element
        // instead. That is a property of one lowering rather than of the IR,
        // and such a return has already skipped the rules below it here once,
        // turning a suspicion into a proof. Written first because the order is
        // free and the alternative is guarded by nothing.
        value.met(dereferenced_in_element(element));

        // Every field written out, never `..`, which would let a field added
        // to a variant that already exists walk past an exhaustive match.
        match element {
            // Evaluating a place writes nowhere, so no local changes what it
            // holds and no site changes what is known about it. What it reads
            // is not nothing, and `Known::met` above has already taken it: this
            // element exists so that `*p;` on its own can be seen at all.
            Element::Evaluate {
                place: _,
                origin: _,
            } => {}
            // **Every free reaching here is now ordered before everything
            // that follows.** No value moves, so no local's set changes; what
            // changes is that a free this check was holding open can be acted
            // on. See ADR-0022.
            Element::Sequenced { origin: _ } => {
                for state in &mut value.state {
                    if let SiteState::Freed { freed, .. } = state {
                        freed.sequenced = true;
                    }
                }
                // The same fact about a free of a may-set, which ADR-0020
                // records on the local that named the set rather than on its
                // members. One rule, applied to both places a free is written.
                for held in &mut value.points_to {
                    if let Some(freed) = &mut held.freed {
                        freed.sequenced = true;
                    }
                }
                // **And every read behind this is now ordered before whatever
                // frees ahead of it**, so there is nothing left for a later
                // free to be unordered against. The same element answering both
                // directions is the point: one marker, one meaning, read from
                // each side. See ADR-0023.
                value.pending.clear();
                // And every call, for the same reason. Not at
                // `ArgumentsEvaluated`, which orders reads before one call and
                // a call in its arguments before nothing else. No program
                // measured tells the two apart, because that marker is not
                // emitted under an operator that does not sequence.
                value.calls.clear();
                // **And what was made reachable while a call was pending is
                // unproven from here**, as an opaque call leaves what it may
                // free: C may have run the call after the event, so the call
                // may have freed it, and the forward walk met the call first.
                // `r = release_all() + (memset(a, 0, 4) != 0); r = r + a[0];`
                // read `a` in silence. A proved free stays proved. See
                // ADR-0042.
                // Reachable rather than freed: the call reached it through
                // the exposure. See ADR-0047.
                for &site in &value.exposed_after_call {
                    value.state[site].doubted();
                }
                value.exposed_after_call.clear();
            }
            // **Every read behind this is ordered before the call that
            // follows**, which is the half of the marker above that this one
            // says. C17 6.5.2.2 p10's first sentence orders a call's arguments
            // before the call unconditionally, so `free(p + *p)` is a program C
            // defines and was reported until this element existed.
            //
            // The other half is left alone, and the element's own doc comment
            // is where the reason is: concluding it here proved a use after
            // free about `g((free(p), 0), *p)`, whose two arguments C leaves
            // unsequenced. See ADR-0026.
            Element::ArgumentsEvaluated { origin: _ } => value.pending.clear(),
            Element::Assign(operation) => {
                // **This check follows a write through exactly one `Deref` and
                // nothing deeper.** The edge recorded at `Rvalue::Address` is
                // one step, and reading it as two would be inventing the
                // second. `**ppp = q` is therefore a write this check cannot
                // follow at all, and so is a write through any other
                // projection.
                let one_step = operation.place.projection.as_slice() == [Projection::Deref];
                let pointer = operation.place.local.index();
                let targets = if one_step {
                    value.written_through(operation.place.local)
                } else {
                    Vec::new()
                };

                // **Whether this write lands in one local and nowhere else.**
                // ADR-0028's condition, read here and at the replacement
                // further down: one is about what the write may have reached
                // *besides* its target and the other about what it does to
                // that target, and the two have to be the same question.
                // Spelled twice they are one rule in two places, and those
                // drift apart.
                //
                // A deeper projection is never certain, and that is the
                // condition above rather than an extra clause: `written_through`
                // answers about the local the place starts at, so for `**ppp`
                // it answers about `*ppp` and names the wrong thing.
                let certain = one_step
                    && targets.len() == 1
                    && !value.points_to[pointer].writes_elsewhere
                    && !value.escaped[pointer];

                // **Any write through a projection that is not the certain one
                // may have landed in an escaped local.** Not only the one this
                // arm can follow: a write it cannot follow at all is the case
                // that needs this most, and keying the rule on the shape the
                // arm below reads left `**ppp = q` saying nothing whatever
                // while its own corpus case, spelled with a temporary, was
                // answered. Two review lenses found that independently. See
                // ADR-0031.
                if !operation.place.projection.is_empty() && !certain {
                    replaced_by(self.unit, function, &operation.place, value);
                }

                // **Into the allocations the pointer holds, what the write
                // carries is inside them**, exposed whenever they are, by
                // [`Known::expose`]'s closure. A write this check cannot place
                // exposes what it carries at once: one deeper than one `Deref`
                // whose level above names nothing, or one through a pointer that holds
                // neither an allocation nor a local's address, or, for that
                // part, one through a pointer that may also point into memory
                // this check does not model or hold something it lost. A write that
                // may land in followed locals records nothing here: their
                // addresses escaped, and what they hold is in every call's
                // reach. A store into a local aggregate, once fields and
                // indices are lowered, has no targets and lands in the
                // exposing branch, which is a false report the reader can see
                // until it is recorded inside the local instead. See ADR-0039.
                if !operation.place.projection.is_empty() {
                    let written = self.carried(function, &operation.value, value);
                    let mut carried: Vec<usize> = written.sites().collect();
                    // And what a pointer read out of memory may be, which the
                    // sites above do not name: `*b = *a;` stores in `b` what
                    // `a` held. See ADR-0040.
                    if let Rvalue::Use(written) = &operation.value {
                        carried.extend(self.read_out(function, written, value));
                    }
                    // **A store of more than one dereference lands in what the
                    // level above may be**, `**t3 = r` in what `*t3` may point
                    // at, and is recorded there as a store through one is;
                    // `unnamed` below exposes it too, since that set is a lower
                    // bound. Unplaced, `**t3 = r; free(r); ***t3` read nothing
                    // of it. See ADR-0045.
                    let deep = derefs(&operation.place);
                    let containers: Vec<usize> = if one_step {
                        value.sites_of(operation.place.local).collect()
                    } else if deep > 1 {
                        value
                            .stored_below(operation.place.local, deep - 1)
                            .into_iter()
                            .collect()
                    } else {
                        Vec::new()
                    };
                    // **And in the locals the level above may be**, `**t3 = r`
                    // with `*t3 = &slot` in `slot`, as a write through an
                    // alias that may land in several locals: by union, the
                    // proof dropped. See ADR-0045 and ADR-0019.
                    let deep_targets: Vec<usize> = if deep > 1 {
                        value
                            .levels_below(operation.place.local, deep - 1)
                            .1
                            .into_iter()
                            .collect()
                    } else {
                        Vec::new()
                    };
                    for &target in &deep_targets {
                        value.points_to[target].accumulated(&written);
                        // No program observes this, as for the write through
                        // one dereference below, and it is here for the same
                        // reason: keeping the proof is the confident direction.
                        value.points_to[target].freed = None;
                    }
                    // With a target, what it carries is in that local now, and
                    // the local's address escaped: every call reaches it.
                    let unplaced = containers.is_empty() && targets.is_empty();
                    // Written into locals whose addresses escaped, which every
                    // call reaches, so a pending read is told, as at an
                    // exposure. See ADR-0042.
                    if !targets.is_empty() {
                        let routes = value.closure(carried.clone());
                        value.noticed(Some(operation.origin.span()), &routes);
                    }
                    // **A pointer that holds a site may also hold what this
                    // check cannot name**: a pointer read out of memory, which
                    // may be memory the caller owns; a site it lost the name
                    // for; or what a call this check cannot read may have
                    // written into it. For that part the write is unplaced, so
                    // what it carries is exposed as well as recorded in the
                    // sites: `t = c ? s : *tab; *t = a;` stored `a` only in
                    // `s` and was silent after a later call. See ADR-0044.
                    let held = &value.points_to[pointer];
                    let unnamed = held.loaded || held.lost || held.foreign || deep > 1;
                    if unplaced {
                        value.expose(carried, Some(operation.origin.span()));
                    } else {
                        if unnamed {
                            value.expose(carried.clone(), Some(operation.origin.span()));
                        }
                        for container in &containers {
                            for &site in &carried {
                                value.inside[*container][site] = true;
                            }
                            // And the locals whose address it carries, which
                            // are no site. See ADR-0045.
                            for (target, &edge) in written.writes_to.iter().enumerate() {
                                if edge {
                                    value.inside_locals[*container][target] = true;
                                }
                            }
                            // **A load out of a marked allocation is one still
                            // when it is stored**, so the allocation it is
                            // stored in is marked too: `*t4 = *t2;`, or the
                            // same through a local, was followed as nothing
                            // and read in silence. Found by review. Not any
                            // lost value, for the reason `Held::stale_read`
                            // gives. See ADR-0045.
                            if written.stale_read {
                                value.stale[*container] = true;
                            }
                            // And a pointer read out of caller memory keeps
                            // that fact where it is stored. See ADR-0040.
                            if written.from_caller {
                                value.from_caller[*container] = true;
                            }
                        }
                        // Stored where code this check cannot read already
                        // reaches is reachable to it from now on, which no
                        // call marks until the next one: a read before this
                        // store in the same expression is told here.
                        let reachable = value.reachable_now();
                        if containers
                            .iter()
                            .any(|container| reachable.contains(container))
                        {
                            let routes = value.closure(carried.clone());
                            value.noticed(Some(operation.origin.span()), &routes);
                        }
                    }
                }

                if one_step {
                    if targets.is_empty() {
                        return;
                    }

                    // **What is written, before what it is written into.**
                    // Whether this lands in one certain local or in any of
                    // several possible ones is decided below, once the value
                    // is in hand; a pointer that may point at one local is not
                    // a pointer that must, and telling those apart is
                    // ADR-0028.
                    //
                    let written = self.carried(function, &operation.value, value);

                    // **A write this check can be certain about replaces what
                    // the target held.** The set names one local and says it
                    // names all of them, so this write landed in that local
                    // and whatever was there is gone. Union would manufacture
                    // a two-element may-set out of a program that has none,
                    // and ADR-0020 then reads that as real ambiguity and gives
                    // up a proof over it: `int **pp = &p; *pp = q; free(p);
                    // *q = 1;` is a certain use after free and was reported as
                    // a suspicion. See ADR-0028, which is where ADR-0019's
                    // rejected option was taken up once the flag above made
                    // "one target" distinguishable from "at most one target".
                    //
                    // The whole row, as an assignment to the target would
                    // replace it: the sites, what it had lost, the proof about
                    // the set it named and the edge alike. `unproved` is the
                    // one thing the direct assignment does that this does not,
                    // for the reason the paragraph below gives, which is about
                    // the allocation rather than about the local.
                    //
                    // **Both halves of "the edge is all of it" are read here.**
                    // The flag above lives in `Held`, so an assignment to the
                    // pointer destroys it, and that is right for the assignment
                    // itself and wrong for an escape: `int ***ppp = &pp; pp =
                    // &p; opaque(ppp); *pp = q;` gave `pp` a fresh row after
                    // something had already taken its address, and the
                    // replacement fired on an edge anybody could have
                    // overwritten since. `Known::escaped` is the half that
                    // outlives an assignment, which is ADR-0018's rule for
                    // which struct a fact belongs in, and it is why the answer
                    // cannot be recorded on the local whose address is taken.
                    // See ADR-0028. An analysis that proves anything positive
                    // about a local has to answer for its address escaping.
                    if certain {
                        value.points_to[targets[0]] = written;
                        return;
                    }

                    // **The union and nothing else.** A write through an alias
                    // does not unprove what the target held, which #155's rule
                    // for a direct assignment would suggest it should. The
                    // difference is whose fact is at stake: `unproved` writes
                    // on the *sites*, which are shared, so doing it here wiped
                    // a `Freed` that a second local holding the same allocation
                    // had proved. `free(p); *pp = 0; *p = 1;` went from a
                    // proved use after free to a suspicion, and exit 1 to exit
                    // 0, with the write carrying nothing at all.
                    //
                    // Nothing is lost by leaving it out. The target's address
                    // was taken, so ADR-0017 answers `Reached::Lost` for it
                    // wherever a report is made, and `Known::settle` applies
                    // the heap half over the merged value at every join. See
                    // ADR-0019.
                    for target in targets {
                        value.points_to[target].accumulated(&written);
                        // **The proof goes, whatever the accumulator would
                        // say about it.** [`Held::accumulated`] keeps it while
                        // the set does not grow, which is right where an
                        // expression is built from its operands and wrong
                        // here: this write may have replaced the pointer, and
                        // then freeing the target again is not a second free
                        // of anything. The sites stay because keeping them is
                        // the conservative direction for a use after free; the
                        // proof goes because keeping *it* is the confident
                        // one. See ADR-0024.
                        //
                        // **No program observes this, and it is here anyway.**
                        // `writes_to` is written only where an address is
                        // taken, so a target of a write through a pointer has
                        // always escaped, and ADR-0017 answers `Reached::Lost`
                        // for an escaped local wherever a report is made: the
                        // answer is `Unknown` whatever this field says.
                        // Measured, and the narrow claim is the true one:
                        // removing this line breaks nothing today. What it
                        // would cost if the escape stopped covering it is a
                        // proof about a pointer this write may have replaced.
                        value.points_to[target].freed = None;
                    }

                    return;
                }

                // A write through any other projection changes what a pointer
                // points at rather than which allocation a local holds, and
                // this check follows locals.
                if !operation.place.projection.is_empty() {
                    return;
                }

                let destination = operation.place.local;
                match &operation.value {
                    // **Not an optimisation.** `int *p = malloc(4);` lowers to
                    // a call into a temporary and a copy out of it, so without
                    // this the allocation never leaves the temporary and the
                    // headline program is not an error at all.
                    Rvalue::Use(Operand::Copy(source)) if source.projection.is_empty() => {
                        value.points_to[destination.index()] =
                            value.points_to[source.local.index()].clone();
                    }
                    // **Arithmetic on a pointer is followed.** C17 6.5.6 p8
                    // keeps the result inside the object the operand points
                    // into, so `p + 1` may hold whatever `p` holds, and `p[i]`
                    // is that addition: 6.5.2.1 p2 defines `E1[E2]` as
                    // `(*((E1)+(E2)))`. This arm used to clear the destination
                    // instead, on the stated ground that nothing read the
                    // precision yet. Something does now, and until it did the
                    // cost was invisible: `free(p); p[i] = 42;` was silence
                    // rather than a diagnostic, which is the worst answer this
                    // compiler has.
                    //
                    // **The pointer operand, and the integer beside it is not
                    // one.** Which is which is a question about types and
                    // [`built_from`] reads them: 6.5.6 p8 keeps the result
                    // inside the object the *pointer* points into, so a site
                    // the index happens to be is not a site the subscript may
                    // reach. A parameter is a site, so until this was read
                    // `p[i]` unioned the allocation `p` holds with the site `i`
                    // is, and a live site stops the result being proved:
                    // `free(p); p[i] = 42;` was a warning where
                    // `free(p); p[0] = 42;` was an error. See ADR-0030.
                    //
                    // Read before the write, so `p = p + 1` keeps what `p`
                    // held rather than clearing it and unioning the result.
                    Rvalue::Binary { op, lhs, rhs } => {
                        let mut reached = built_from(
                            *op,
                            [lhs, rhs],
                            value,
                            |local| self.is_pointer(function, local),
                            |place| self.may_be_pointer(function, place),
                            |place| self.reads_a_byte(function, place),
                            |local, depth| self.reads_caller_memory(local, depth, value),
                        );
                        // **The sites travel, and the edge only where the offset
                        // may be zero.** C17 6.5.6 p8 keeps the result inside
                        // the object the operand points into, which is why the
                        // allocation comes along; what the edge does is
                        // [`Held::moved_by_arithmetic`]'s. A literal zero the
                        // lowering did not fold is an offset that may be zero,
                        // and is followed. See ADR-0019 and ADR-0021.
                        reached.moved_by_arithmetic();
                        value.points_to[destination.index()] = reached;
                    }
                    // A read through a projection is not a pointer this check
                    // follows, and the report reads it as holding nothing,
                    // which is ADR-0017. It may be a pointer read out of
                    // memory all the same, and what a call reaches and what a
                    // write stores have to know it. See
                    // `Allocations::read_through`.
                    Rvalue::Use(Operand::Copy(source)) => {
                        value.points_to[destination.index()] =
                            self.read_through(function, source, value);
                    }
                    // A constant or a unary operator. Neither is a pointer this
                    // check can follow: C17 6.5.3.3 gives unary `+`, `-` and
                    // `~` arithmetic operands only, and `!` yields an `int`.
                    Rvalue::Use(Operand::Constant(_)) | Rvalue::Unary { .. } => {
                        value.clear(destination);
                    }
                    // The address of a place is not an allocation this check
                    // follows: nothing here frees a local's own storage, which
                    // is `StorageDead` and is the lifetime phase's.
                    //
                    // **But the place whose address is taken stops being
                    // something anything here proved.** A write through the
                    // pointer that just escaped can put a different allocation
                    // in it, and this check does not follow what a pointer
                    // points at, so it would not see the write. `Rvalue::Address`
                    // is the only way a local's address is taken in this IR, so
                    // this is the one door, and leaving it open is what made
                    // `int **pp = &p; *pp = q; free(q); free(p);` silent about
                    // a double free.
                    Rvalue::Address(taken) => {
                        value.clear(destination);
                        // **The edge and the bit, and both are needed.** The
                        // bit outlives everything done to this destination and
                        // is what keeps an escaped local unproved; the edge
                        // dies with the destination and is what lets a write
                        // through it be followed. See ADR-0019.
                        value.points_to[destination.index()].writes_to[taken.local.index()] = true;
                        // **And the set is all of it, where the address is of
                        // the local itself.** `Held::clear` ran a line above,
                        // so this destination points at exactly this local,
                        // which is what lets a write through it replace rather
                        // than union. See ADR-0028.
                        //
                        // `&*pp` is not that. C17 6.5.3.2 p3 makes it `pp`, so
                        // the place it names is what `pp` points at and not
                        // `pp`, while the edge above can only name a local. The
                        // union is the right answer to an edge that names the
                        // wrong thing and a replacement is not, so the flag
                        // stays set and the write stays a may-write. No C
                        // reaches this: the lowering applies the same clause and
                        // folds `&*pp` to a copy of `pp`. Another frontend need
                        // not, which is `docs/c-family.md`'s reason for the IR
                        // expressing the shape at all.
                        value.points_to[destination.index()].writes_elsewhere =
                            !taken.projection.is_empty();
                        value.escaped[taken.local.index()] = true;
                        value.unproved(taken.local.index());
                        // What the local holds is reachable to every call from
                        // here, as an exposure is. See ADR-0042.
                        let routes = value.closure(value.sites_of(taken.local).collect());
                        value.noticed(Some(operation.origin.span()), &routes);
                    }
                }

                // **After the match rather than inside it**, so that every arm
                // is covered and the next one is covered before it is written.
                // Placed per arm, this was two calls and a hole: `p = r + i;`
                // lowers to arithmetic into a temporary and a copy out of it,
                // so the arm that looks like the arithmetic case is reached
                // through the copy, and a mutation of the arithmetic arm broke
                // nothing at all. One call cannot be put in the wrong place.
                value.unproved(destination.index());
                // What an escaped local is given, every call reaches through
                // `Known::reach_of`, as an exposure does. Here and not in
                // `Known::unproved`, which `settle` also runs at a join, where
                // one arm's read and the other arm's call would be paired.
                // See ADR-0042.
                if value.escaped[destination.index()] {
                    let routes = value.closure(value.sites_of(destination).collect());
                    value.noticed(Some(operation.origin.span()), &routes);
                }
            }
            // Storage beginning or ending says nothing about what the local
            // held before, and what it holds now is nothing.
            Element::StorageLive { local, origin: _ } => value.clear(*local),
            Element::StorageDead { origin: _, local } => value.clear(*local),
        }
    }

    fn terminator(&self, function: &Function, terminator: &Terminator, value: &mut Self::Value) {
        // Before the `let ... else` below, which returns for every terminator
        // that is not a call. A `Terminator::Branch` reads its condition and a
        // condition that is exactly a place never becomes an element, so
        // leaving it to the arm that handles calls would be silent about
        // `(*p ? 1 : 0) + (free(p), 0)`.
        value.met(dereferenced_in_terminator(terminator));

        let Terminator::Call {
            callee,
            arguments,
            destination,
            then: _,
            origin,
        } = terminator
        else {
            // Nothing else moves an allocation. Written out rather than `_`,
            // so that a terminator added later has to be answered for here.
            match terminator {
                Terminator::Goto(_)
                | Terminator::Branch { .. }
                | Terminator::Return
                | Terminator::Abnormal { .. } => return,
                Terminator::Call { .. } => unreachable!("the let above took it"),
            }
        };

        // **What the call is handed is carried forwards, as a dereference is
        // by the line above.** The callee's body reads it, and C17 6.5.2.2
        // p10 leaves that body indeterminately sequenced with a later call in
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
            Callee::Reallocates => &arguments[..arguments.len().min(1)],
            Callee::Frees
            | Callee::Allocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Opaque => &arguments[..],
        };
        let touched = Self::touching(handed.iter(), value);
        let sites = || named(&touched);

        // **What a later call may have done to an old allocation is not what
        // the `realloc` did**, so a fact naming one this call touches is
        // forgotten, before anything this call records: a free of `p`, a
        // second `realloc` of it, or `g(p)` between the call and the branch
        // left `p` unproven, and the null arm brought it back live. Found by
        // review. See ADR-0039.
        let touched_now: Vec<usize> = sites().collect();
        value.forget_reallocs_touching(|site| touched_now.contains(&site));

        // **What a `realloc` with a non-zero constant size was handed, while
        // it is all live**, before the transfer below makes it unproven: the
        // branch on the result is what says which it became. See ADR-0039.
        let realloced: Option<Vec<(usize, Option<Span>)>> = match (kind, &arguments[..]) {
            (Callee::Reallocates, [Operand::Copy(old), Operand::Constant(size), ..])
                if *size != 0 && old.projection.is_empty() =>
            {
                let held = &value.points_to[old.local.index()];
                let old: Vec<(usize, Option<Span>)> = held
                    .sites()
                    .filter_map(|site| match value.state[site] {
                        SiteState::Live(made) => Some((site, made)),
                        SiteState::Freed { .. } | SiteState::Unknown | SiteState::Reachable => None,
                    })
                    .collect();
                (!held.lost && !old.is_empty() && old.len() == held.sites().count()).then_some(old)
            }
            _ => None,
        };

        // **A free of what may be the caller's may free anything the caller
        // can see**, as a call this check cannot read may: `f(p, p)` hands
        // `free(b); return *a;` one allocation twice. **Before the `match`,
        // because the free's own arm returns early** for a pointer read out
        // of memory, which is what a pointer read out of caller memory is:
        // after it, `q = *pp; free(q); return **pp;` went silent. Found by
        // review. See ADR-0040.
        if matches!(kind, Callee::Frees | Callee::Reallocates) && self.frees_callers(handed, value)
        {
            value.callers_memory_may_be_freed();
        }

        match kind {
            Callee::Frees => {
                let reached: Vec<usize> = sites().collect();

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
                // That local is `void` and holds none of this, which is #135;
                // measured, putting the clear back inside this branch leaves
                // the whole workspace green.
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
                            value.points_to[place.local.index()].freed =
                                Some(Freeing::new(origin.span()));
                        }
                    }

                    return;
                }

                for site in reached {
                    // Whatever the site was known to have come from survives
                    // the free: the diagnostic wants to name it.
                    let made = match value.state[site] {
                        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                        SiteState::Unknown | SiteState::Reachable => None,
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
                if matches!(kind, Callee::Copies) {
                    if let [Operand::Copy(dest), Operand::Copy(source), ..] = &arguments[..] {
                        let (into_depth, from_depth) = (derefs(dest), derefs(source));
                        let placed = (dest.projection.is_empty() || into_depth > 0)
                            && (source.projection.is_empty() || from_depth > 0);
                        if placed {
                            let (copied, copied_locals) =
                                value.levels_below(source.local, from_depth + 1);
                            // The second half has no case: a lost local copied
                            // is doubted by its own escape first, measured.
                            let marked = value.stale_below(source.local, from_depth + 1)
                                || value.lost_through(source.local, from_depth + 1);
                            // And what was read out of caller memory, as a
                            // store of a load of the source would carry it:
                            // `memcpy(box, pp, 8)` is `*box = *pp;`. Found by
                            // review. See ADR-0040.
                            let caller =
                                self.reads_caller_memory(source.local, from_depth + 1, value);
                            let into: Vec<usize> = if into_depth == 0 {
                                value.sites_of(dest.local).collect()
                            } else {
                                value
                                    .stored_below(dest.local, into_depth)
                                    .into_iter()
                                    .collect()
                            };
                            for container in into {
                                for &site in &copied {
                                    value.inside[container][site] = true;
                                }
                                // And the locals whose address it copies.
                                // See ADR-0045.
                                for &target in &copied_locals {
                                    value.inside_locals[container][target] = true;
                                }
                                if marked {
                                    value.stale[container] = true;
                                }
                                if caller {
                                    value.from_caller[container] = true;
                                }
                            }
                        }
                    }
                }
                // What a pointer it was handed may be, read out of memory, as
                // for an opaque call. See ADR-0040.
                let reach = self.reach(function, handed, sites(), value);
                value.expose(reach, Some(origin.span()));
                value.replaced(|_| true);
            }
            Callee::Opaque => {
                // **What the caller stored is out of sight from here on**, as
                // it is after a free of the caller's pointer. See ADR-0040.
                value.callers_memory_may_be_freed();
                // **A load's sites are a lower bound, and a proof is not taken
                // away on one.** A freed allocation a load may hold is not one
                // this call is known to have been handed, so blanking it would
                // turn a proved use after free into a doubt, which a hatch only
                // lists: that is how `free(p); q = *tab; g(q); return *p;` in
                // a hatch built. What an argument names outright is blanked as
                // before. See ADR-0045.
                let outright = Self::named_outright(handed, value);
                for site in outright {
                    value.state[site] = SiteState::Unknown;
                }
                // **What it holds it may have freed**: what it is handed, by
                // name or as a load, the allocations the memory it was handed
                // holds however deep, and what an argument read out of memory
                // may be. Not the locals whose address that memory holds,
                // which `Known::closure` does not follow (ADR-0047 lists it). `q = *t; release(q);` and `*d = a; release_in(d);` may
                // each free `a`'s allocation, and neither can replace `a`, so
                // a later call by address asks about it. A proved free stays
                // proved here; only what is named outright lost it above. Not
                // `reach`, which adds every escaped local's sites to every
                // call, so `grow(&a)` would reach `a` there and the in-out
                // idiom would be doubted at the next call by address; what
                // only an address or an earlier exposure reaches stays
                // `Reachable`, below. See ADR-0047.
                let mut handed_memory: Vec<usize> = sites().collect();
                for argument in handed {
                    handed_memory.extend(self.read_out(function, argument, value));
                }
                for site in value.closure(handed_memory) {
                    value.state[site].may_be_freed();
                }

                // **Everything this call could reach, and everything reached
                // before it by code this check cannot read**, is unproven
                // after it, because any of it may be what this call frees.
                // What it can reach includes what an argument read out of
                // memory may be, which `reach` says. See ADR-0039.
                let reach = self.reach(function, handed, sites(), value);
                value.expose(reach, Some(origin.span()));
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
                value.replaced(|_| true);
            }
        }

        let Some(place) = destination else {
            return;
        };
        if !place.projection.is_empty() {
            return;
        }

        match kind {
            // `free` returns nothing. The local the lowering writes it into is
            // `void` and holds none of this, which is #135.
            Callee::Frees => {
                value.clear(place.local);
                return;
            }
            // What it returns is its first argument, unmoved: C17 7.24.2 to
            // 7.24.6 say so of each. See ADR-0039.
            Callee::ReturnsFirst | Callee::Copies => {
                let first = match arguments.first() {
                    Some(Operand::Copy(source)) if source.projection.is_empty() => {
                        value.points_to[source.local.index()].clone()
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
                value.points_to[place.local.index()] = first;
                return;
            }
            Callee::Allocates | Callee::Reallocates | Callee::Opaque => {}
        }

        // A call leaves behind something this function did not have before, and
        // the local it landed in is what names it. **Live rather than joined
        // with what was there**: a second turn of a loop through the same call
        // is a second allocation, and carrying the first one's `Freed` across
        // would report a double free for code that allocates each time round.
        let site = place.local.index();
        // Read before the rebirth clears it: a loop through this call writes
        // this site every turn, and what the call returns may be last turn's
        // allocation, exposed, which the site number cannot tell apart.
        let was_exposed = value.exposed[site];
        value.clear(place.local);
        // At its start: the site is whatever the call handed back, so the
        // value is that value and not an offset into it.
        value.points_to[site].hold(site, Offset::Zero);
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
        };
        value.reborn(site, made);
        // **`realloc`'s new object holds what the old one held**, C17 7.22.3.5
        // p2, so what a pointer stored in the old one may hold, the new one
        // may. Reborn with nothing in it, a table grown by `realloc` and then
        // handed to a call exposed nothing it held. See ADR-0039.
        match kind {
            Callee::Reallocates => {
                let old: Vec<usize> = handed
                    .iter()
                    .filter_map(|argument| match argument {
                        Operand::Copy(source) if source.projection.is_empty() => Some(source.local),
                        Operand::Copy(_) | Operand::Constant(_) => None,
                    })
                    .flat_map(|local| value.sites_of(local).collect::<Vec<_>>())
                    .filter(|&old| old != site)
                    .collect();
                for old in old {
                    for held in 0..value.inside.len() {
                        if value.inside[old][held] {
                            value.inside[site][held] = true;
                        }
                        if value.inside_locals[old][held] {
                            value.inside_locals[site][held] = true;
                        }
                    }
                    // And the mark with the row: what the old allocation may
                    // hold of a reborn site is what the new one holds after
                    // the copy. See ADR-0045.
                    if value.stale[old] {
                        value.stale[site] = true;
                    }
                    // And the caller's mark, for the same reason. See
                    // ADR-0040.
                    if value.from_caller[old] {
                        value.from_caller[site] = true;
                    }
                }
                if let Some(old) = realloced {
                    value.realloced[site] = Some(Realloced {
                        at: origin.span(),
                        old,
                    });
                    value.points_to[site].returned_by = Some(site);
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
            Callee::Opaque => {
                let exposed: Vec<usize> = (0..value.exposed.len())
                    .filter(|&other| other != site && value.exposed[other])
                    .collect();
                for other in exposed {
                    value.points_to[site].hold(other, Offset::Unknown);
                }
                if was_exposed {
                    value.state[site] = SiteState::Unknown;
                }
                // **And what it returns is exposed**: the callee had the pointer,
                // and may have kept it where the next call can reach it.
                value.expose([site], Some(origin.span()));
            }
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies => {}
        }
        // After the state, because this is what takes it away again. No C
        // reaches here with an escaped destination: the lowering writes every
        // call into a fresh temporary and copies it out, so the copy above is
        // what a C program goes through. Another frontend need not, and
        // `a_call_into_a_local_whose_address_escaped` builds the shape by hand.
        value.unproved(site);
    }
}

/// Per block of `function`, the locals that may be read from its entry before
/// they are written: a backward fixpoint over the graph.
///
/// **It over-counts reads, and must.** A read it misses clears a local at an
/// edge that something reads later, and a dereference of a local holding
/// nothing says nothing, which is silence. So a local is read wherever it
/// appears in a place other than as the whole destination of an assignment or
/// a call: the base of a projection, an `Index` operand, an operand of an
/// rvalue, a call's argument, a branch's condition, and the return place at
/// `Return`. A local whose address is taken anywhere in the function is live
/// in every block, since a pointer may read it. Every kind of element,
/// rvalue, projection and terminator is written out, so a new one is
/// `error[E0004]` here until it says what it reads. See ADR-0048.
pub(super) fn live_in(function: &Function) -> Vec<Vec<bool>> {
    let locals = function.locals().len();
    let mut always = vec![false; locals];
    // What each block reads before writing it, and what it writes.
    let mut reads_first: Vec<Vec<bool>> = Vec::new();
    let mut writes: Vec<Vec<bool>> = Vec::new();
    let mut next: Vec<Vec<usize>> = Vec::new();

    for block in function.blocks() {
        let mut read = vec![false; locals];
        let mut written = vec![false; locals];
        // A function rather than a closure, since an `Index` operand is a
        // place with projections of its own, `a[b[x]]`, and is read however
        // deep. Found by review.
        fn reads_of_place(place: &Place, read: &mut Vec<bool>, written: &[bool]) {
            if !written[place.local.index()] {
                read[place.local.index()] = true;
            }
            for step in &place.projection {
                match step {
                    Projection::Deref => {}
                    Projection::Index(Operand::Copy(index)) => reads_of_place(index, read, written),
                    Projection::Index(Operand::Constant(_)) => {}
                }
            }
        }
        let reads_of_operand =
            |operand: &Operand, read: &mut Vec<bool>, written: &[bool]| match operand {
                Operand::Copy(place) => reads_of_place(place, read, written),
                Operand::Constant(_) => {}
            };
        // A destination that is the whole local writes it; one through a
        // projection reads the local it goes through.
        let writes_to = |place: &Place, read: &mut Vec<bool>, written: &mut Vec<bool>| {
            if place.projection.is_empty() {
                written[place.local.index()] = true;
            } else {
                reads_of_operand(&Operand::Copy(place.clone()), read, written);
            }
        };

        for element in &block.elements {
            match element {
                Element::Assign(operation) => {
                    match &operation.value {
                        Rvalue::Use(operand) | Rvalue::Unary { operand, .. } => {
                            reads_of_operand(operand, &mut read, &written);
                        }
                        Rvalue::Binary { lhs, rhs, .. } => {
                            reads_of_operand(lhs, &mut read, &written);
                            reads_of_operand(rhs, &mut read, &written);
                        }
                        Rvalue::Address(place) => {
                            always[place.local.index()] = true;
                            reads_of_operand(&Operand::Copy(place.clone()), &mut read, &written);
                        }
                    }
                    writes_to(&operation.place, &mut read, &mut written);
                }
                Element::Evaluate { place, origin: _ } => {
                    reads_of_operand(&Operand::Copy(place.clone()), &mut read, &written);
                }
                Element::StorageLive {
                    local: _,
                    origin: _,
                }
                | Element::StorageDead {
                    local: _,
                    origin: _,
                }
                | Element::Sequenced { origin: _ }
                | Element::ArgumentsEvaluated { origin: _ } => {}
            }
        }
        match &block.terminator {
            Terminator::Goto(_) | Terminator::Abnormal { to: _ } => {}
            Terminator::Branch { condition, .. } => {
                reads_of_operand(condition, &mut read, &written);
            }
            Terminator::Call {
                arguments,
                destination,
                ..
            } => {
                for argument in arguments {
                    reads_of_operand(argument, &mut read, &written);
                }
                if let Some(place) = destination {
                    writes_to(place, &mut read, &mut written);
                }
            }
            Terminator::Return => {
                let place = function.return_place();
                if !written[place.index()] {
                    read[place.index()] = true;
                }
            }
        }
        reads_first.push(read);
        writes.push(written);
        let mut successors = Vec::new();
        block.terminator.successors(&mut successors);
        next.push(successors.iter().map(|block| block.index()).collect());
    }

    let mut live = vec![vec![false; locals]; reads_first.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for block in (0..live.len()).rev() {
            let mut out = vec![false; locals];
            for &successor in &next[block] {
                for (out, &live) in out.iter_mut().zip(&live[successor]) {
                    *out |= live;
                }
            }
            let entry: Vec<bool> = (0..locals)
                .map(|local| {
                    always[local]
                        || reads_first[block][local]
                        || (out[local] && !writes[block][local])
                })
                .collect();
            if entry != live[block] {
                live[block] = entry;
                changed = true;
            }
        }
    }
    live
}

#[cfg(test)]
mod tests {
    //! [`live_in`], one kind of read at a time. Each test builds a function
    //! whose only read of `x` after it is written is of one kind, and asserts
    //! `x` is live at the block that reads it and not at the block that writes
    //! it first. A read `live_in` misses is a local cleared at an edge while
    //! something still reads it, which is silence (ADR-0048), so each test's
    //! mutation is dropping its own kind of read, and it fails on the first
    //! assertion.

    use super::live_in;
    use crate::ir::{
        Block, BlockId, Element, Function, LocalId, Operand, Operation, Origin, Place, Projection,
        Rvalue, Terminator, TranslationUnit, Ty,
    };
    use crate::source::{SourceMap, Span};
    use crate::target::Target;

    /// `x = 0;` in the first block, then `read` in the second, then a return.
    fn liveness(
        read: impl FnOnce(&mut TranslationUnit, LocalId, LocalId, Span, BlockId) -> Block,
    ) -> (bool, bool) {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual(
            "t.c",
            "int f(void) { return 0; }
",
        );
        let at = Span::new(file, 0, 3);
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let pointer = unit.push_type(Ty::Pointer(int));
        let mut function = Function::new(at, int, []);
        let x = function.push_local(pointer);
        let y = function.push_local(int);
        let writes = function.reserve_block();
        let reads = function.reserve_block();
        let exit = function.reserve_block();
        function.fill_block(
            writes,
            Block {
                elements: vec![Element::Assign(Operation {
                    place: Place::local(x),
                    value: Rvalue::Use(Operand::Constant(0)),
                    origin: Origin::Written(at),
                })],
                terminator: Terminator::Goto(reads),
            },
        );
        let block = read(&mut unit, x, y, at, exit);
        function.fill_block(reads, block);
        function.fill_block(
            exit,
            Block {
                elements: vec![],
                terminator: Terminator::Return,
            },
        );
        let live = live_in(&function);
        (
            live[reads.index()][x.index()],
            live[writes.index()][x.index()],
        )
    }

    fn assign(place: Place, value: Rvalue, at: Span, then: BlockId) -> Block {
        Block {
            elements: vec![Element::Assign(Operation {
                place,
                value,
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Goto(then),
        }
    }

    fn deref(local: LocalId) -> Place {
        Place {
            local,
            projection: vec![Projection::Deref],
        }
    }

    #[test]
    fn a_local_copied_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            assign(
                Place::local(y),
                Rvalue::Use(Operand::Copy(Place::local(x))),
                at,
                then,
            )
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_dereferenced_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            assign(
                Place::local(y),
                Rvalue::Use(Operand::Copy(deref(x))),
                at,
                then,
            )
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_used_as_an_index_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            let place = Place {
                local: y,
                projection: vec![Projection::Index(Operand::Copy(Place::local(x)))],
            };
            assign(Place::local(y), Rvalue::Use(Operand::Copy(place)), at, then)
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_written_through_is_read() {
        let (at_read, at_write) = liveness(|_, x, _, at, then| {
            assign(deref(x), Rvalue::Use(Operand::Constant(1)), at, then)
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_in_arithmetic_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            assign(
                Place::local(y),
                Rvalue::Binary {
                    op: crate::ir::BinOp::Add,
                    lhs: Operand::Copy(Place::local(x)),
                    rhs: Operand::Constant(1),
                },
                at,
                then,
            )
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_evaluated_is_read() {
        let (at_read, at_write) = liveness(|_, x, _, at, then| Block {
            elements: vec![Element::Evaluate {
                place: deref(x),
                origin: Origin::Written(at),
            }],
            terminator: Terminator::Goto(then),
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_handed_to_a_call_is_read() {
        let (at_read, at_write) = liveness(|unit, x, _, at, then| {
            let void = unit.push_type(Ty::Void);
            let int = unit.push_type(Ty::Int);
            let pointer = unit.push_type(Ty::Pointer(int));
            let callee = unit.push_function(Function::declaration(at, void, [pointer]));
            Block {
                elements: vec![],
                terminator: Terminator::Call {
                    callee,
                    arguments: vec![Operand::Copy(Place::local(x))],
                    destination: None,
                    then,
                    origin: Origin::Written(at),
                },
            }
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_a_branch_tests_is_read() {
        let (at_read, at_write) = liveness(|_, x, _, at, then| Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(x)),
                then,
                otherwise: then,
                origin: Origin::Written(at),
            },
        });
        assert!(at_read);
        assert!(!at_write);
    }

    /// Mutation: count only the local an `Index` operand names, not the
    /// indices inside it; this fails.
    #[test]
    fn a_local_used_as_an_index_inside_an_index_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            let inner = Place {
                local: y,
                projection: vec![Projection::Index(Operand::Copy(Place::local(x)))],
            };
            let place = Place {
                local: y,
                projection: vec![Projection::Index(Operand::Copy(inner))],
            };
            assign(Place::local(y), Rvalue::Use(Operand::Copy(place)), at, then)
        });
        assert!(at_read);
        assert!(!at_write);
    }

    /// And a local whose address is taken is live everywhere, the block that
    /// writes it included, since a pointer may read it anywhere. Mutation:
    /// leave out `always`; the second assertion fails.
    #[test]
    fn a_local_whose_address_is_taken_is_live_everywhere() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            assign(Place::local(y), Rvalue::Address(Place::local(x)), at, then)
        });
        assert!(at_read);
        assert!(at_write);
    }
}
