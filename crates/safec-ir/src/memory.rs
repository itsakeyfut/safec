//! Whether a program frees one allocation twice, uses one after it was freed,
//! or frees a pointer that is not the start of one.
//!
//! The first checks on [the safety model]'s memory axis, and the first thing
//! this compiler says about what a C program *does* rather than about how it is
//! written.
//!
//! **Five answers out of one walk.** They read one lattice: what a free does to
//! a site is what makes a later use of it a defect, so computing the states
//! twice would be the same computation twice and a second chance for the two
//! copies to disagree. The third asks a free where in its allocation the
//! pointer is, which is ADR-0036, the fourth asks a `return` whether what it
//! hands back was freed, which is ADR-0041, and the fifth asks the same of what
//! a call is handed, which is ADR-0042. [`Kind`] is how the caller tells them
//! apart.
//!
//! **This answers a [`Finding`] rather than a diagnostic.** ADR-0011 keeps this
//! crate from seeing one, and what that buys is a check testable against IR
//! built by hand instead of only through the frontend. `safec_llvm`'s `Refusal`
//! is the same shape, and `safec` turns one of those into a diagnostic too.
//!
//! **An allocation is named by the local it first landed in.** A call's
//! destination and a parameter are the two places one can arrive from, and both
//! are locals, so a site is a [`LocalId`] and there is no second numbering to
//! keep in step. Two calls are two locals because the lowering gives each call
//! its own temporary. A parameter has to be a site as well: without one,
//! `void f(int *p) { free(p); free(p); }` has nothing to mark and the second
//! free is missed in silence, which is the worst thing this compiler can do.
//!
//! **Everything here is about one function.** Nothing reads a callee's body and
//! nothing reads a caller, so `void g(int *a, int *b) { free(a); free(b); }` is
//! silent: `a` and `b` are two sites as far as this can see, and a caller that
//! hands it one pointer twice is a double free this does not find. That is the
//! boundary rather than a defect in it, and what moves the boundary is a
//! summary per function, which nothing here has.
//!
//! [the safety model]: https://github.com/itsakeyfut/safec/blob/main/docs/safety-model.md

mod built;
mod known;
mod parts;

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::Conclusion;
use crate::cfg::Cfg;
use crate::dataflow::{Analysis, solve};
use crate::ir::{
    BlockId, Element, FuncId, Function, LocalId, Operand, Place, Projection, Rvalue, Terminator,
    TranslationUnit, Ty,
};
use crate::nullability::{self, NullAtTerminators};
use crate::source::{SourceMap, Span};

use built::{built_from, named, replaced_by};
use known::Known;
use parts::{
    Callee, Freeing, Held, Offset, PendingRead, Reached, Read, Realloced, SiteState, same,
};

/// The analysis: where an allocation is, and whether it has been freed.
struct Allocations<'a> {
    sources: &'a SourceMap,
    unit: &'a TranslationUnit,
    /// The function this is the analysis of, which every finding names.
    function: FuncId,
    /// How many locals the function has, which is how many sites there can be.
    locals: usize,
    /// The locals a caller filled, which are sites because an allocation can
    /// arrive through one.
    parameters: Vec<LocalId>,
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
    exposed_parameters: Vec<LocalId>,
}

impl Allocations<'_> {
    /// What this check can read in the name of the function being called.
    fn callee(&self, id: FuncId) -> Callee {
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
    fn is_pointer(&self, function: &Function, local: LocalId) -> bool {
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
    /// that reason. Reading every character as a load would expose every
    /// stored pointer at any call a character reaches, and what that costs is
    /// unmeasured; a use after free through such a copy builds, which is
    /// #257.
    fn may_be_pointer(&self, function: &Function, place: &Place) -> bool {
        match self
            .unit
            .place_ty(function, place)
            .map(|ty| self.unit.ty(ty))
        {
            Some(Ty::Pointer(_)) | None => true,
            Some(Ty::Int | Ty::Char | Ty::Void) => false,
        }
    }

    /// What a value read through a projection holds: no site, and whether it
    /// may be a pointer read out of memory.
    ///
    /// One answer for the three places a load is given to something: an
    /// assignment, a write through a pointer, and what a library copy returns
    /// when handed one. One rule in several places drifts apart.
    /// See ADR-0040.
    fn read_through(&self, function: &Function, source: &Place, value: &Known) -> Held {
        let mut held = Held::none(value.points_to.len());
        held.loaded = self.may_be_pointer(function, source);
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
    /// [`used_before`], which asks a read carried to the call about it, so
    /// that what a call is asked about and what it is taken to have reached
    /// cannot disagree. See ADR-0039 and ADR-0040.
    fn reach(
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
    fn frees_anything(&self, callee: FuncId) -> bool {
        self.unit.function(callee).hatch()
    }

    /// What the arguments of a call reach, in the order they were written.
    ///
    /// Shared with [`findings`], so that the walk which reports and the walk which
    /// computes cannot disagree about what a call touches.
    ///
    /// **What they may disagree about is which arguments are handed here**, and
    /// exactly one thing does it: `asked` leaves out a pointer the nullability
    /// check established is null, because C says such a call does nothing, and
    /// only the walk that reports asks that. See ADR-0027, and `asked` for why
    /// the transfer is deliberately not told.
    fn touching<'o>(
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
}

impl Analysis for Allocations<'_> {
    /// **A branch on what a `realloc` returned says what became of what it was
    /// handed.** On the arm where the pointer is null the call failed and the
    /// old allocations are live again; on the other it succeeded and they were
    /// freed at the call. Only where the tested pointer holds exactly the one
    /// allocation the fact is about, and is not one this check stopped
    /// following. Which local the branch tested is the nullability check's
    /// answer, shared, so the two checks read one branch alike. See ADR-0039.
    fn edge(
        &self,
        function: &Function,
        block: BlockId,
        terminator: &Terminator,
        index: usize,
        value: &mut Self::Value,
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

    type Value = Known;

    fn height(&self, function: &Function) -> usize {
        let locals = function.locals().len();
        // Each local's set of sites only grows, so it takes at most one step
        // per site, and a site is a local. Each site's state walks `Live` to
        // `Freed` to `Unknown`; its `freed` span can only move to an earlier
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
        locals * locals * 4 + locals * (locals + 17) + positions * (2 * locals + 3) + locals
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

        for (here, there) in state.iter_mut().zip(&from.state) {
            *here = here.joined(*there);
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
                for &site in &value.exposed_after_call {
                    if let SiteState::Live(_) = value.state[site] {
                        value.state[site] = SiteState::Unknown;
                    }
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
                        SiteState::Freed { .. } | SiteState::Unknown => None,
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
                        SiteState::Unknown => None,
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
                    if let SiteState::Live(_) = value.state[site] {
                        value.state[site] = SiteState::Unknown;
                    }
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
                for site in sites().collect::<Vec<_>>() {
                    if matches!(value.state[site], SiteState::Freed { .. })
                        && !outright.contains(&site)
                    {
                        continue;
                    }
                    value.state[site] = SiteState::Unknown;
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
                        if let SiteState::Live(_) = state {
                            *state = SiteState::Unknown;
                        }
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

/// Which of the five things this check answers about a finding is.
///
/// One walk over one lattice, so this is not five checks and
/// `docs/diagnostics.md` says so where it hands them their codes. What differs
/// is the question: the codes and the words are different, and the thing a
/// caret lands on is a call for a double free, an interior free and an
/// argument after free, a dereference for a use after free, and a `return` for
/// a return after free.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `free(p); free(p);`
    DoubleFree,
    /// `free(p); *p = 42;`
    UseAfterFree,
    /// `free(p + 1);`
    ///
    /// Named for what it catches and not for every invalid free: this is a
    /// pointer into an allocation this check followed, and `free(17)` or a
    /// free of a local's address is not reported under it. See ADR-0036.
    InteriorFree,
    /// `free(p); return p;`
    ///
    /// One of the two reads of a pointer without a dereference this check asks
    /// about, because a caller believes what it is handed is live and nothing
    /// reads this function's body from there. See ADR-0041.
    ReturnAfterFree,
    /// `free(p); g(p);`
    ///
    /// The other, the same belief arriving by the other door: a function's body
    /// believes its pointer parameters live where it starts, and nothing reads
    /// its callers from there. See ADR-0042.
    ArgumentAfterFree,
    /// `free(a); g(&a);`
    ///
    /// Not [`Kind::ArgumentAfterFree`], because what is handed over is live:
    /// it is what it points at that holds a freed pointer, which the callee
    /// may read and use. Never a proof, since it may only write there. See
    /// ADR-0042.
    FreedBehindArgument,
}

/// One thing this check concluded, and where.
///
/// Not a diagnostic: this crate cannot see one. What each conclusion costs a
/// build is `Diagnostic::concluded`'s in `safec`, which is the one place that
/// answers it, and ADR-0001 is why there is only one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The function it was concluded in.
    ///
    /// What lets a driver tell a conclusion about a hatch from one about the
    /// program. The span alone cannot say, because nothing here maps a span
    /// back to the function whose body holds it. See ADR-0038.
    pub function: FuncId,
    /// Which of the two this is.
    pub kind: Kind,
    /// What that check concluded.
    pub conclusion: Conclusion,
    /// Where a caret goes: the call that frees or is handed a freed pointer, or
    /// the element that reads or writes through one, or returns one.
    ///
    /// Not the place's own span, which a [`Place`] does not have: the element's
    /// or the terminator's.
    pub at: Span,
    /// The earliest free reaching here, where the check knows which one it was.
    ///
    /// `None` for most `Unknown`s: what usually makes one unknown is that the
    /// paths or the sites reaching here disagree, so there is no single free to
    /// point at. [`Unproven::Unsequenced`] is the exception, where there is one
    /// and what is open is the order.
    pub freed: Option<Span>,
    /// Where the allocation was made, where this check saw it happen.
    ///
    /// `None` for a parameter, whose allocation is a caller's, and for an
    /// `Unknown` for the reason above. `docs/safety-model.md` asks the
    /// diagnostic for this line and it is honest to leave it off rather than
    /// point at an allocation that may not be the one.
    pub made: Option<Span>,
    /// Why this could not be proven, and `None` where it was.
    ///
    /// One reason rather than a flag per reason. The words a reader is given
    /// differ by reason, so this is what chooses them, and two flags beside
    /// each other would have combinations that mean nothing with only a doc
    /// comment to say so. `Some` exactly where [`Self::conclusion`] is
    /// [`Conclusion::Unknown`].
    ///
    /// **That last sentence is held by nothing.** Every `Verdict` is built
    /// in one function and every [`Finding`] out of a `Verdict`, so the two
    /// fields agree by being written together rather than by a rule anything
    /// checks. A mutation cannot show it either: the arm that would answer for
    /// a reason beside a proof is unreachable.
    pub unproven: Option<Unproven>,
}

/// Why a finding could not be proven.
///
/// **Four reasons and not two, because one of them is about this check and
/// the others are about the program.** A reader told a value may have been
/// freed already is being told something was established somewhere; where this
/// check lost the pointer, nothing was, and saying it anyway is a claim about
/// a program that nobody worked out. Which words each reason gets is
/// `memory_finding`'s in `safec`, for the reason [`Finding`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unproven {
    /// The paths or the sites reaching here disagree about a site that really
    /// was freed, or a call this check cannot read was handed one, or could
    /// reach one because it was exposed, and may have freed it. There is a free
    /// to suspect, and no single one to point at. See ADR-0039.
    Disagreement,
    /// This check stopped following the pointer, so nothing here established a
    /// free at all.
    ///
    /// `Reached::Lost` with nothing else contributing, and it takes the same
    /// word as that variant because it is the same fact reaching the reader.
    /// Every producer sits in one of two functions, which is worth writing out
    /// because the ones a reader meets first are not all of them.
    /// `Known::reached_by` answers it for a local that held a site and lost
    /// the name for it, which is ADR-0018 and which a call this check cannot
    /// read also produces, ADR-0029; and for one whose address escaped, which
    /// is ADR-0017. `Allocations::touching` answers it for an argument
    /// written through a projection, `free(*pp)`, and for one whose local
    /// reaches no site at all. None of them says a free happened; each says
    /// this check can no longer say what the pointer points at.
    ///
    /// Those are private, so they are named here rather than linked: a link
    /// out of a public item to one of them is
    /// `rustdoc::private_intra_doc_links`, which this crate denies.
    Lost,
    /// C has not said which order runs. See ADR-0022.
    Unsequenced,
    /// A pointer this check followed was moved by something it cannot
    /// evaluate, so whether it is still at the start of its allocation is not
    /// known. Only [`Kind::InteriorFree`] carries it. See ADR-0036.
    Offset,
}

/// Every double free this unit contains, and every one it cannot rule out.
///
/// One walk per function: the fixpoint answers what holds where each block
/// starts, and this replays each block from there to find the calls to report.
/// The replay rather than a second lattice, because the transfer is what
/// decides which sites a call touches and having two answers to that is having
/// one of them be wrong.
pub fn findings(sources: &SourceMap, unit: &TranslationUnit) -> Vec<Finding> {
    let mut findings = Vec::new();

    for func in unit.functions() {
        let function = unit.function(func);
        // A declaration has no blocks, and `Function::blocks` panics rather
        // than answering for one.
        if !function.is_defined() {
            continue;
        }

        let analysis = Allocations {
            sources,
            unit,
            function: func,
            locals: function.locals().len(),
            parameters: function.parameters().collect(),
            exposed_parameters: function
                .parameters()
                .filter(|_| sources.snippet(function.name) != "main")
                // A `match` for `Allocations::is_pointer`'s reason: a kind of
                // type added later is asked whether it is exposed.
                .filter(|&local| match unit.ty(function.local(local)) {
                    Ty::Pointer(_) => true,
                    Ty::Int | Ty::Char | Ty::Void => false,
                })
                .collect(),
        };
        let cfg = Cfg::of(function);
        let solution = solve(&analysis, function, &cfg);
        // The other check's answer, asked at each block's terminator, which is
        // where a free is reached. A free of a pointer that check established
        // is null frees nothing, and `asked` is the one place that is applied.
        // See ADR-0027.
        let null = nullability::null_at_terminators(unit, function, &cfg);

        // Per function, because a span belongs to one of them. The third
        // field is where in `findings` the report standing at that caret is,
        // so that a proof arriving later can replace a suspicion: `findings`
        // is appended to and assigned into, never removed from or reordered,
        // until the sort at the end of this function.
        let mut said: Vec<(Span, Place, usize)> = Vec::new();

        for &id in cfg.order() {
            // `Cfg::order` holds exactly the reachable blocks and `solve` gives
            // every reachable block a value, so nothing skips here today. It is
            // a `continue` rather than a panic because what the `None` says is
            // that no execution reaches this block, and saying nothing about
            // code nothing runs is the right answer to that however it arose.
            let Some(mut known) = solution.value(id).cloned() else {
                continue;
            };

            let block = function.block(id);
            for element in &block.elements {
                // Before the transfer, which is what the element does: the
                // question is what was true where it runs.
                used(
                    &mut findings,
                    &mut said,
                    dereferenced_in_element(element),
                    &known,
                    func,
                );
                // After the dereferences in the element, as `handed` is after
                // the ones at a call, so that `return *tab;` keeps a doubt
                // about reading `*tab` over one about what it returns, as
                // `release(*tab)` does. Before the transfer, for the reason
                // above: what is returned is what was held as the write ran.
                returned(
                    &mut findings,
                    &mut said,
                    &analysis,
                    function,
                    element,
                    &known,
                );
                // Just before what is pending is cleared, so that a free in
                // the same expression has reported first, with its label.
                if matches!(
                    element,
                    Element::Sequenced { .. } | Element::ArgumentsEvaluated { .. }
                ) {
                    after_a_call(&mut findings, &mut said, &known, func);
                }
                analysis.element(function, element, &mut known);
            }

            // Before the terminator's own transfer, which is what turns a live
            // allocation into a freed one: the question is what was true when
            // the call was reached.
            used(
                &mut findings,
                &mut said,
                dereferenced_in_terminator(&block.terminator),
                &known,
                func,
            );
            // After the dereferences at this call, so a caret carrying both
            // reads `SC0402` above `SC0407`, and before the transfer, for the
            // reason above.
            handed(
                &mut findings,
                &mut said,
                &analysis,
                function,
                &block.terminator,
                &known,
            );
            findings.extend(reported(&analysis, &block.terminator, &known, &null, id));
            // After the free's own finding, which is the one whose caret is
            // here: what this adds are reports about carets further back, and a
            // reader meets them in the order the sort at the end puts them in
            // rather than the order they were made. Still before the transfer,
            // because what it asks about is what had been read when the call
            // was reached.
            used_before(
                &mut findings,
                &mut said,
                &analysis,
                &block.terminator,
                &known,
                &null,
                id,
            );
            if matches!(block.terminator, Terminator::Return) {
                after_a_call(&mut findings, &mut said, &known, func);
            }
        }
    }

    // In the order a reader's eye goes rather than the order the walk reached
    // them. `Cfg::of` hands blocks back in reverse postorder, so two findings
    // in two arms of one branch come out with the later line first, and
    // `DiagnosticSink` does not sort. Doing it here rather than there because
    // the sink holds diagnostics from every stage and their order is the order
    // the stages ran, which is right; this is one stage disagreeing with
    // itself. `Terminator::successors` already says its order will change when
    // an unwinding call gains an edge, and this is what keeps that from moving
    // every expectation that holds two findings.
    findings.sort_by_key(|finding| (finding.at.file().index(), finding.at.start()));

    findings
}

/// What a set of sites says about whatever touched them.
///
/// The fold from many sites to one conclusion, and the one place either kind
/// of finding makes it. There is one because a may-analysis proves nothing
/// from one member of its set: its join is forced to be right by the lattice,
/// and the code that
/// reads the answer is where the same rule gets lost.
struct Verdict {
    conclusion: Conclusion,
    /// The earliest free reaching here, where there is one to name.
    freed: Option<Span>,
    /// Where that allocation came from, where this check saw it happen.
    made: Option<Span>,
    /// Why this could not be proven, and `None` where it was. Carried through
    /// to [`Finding::unproven`] unchanged.
    unproven: Option<Unproven>,
}

/// How many dereferences `place` is, when it is nothing but dereferences, and
/// zero otherwise.
///
/// One answer for the load and for the report, so that the two cannot disagree
/// about which places are followed below their first level. A place with an
/// `Index` in it is an array's element and is not one of them. No program
/// this compiler accepts tells that apart from following it too, measured: an
/// array of pointers is refused as `SC0304`. See ADR-0045.
fn derefs(place: &Place) -> usize {
    if place
        .projection
        .iter()
        .all(|step| matches!(step, Projection::Deref))
    {
        place.projection.len()
    } else {
        0
    }
}

/// What these sites amount to, or nothing where they amount to no report.
///
/// The caller decides what reaching nothing means, by what it puts in
/// `reached`: a free hands a [`Reached::Lost`] for an argument it stopped
/// following, and a dereference hands an empty iterator. That asymmetry is the
/// design rather than an accident, and [`used`] says why.
fn verdict(
    kind: Kind,
    reached: impl IntoIterator<Item = Reached>,
    known: &Known,
) -> Option<Verdict> {
    // The earliest free reaching here, and whether C has sequenced **every**
    // free that reaches here. [`Freeing::joined`] is both rules, because
    // folding over the sites reached at one point wants exactly what folding
    // over two paths wants: the earlier span, and the conjunction.
    let mut earliest: Option<Freeing> = None;
    // Where the allocation came from, kept only while every freed site agrees.
    // **One level down**: proving a double free from one of several sites
    // is the may-set mistake the fold below is written to avoid, and
    // naming one of several allocations as *the* one is the same mistake about
    // a label. `if (c) p = malloc(); else p = malloc();` reaches both, and
    // pointing at either would be a caret on an allocation the value may not
    // hold.
    let mut made: Option<Span> = None;
    let mut any_freed = false;
    let mut live = false;
    let mut unknown = false;
    // Kept apart from `unknown` because they are unproven for opposite
    // reasons, and the words a reader is given turn on which. A site the paths
    // disagree about was freed on one of them, and a site an opaque call was
    // handed may have been freed by it; a pointer this check lost says nothing
    // about any free anywhere. Counting both as one flag is what put
    // `may free it again here` on a program with one free in it.
    let mut lost = false;
    let mut partial = false;
    // Whether more than one free was folded in. The span below is the earliest
    // of them and the flag beside it is the conjunction, so where they differ
    // the span can be a free this check *has* seen sequenced while the flag is
    // false because another was not. Saying "the order is what is open" about
    // that pair would point two carets at two evaluations C does order.
    let mut several = false;

    for entry in reached {
        let site = match entry {
            Reached::Site(site) => site,
            // A proof about the set: freeing it again takes the same member,
            // whichever it was. It carries no `made`, because naming one of
            // several allocations as the one that was freed is the may-set
            // mistake this whole rule is against.
            Reached::SetFreed(freed) => {
                several |= any_freed;
                any_freed = true;
                earliest = Some(match earliest {
                    Some(already) => already.joined(freed),
                    None => freed,
                });
                continue;
            }
            // **Not the same as proving it live.** This is the check having
            // lost the pointer, and answering nothing about it is the failure
            // `docs/safety-model.md` is written to prevent rather than the one
            // it tolerates.
            Reached::Lost => {
                lost = true;
                continue;
            }
            Reached::Partial => {
                partial = true;
                continue;
            }
        };

        match known.state[site] {
            SiteState::Live(_) => live = true,
            SiteState::Freed {
                made: from,
                freed: before,
            } => {
                made = if any_freed { same(made, from) } else { from };
                several |= any_freed;
                any_freed = true;
                earliest = Some(match earliest {
                    Some(already) => already.joined(before),
                    None => before,
                });
            }
            SiteState::Unknown => unknown = true,
        }
    }

    // **`points_to` is a may-set, so one freed site among several is not a
    // proof.** `SiteState::joined` already answers `Unknown` where one path
    // freed a site and another did not; this is the same question across two
    // sites rather than across two paths, and answering it differently let the
    // spelling of a program decide whether it was a warning or an error. A
    // proof needs every site reached to have been freed, and nothing about it
    // to have been lost.
    // Every site reached was freed and nothing about them was lost. That is
    // the whole of what this check can work out from the states; whether C has
    // put the free first is a separate question with a separate answer, and
    // running them together is a known mistake: one test meaning "proved" and
    // "gave up" at once reports the second as the first.
    // A set that may be missing members proves nothing. See ADR-0045.
    let settled = !live && !unknown && !lost && !partial;

    // **A double free does not turn on which ran first.** Two frees of one
    // allocation are a double free in either order, so there is nothing for a
    // sequence point to settle and asking for one turned `(free(p), 0) +
    // (free(p), 0)` from an error into a warning. Issue #142 said so in as many
    // words and this check did it anyway until review measured it. A use is the
    // other way round: one allowed order reads freed storage and another does
    // not, which is the whole of what this field is for.
    let ordered = match kind {
        Kind::DoubleFree => true,
        Kind::UseAfterFree => earliest.is_some_and(|freed| freed.sequenced),
        // Never asked: `interior` answers that one without folding frees at
        // all. `true` because a pointer that is not the start of an
        // allocation is the wrong thing to hand `free` whatever ran first.
        Kind::InteriorFree => true,
        // A `return` leaves the function after its whole expression, so a free
        // anywhere in it has run by then, and what leaves is freed in every
        // order C allows. See ADR-0041.
        Kind::ReturnAfterFree => true,
        // **Not a `return`'s answer.** A call is not the end of its full
        // expression, so `(free(a), 0) + use(a)` may call `use` before the
        // free, and answering `true` proved a program C defines on that order.
        // See ADR-0042.
        Kind::ArgumentAfterFree => earliest.is_some_and(|freed| freed.sequenced),
        // The same question, one level in. No proof reaches it, because
        // `handed_below` always answers `Reached::Partial` beside its sites.
        Kind::FreedBehindArgument => earliest.is_some_and(|freed| freed.sequenced),
    };

    match earliest {
        // **A null nothing here established does not weaken this.** The site a
        // parameter stands for carries no allocation of its own, so asking for one
        // before answering `Unsafe` would drop the proof on the commonest double
        // free there is, and C17 7.22.3.3 p2 exempts a failed allocation by the
        // same sentence it exempts a null caller passes. What this asserts is that
        // some execution of this function is undefined, which is ADR-0027.
        Some(freed) if settled && ordered => Some(Verdict {
            conclusion: Conclusion::Unsafe,
            freed: Some(freed.at),
            made,
            unproven: None,
        }),
        // **Unproven, and the free is still named.** What is open here is only
        // the order: the sites agree, nothing was lost, and there is exactly
        // one free to point at. A reader given two carets and the note beside
        // them can see the shape of it. See ADR-0022.
        Some(freed) if settled && !several => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: Some(freed.at),
            made,
            unproven: Some(Unproven::Unsequenced),
        }),
        // Neither span is carried. What makes this unproven is that the sites
        // or the paths disagree, so there is no one free that every execution
        // reaching here went through, and no one allocation to name beside it.
        Some(_) => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Disagreement),
        }),
        // **Nothing established a free at all**, so nothing here may say one
        // happened. This check lost the pointer and no site it reached says
        // otherwise, which is what `!unknown` is doing: a site the paths
        // disagree about, and a site an opaque call was handed, are each a
        // free worth suspecting, and the arm below is right about them.
        None if lost && !unknown => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Lost),
        }),
        None if unknown => Some(Verdict {
            conclusion: Conclusion::Unknown,
            freed: None,
            made: None,
            unproven: Some(Unproven::Disagreement),
        }),
        None => None,
    }
}

/// What this terminator is worth reporting as a free, if anything.
///
/// One finding per call and question rather than one per site: a local may
/// point at several allocations where a branch put them there, and two carets
/// on one `free` say one thing twice. **Two questions, though, and one caret
/// can carry both**: in `free(p); free(p + 1);` the second call frees an
/// allocation already freed, and through a pointer that is not its start.
///
/// The double free first, so that a caret carrying both reads `SC0401` above
/// `SC0404`: the sort at the end of [`findings`] is stable and the two share a
/// span. See ADR-0036.
fn reported(
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
    null: &NullAtTerminators,
    block: BlockId,
) -> Vec<Finding> {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return Vec::new();
    };

    // What is handed to be freed: every argument of `free`, and the first of
    // `realloc`, which C17 7.22.3.5 p3 holds to what `free` is held to. A
    // `match` so that a new kind of callee answers here. See ADR-0039.
    let arguments = match analysis.callee(*callee) {
        Callee::Frees => &arguments[..],
        Callee::Reallocates => &arguments[..arguments.len().min(1)],
        Callee::Allocates | Callee::ReturnsFirst | Callee::Copies | Callee::Opaque => {
            return Vec::new();
        }
    };

    // **One answer to what the arguments reached feeds both questions**,
    // because the two have to agree about it, and two walks deciding it would
    // drift apart. The offset is read beside it from the same `asked`, so an
    // argument established null is left out of both or of neither.
    let reached = Allocations::touching(asked(arguments, null, block), known);
    let offset = asked(arguments, null, block)
        .filter_map(|argument| match argument {
            Operand::Copy(place) if place.projection.is_empty() => {
                Some(known.points_to[place.local.index()].offset)
            }
            // A constant frees nothing and a projection is already a
            // `Reached::Lost` above, which `interior` answers nothing for.
            Operand::Copy(_) | Operand::Constant(_) => None,
        })
        .reduce(Offset::joined)
        .unwrap_or(Offset::Zero);

    let finding = |kind, verdict: Verdict| Finding {
        function: analysis.function,
        kind,
        conclusion: verdict.conclusion,
        at: origin.span(),
        freed: verdict.freed,
        made: verdict.made,
        unproven: verdict.unproven,
    };

    // Asked first because `verdict` takes what was reached by value; pushed
    // second, for the reason above.
    let inside = interior(&reached, offset, known);

    let mut found = Vec::new();
    if let Some(verdict) = verdict(Kind::DoubleFree, reached, known) {
        found.push(finding(Kind::DoubleFree, verdict));
    }
    if let Some(verdict) = inside {
        found.push(finding(Kind::InteriorFree, verdict));
    }
    found
}

/// Whether a free hands `free` something other than the start of what it
/// reached.
///
/// C17 7.22.3.3 p2 makes a `free` of anything but a pointer an allocation
/// function returned undefined, and `p + 1` reaches the allocation `p` does,
/// so [`verdict`] cannot see this: the sites are the same and so are their
/// states. What differs is [`Held::offset`].
///
/// Nothing, where any of these holds:
///
/// * a [`Reached::Lost`] is among them. ADR-0017 makes [`Known::reached_by`]
///   answer it for an escaped local that holds sites, so `int **pp = &p; *pp =
///   p + 1; free(p);` never reaches the offset at all, and neither does
///   anything a call this check cannot read may have written. The offset is a
///   positive claim about a local and the address-taken set is what stops it
///   being believed, which any analysis proving something positive about a
///   local owes. **That rule is what covers this line**, and the day something
///   narrows what an escaped local is reported as, this line stops being
///   covered: a field a stronger rule upstream answers for has no guard of
///   its own.
/// * a [`Reached::SetFreed`] is. The sites are gone from `reached` by then, so
///   there is nothing left to be an offset into.
/// * no site at all. A local that reaches nothing is one this check never
///   followed, and [`verdict`] already answers [`Unproven::Lost`] for the same
///   call. [`Allocations::touching`] pushes a `Reached::Lost` for an argument
///   whose local reaches no site, so this rule is reached with nothing in hand
///   in two ways only. One is after a `SetFreed`, which the arm above answers
///   too: **the two hold each other**, so removing either leaves the whole
///   workspace green, and removing both fails
///   `a_free_after_an_offset_that_kept_the_set_is_proved`. The other is a
///   constant argument, `free(0)`, which `touching` skips and so hands this
///   nothing at all; [`reported`] then hands this `Offset::Zero`, which says
///   nothing either. See ADR-0036.
///
/// **A parameter is proved on, and that is sound.** `void f(int *p) { free(p +
/// 1); }` can only free the start of an object if a caller passed a `p` one
/// element before the start of one, which C17 6.5.6 p8 already makes
/// undefined, so every conforming caller leaves this call undefined. ADR-0027
/// is the record that says a conclusion is about an execution this function
/// has.
///
/// `freed` is `None` whatever the answer: what this reports is not about a free
/// that already happened.
fn interior(reached: &[Reached], offset: Offset, known: &Known) -> Option<Verdict> {
    let mut sites = Vec::new();
    for entry in reached {
        match entry {
            Reached::Site(site) => sites.push(*site),
            // `SetFreed` is answered twice: `Known::reached_by` clears the
            // sites whenever it answers it, so the rule below this loop says
            // the same. The doc comment above says what holds the pair.
            Reached::SetFreed(_) | Reached::Lost | Reached::Partial => return None,
        }
    }

    let (&first, rest) = sites.split_first()?;

    // The allocation, where every site agrees about it, because naming
    // one of several as the one freed is a caret on an allocation the value
    // may not hold.
    let made_of = |site: usize| match known.state[site] {
        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
        SiteState::Unknown => None,
    };
    let made = rest
        .iter()
        .fold(made_of(first), |made, &site| same(made, made_of(site)));

    let (conclusion, unproven) = match offset {
        Offset::Zero => return None,
        Offset::NonZero => (Conclusion::Unsafe, None),
        Offset::Unknown => (Conclusion::Unknown, Some(Unproven::Offset)),
    };

    Some(Verdict {
        conclusion,
        freed: None,
        made,
        unproven,
    })
}

/// The arguments of a `free` this report asks about, which is every one except
/// a pointer this compiler established is null.
///
/// C17 7.22.3.3 p2: if the argument is a null pointer, no action occurs. So a
/// call whose only argument is such a pointer frees nothing, and there is
/// nothing here for a double free to be about. `Allocations::touching` already
/// applies this to an argument *written* as a constant, with the clause quoted;
/// this is the same rule reaching a local the nullability check proved. See
/// ADR-0027, which is also why nothing weaker exempts one: a pointer nobody
/// established anything about still weakens no proof.
///
/// **Only the report, never the transfer.** This is called from the walk that
/// reports and `Analysis::terminator` cannot reach it, so a free of a pointer
/// established null still marks its sites freed and still leaves the local
/// holding them. Clearing them instead would make the local reach no site,
/// which is how this check spells having lost a pointer, and the reader would
/// get a warning about the wrong thing. That is the third of ADR-0027's three
/// conditions and it is held by where this function is called from.
///
/// **The result being empty is not [`Reached::Lost`]**, and the two are worth
/// keeping apart here because this check has two emptinesses, and an empty
/// may-set means opposite things to its two readers. `verdict` answers `None`
/// for no sites and nothing
/// lost, which is silence, and that is the right answer to a call C says does
/// nothing. What is lost still arrives as a `Reached::Lost` from the arguments
/// that were asked about.
fn asked<'o>(
    arguments: &'o [Operand],
    null: &'o NullAtTerminators,
    block: BlockId,
) -> impl Iterator<Item = &'o Operand> + 'o {
    arguments
        .iter()
        .filter(move |argument| !established_null(argument, null, block))
}

/// Whether this argument is a pointer this compiler established is null where
/// the call runs.
///
/// **A projection answers `false`.** `free(*pp)` asks what a *place* holds, and
/// the nullability lattice is keyed by the local, which its own doc comment
/// says it pays for. Answering anything else here would be reading a claim
/// about `pp` as a claim about what `pp` points at. Mutation: drop the
/// `projection.is_empty()` guard. `a_free_read_out_of_a_pointer_proved_null`
/// loses its `error[SC0401]` and fails, and nothing else in the suite moves.
///
/// A constant answers `false` as well, although `free(0)` is exempt: it is
/// exempt in `Allocations::touching`, where the operand is read, and two rules
/// for one argument is one of them being wrong. **That arm is held by nothing
/// and cannot be**: answering `true` for a constant changes no program,
/// because `touching` has already skipped every one of them before this is
/// asked. It is written out so the arm says which rule owns it.
fn established_null(argument: &Operand, null: &NullAtTerminators, block: BlockId) -> bool {
    match argument {
        Operand::Copy(place) if place.projection.is_empty() => null.established(block, place.local),
        Operand::Copy(_) | Operand::Constant(_) => false,
    }
}

/// Report every dereference at `at` of something that was freed.
///
/// **A place this check follows no allocation for says nothing**, which is the
/// opposite of what a free of one says, and the asymmetry is deliberate. A free
/// acts on an allocation, so freeing something the check stopped following may
/// be a second free. A dereference only reads one, and a pointer with no
/// allocation behind it is an uninitialised pointer or one into storage that is
/// not the heap: different defects, with checks of their own that do not exist
/// yet. Answering `Unknown` here would warn on every `*p` whose pointer came
/// from anywhere this does not follow, which is most of them.
///
/// **What that silence covers is a boundary rather than a rule.** A pointer
/// written behind this check's back arrives here as `SiteState::Unknown`
/// rather than as no site at all, because taking a local's address is what
/// makes its sites unknown, and `Known::escaped` is what keeps them that way
/// for the rest of the function: `int **pp = &p; p = malloc(8); *pp = q;` used
/// to hand `p` a fresh site nothing had lost, and reading through it was exit
/// 0 on a freed pointer. Pointer arithmetic is followed for the same reason,
/// and so is a controlling expression, which needed `Terminator::Branch` to
/// carry a span before it could be.
///
/// A place whose value is thrown away is read too, because
/// [`Element::Evaluate`] exists to say that it was evaluated: `*p;` on its own
/// used to leave no element at all, so there was nothing here to look at.
///
/// **The rule, rather than a list of what falls outside it: a place whose root
/// reaches no site says nothing, however it came to reach none.** Two ways are
/// known, and the third was closed by making the lowering apply C17 6.5.3.2
/// p3, so `int *r = &*p;` now copies the pointer rather than taking an address
/// of what it reaches. A pointer read out of memory nothing recorded a store
/// into, `int *p = *pp;` with `pp` a parameter, has no site; one read out of
/// the function's own memory holds what was stored there (ADR-0045). And a bare name is never given an element at all, so
/// `free(p); p;` is quiet about reading an indeterminate pointer, which 6.2.4
/// p2 makes undefined and which belongs to an axis with no check. A `return`
/// of one and an argument of a call are the exceptions, and [`returned`] and
/// [`handed`] ask them rather than this.
/// `docs/diagnostics.md` says what exit 0 does not mean here, because a
/// boundary that lives only in a comment is one no user can find.
///
/// [`Element::Evaluate`]: crate::ir::Element::Evaluate
fn used(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    at: Option<(Span, Vec<&Place>)>,
    known: &Known,
    function: FuncId,
) {
    let Some((at, dereferenced)) = at else {
        return;
    };

    for place in dereferenced {
        // **One place at one span said once.** `*p = 42;` lowers to two
        // operations that both read through `p`, because an assignment is an
        // expression with a value and the lowering reads the place back into a
        // temporary. Both are genuine dereferences of one thing and two carets
        // on one line would say it twice.
        //
        // The place and not the finding. Deduplicating finished findings was
        // wrong in both directions at once, measured: two unproven uses on one
        // line carry no spans at all, so they were field-identical and one was
        // thrown away, while `*p = *q;` produced `p`, `q`, `p` in that order
        // and the pair that should have collapsed was not adjacent for
        // `Vec::dedup` to see. What decides whether two reports are one report
        // is which place was dereferenced, and only this knows it.
        //
        // **Each level before the next**, as C reads them: `***t3` reads
        // `t3`'s own allocation, then what was stored in it, then what was
        // stored in that, so a freed level nearer the root is the earlier
        // defect and the one that is said. A deeper level is asked only when
        // every level above it says nothing. Below the first level the order
        // is not visible in what is printed, measured: every such level is
        // unproven and says the same words, so only the first level's place
        // ahead of them is guarded. See ADR-0045.
        let Some(verdict) = verdict(Kind::UseAfterFree, known.reached_by(place.local), known)
            .or_else(|| {
                (1..derefs(place)).find_map(|depth| {
                    verdict(
                        Kind::UseAfterFree,
                        known.reached_below(place.local, depth),
                        known,
                    )
                })
            })
        else {
            continue;
        };

        say(
            findings,
            said,
            place,
            Finding {
                function,
                kind: Kind::UseAfterFree,
                conclusion: verdict.conclusion,
                at,
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }
}

/// Report a `return` of a pointer to an allocation that may have been freed.
///
/// **Asked at the write into the return place, of the local being written**,
/// as [`used`] asks a dereference, so that an escaped local is distrusted here
/// as it is there: asking the return place at [`Terminator::Return`] instead
/// proved a return that a dereference of the same local only doubts, because
/// the return place never escapes. For C the two points are one, since the
/// lowering puts nothing that frees between the write and the return, only the
/// sequence point that ends the `return`'s full expression;
/// `docs/c-family.md` says what that asks of another frontend.
///
/// **Only where the function returns a pointer.** Every call's result is a
/// site and exposed, a later call this check cannot read unproves it, and an
/// addition of two integers keeps its operands' sites (ADR-0030), so
/// `return f() + g();` reached `f`'s result after `g` ran and was refused, in
/// six corpus cases.
///
/// **Every site the local may hold, a parameter's included.** Leaving a doubt
/// about a parameter's allocation to the caller was tried and withdrawn: a
/// caller that hands the result on as an argument asks nothing, so a
/// parameter freed on one arm and returned was silent in every function. See
/// ADR-0041.
///
/// **A place of dereferences is asked what it holds**, `return *tab;` as
/// `int *q = *tab; return q;` is, through [`Known::handed_reached`]. Through
/// [`say`], and after [`used`], because a dereference of the same place at the
/// same span can stand at that key, and it is the earlier read: reading
/// `*tab` comes before returning what it held, so a doubt or a proof about it
/// is the report kept, as at a call. See ADR-0045.
fn returned(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    element: &Element,
    known: &Known,
) {
    let Element::Assign(operation) = element else {
        return;
    };
    if operation.place != Place::local(function.return_place())
        || !analysis.is_pointer(function, function.return_place())
    {
        return;
    }
    // A constant holds no allocation, and a place with an `Index` is an
    // array's element, which this does not follow.
    let Rvalue::Use(Operand::Copy(source)) = &operation.value else {
        return;
    };
    if !source.projection.is_empty() && derefs(source) == 0 {
        return;
    }

    let Some(verdict) = verdict(Kind::ReturnAfterFree, known.handed_reached(source), known) else {
        return;
    };
    say(
        findings,
        said,
        source,
        Finding {
            function: analysis.function,
            kind: Kind::ReturnAfterFree,
            conclusion: verdict.conclusion,
            at: operation.origin.span(),
            freed: verdict.freed,
            made: verdict.made,
            unproven: verdict.unproven,
        },
    );
}

/// Report a pointer handed to a call where the allocation it points at may
/// have been freed.
///
/// **Asked as [`used`] asks a dereference**, of the local an argument reads, so
/// an escaped local is distrusted here as it is there, and a local that
/// reaches no site says nothing: that is the dereference's answer to an empty
/// may-set and not a free's, because the callee reads what it is handed rather
/// than freeing a pointer this check lost. See ADR-0042.
///
/// **And carried to a later call, as a dereference is.** This asks what was
/// true where the call is reached; `Allocations::terminator` records the same
/// arguments as a [`PendingRead`], and [`used_before`] asks them again at a
/// free the same full expression leaves unordered against this call.
///
/// **One finding per place per call**, so `g(p, p)` is one report, and
/// [`handed_places`] is what says which.
///
/// **Through [`say`], because [`used_before`] reaches the same caret about the
/// same local.** `int **q = &a; (memset(a, 0, 4) != 0) + (free(a), 0)` is
/// doubted here, since `a` escaped, and doubted again from the free: pushed,
/// that was two `SC0407` about `a` at one caret. Pushing fails
/// `an_escaped_pointer_handed_to_a_call_before_a_free_is_one_report`. **And a
/// place of dereferences meets the key a dereference of the same place at
/// the same call holds**, which [`used`] fills first, so the read through
/// `tab` is the report kept over what `*tab` hands on.
fn handed(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    function: &Function,
    terminator: &Terminator,
    known: &Known,
) {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return;
    };

    // What each pointer is handed as, first, so that a pointer handed on in
    // its own right keeps its own words: in `give(tab, *tab)`, `*tab` is a
    // freed pointer passed, which says more than what `tab` holds behind it.
    let mut silent: Vec<&Place> = Vec::new();
    for place in handed_places(analysis, function, *callee, arguments) {
        let Some(verdict) = verdict(Kind::ArgumentAfterFree, known.handed_reached(place), known)
        else {
            if place.projection.is_empty() {
                silent.push(place);
            }
            continue;
        };
        say(
            findings,
            said,
            place,
            Finding {
                function: analysis.function,
                kind: Kind::ArgumentAfterFree,
                conclusion: verdict.conclusion,
                at: origin.span(),
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }

    // **And what a pointer that said nothing points at, one level in**, keyed
    // as `*place`, so that `give(tab, *tab)` is one report rather than two:
    // what `tab` hands on one level in is what `*tab` hands on. Not carried
    // forwards as the pointer is, because only allocations already freed are
    // asked, and those are reported here. See ADR-0042.
    for place in silent {
        let Some(verdict) = verdict(
            Kind::FreedBehindArgument,
            known.handed_below(place.local),
            known,
        ) else {
            continue;
        };
        say(
            findings,
            said,
            &Place {
                local: place.local,
                projection: vec![Projection::Deref],
            },
            Finding {
                function: analysis.function,
                kind: Kind::FreedBehindArgument,
                conclusion: verdict.conclusion,
                at: origin.span(),
                freed: verdict.freed,
                made: verdict.made,
                unproven: verdict.unproven,
            },
        );
    }
}

/// The arguments a call is asked about as a pointer it was handed, one place
/// per local.
///
/// **One function for its two readers**, [`handed`] at the call and
/// `Allocations::terminator` carrying them forwards, because one rule written
/// in two places drifts apart inside the change that touches one of them.
///
/// **Which arguments turn on the callee**, and the `match` is written out so
/// that a new kind of callee answers here. What `free` and `realloc`'s first
/// argument are handed is asked already, as a double free, by [`reported`];
/// an allocator and `realloc`'s size are handed integers.
///
/// **A local of pointer type, or a place of dereferences that may be a
/// pointer.** The second is asked what its deepest level holds, through
/// [`Known::handed_reached`], so `release(*tab)` is asked what `q = *tab;
/// release(q);` is (ADR-0045). What the call reaches through it is
/// `read_out`'s (ADR-0040). One place is asked once, compared as a place, so
/// `g(p, *p)` asks both. The pointer test on a place of dereferences changes
/// no case, measured: without a cast, an integer is read out of an allocation
/// that holds integers, and nothing was stored there for it to hold. An integer can hold sites, since
/// an addition keeps its operands' (ADR-0030), and asking one refused
/// `h(f() + g())`, a program with no pointer in it.
///
/// **The repeat test is answered twice, and a mutation sees only the other
/// answer.** Since [`handed`] reports through [`say`], a place handed twice
/// lands on one key, so dropping the test leaves the whole suite green. It is
/// kept, because it says what is asked rather than what happens to collapse
/// afterwards, and measured without `say` it fails
/// `a_freed_pointer_handed_twice_to_one_call_is_one_report`.
///
/// **A place of dereferences lands on the key a dereference of the same
/// place at the same call holds**, which [`used`] fills first. So
/// `free(tab); release(*tab);` keeps the dereference's proof about reading
/// `*tab`, and what `*tab` holds is asked only when that says nothing.
fn handed_places<'a>(
    analysis: &Allocations<'_>,
    function: &Function,
    callee: FuncId,
    arguments: &'a [Operand],
) -> Vec<&'a Place> {
    let arguments = match analysis.callee(callee) {
        Callee::Opaque | Callee::ReturnsFirst | Callee::Copies => arguments,
        Callee::Reallocates => &arguments[arguments.len().min(1)..],
        Callee::Frees | Callee::Allocates => return Vec::new(),
    };

    let mut asked: Vec<&Place> = Vec::new();
    for argument in arguments {
        let Operand::Copy(place) = argument else {
            continue;
        };
        let pointer = if place.projection.is_empty() {
            analysis.is_pointer(function, place.local)
        } else {
            derefs(place) > 0 && analysis.may_be_pointer(function, place)
        };
        if !pointer || asked.contains(&place) {
            continue;
        }
        asked.push(place);
    }
    asked
}

/// Put this finding at its caret, or leave the one already standing there.
///
/// **Both halves of the key, and a case for each.** Keying on the span alone
/// collapses `*p = *q;` after two frees into one report, which
/// `two_pointers_used_after_a_free_on_one_line` fails on. Keying on the place
/// alone collapses `*p = 1; *p = 2;` after one free into one, which
/// `one_pointer_used_after_a_free_on_two_lines` fails on. Neither case reaches
/// the other's mutation, which is why there are two.
///
/// **Recorded where the report is made, and not a line earlier.** Marking the
/// place as said when it had only been looked at spent the right to report it:
/// a dereference this check proved live said nothing and registered anyway, so
/// a later one of the same place at the same span was skipped as a repeat of a
/// report that never happened. Two dereferences do share a span, because both
/// operands of a `&&` or a `||` are written into one temporary at the whole
/// expression's span, and `if (*p || (free(p), *p))` was exit 0 with no output:
/// a proved use of a freed value, silent, which is the worst answer
/// `docs/safety-model.md` allows for. A function rather than the tail of
/// [`used`] because [`used_before`] reaches the same caret from the other
/// direction, and two copies of this rule would be two answers to which report
/// stands.
///
/// **A proof replaces the suspicion standing at this caret**, rather than the
/// first report of a pair winning whatever it concluded.
/// `int **q = &p; if (*p || (free(p), *p))` reports the first read as unproven,
/// because taking a local's address is what makes its sites unknown, and the
/// second read is proved. Keeping the first threw the proof away and exited 0,
/// which is the silence the paragraph above describes arriving through the
/// other door. [`supersedes`] is the rule, and says why only this direction
/// replaces anything.
///
/// The index `said` carries, not the position within `said`: `findings` holds
/// what every caret in this function has said, so anything reported between the
/// pair sits between them. Taking the wrong one overwrites a finding nobody was
/// replacing, and `a_proof_replaces_the_suspicion_at_one_caret` puts a double
/// free in front of the pair so that the two indices differ.
fn say(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    place: &Place,
    finding: Finding,
) {
    let standing = said
        .iter()
        .position(|(said_at, said_place, _)| *said_at == finding.at && said_place == place);

    let Some(standing) = standing else {
        said.push((finding.at, place.clone(), findings.len()));
        findings.push(finding);
        return;
    };

    let index = said[standing].2;
    if supersedes(findings[index].conclusion, finding.conclusion) {
        findings[index] = finding;
    }
}

/// Report every read behind this call that it may be about, where nothing
/// orders the two.
///
/// **The half a forward walk cannot see, arriving from the other side.** A read
/// the walk meets before the call is never asked about it, because a call marks
/// only what follows; carrying the read forwards to the call asks the same
/// question at the only point where both are in hand. `Element::Sequenced` is
/// what says a read is behind rather than beside, and clearing
/// [`Known::pending`] is where that happens. See ADR-0023.
///
/// A read is a dereference or a pointer an earlier call was handed, and each
/// is reported under the code it would have had where it ran. See ADR-0042.
///
/// **Two callees ask it, and they are asking about different things.** A
/// `free` took the site away, so the read may have run after the free. A call
/// this check cannot read may have freed what it was handed, or anything it
/// can reach because it was exposed, which is the same suspicion one step
/// weaker and is what `Allocations::terminator` writes as `SiteState::Unknown`
/// for everything the call may have freed. Both are the question
/// the forward walk already asks on the other side of the call, so refusing one
/// of them here left `g(*p) + h(p)` silent while `h(p) + g(*p)` reported.
/// What differs is the report: an opaque call has no free to point a second
/// caret at, and saying it had one would be this compiler asserting something
/// it did not establish.
///
/// **Always unproven, and that is the shape rather than a caution.** The read
/// and the call are in one full expression with nothing sequencing them, so one
/// allowed order reads freed storage and another does not, and which an
/// implementation picks is unspecified. There is no program this can be right
/// to call `Unsafe` about, so the worst it can do when it is wrong is a report
/// about a read that was ordered after all.
///
/// **It does not go through [`verdict`].** That answers what a set of sites is
/// worth *now*, and now is before the free, where every one of them is still
/// live: it answers `None` here, correctly, to a different question. One
/// judgement point inheriting a rule written for the other question is the
/// mistake in the other direction.
///
/// **It asks [`Allocations::touching`] over the arguments themselves, where
/// [`reported`] asks it over [`asked`], and the two cannot be made to agree
/// because they can never both have something to say.** ADR-0027's exemption
/// takes an argument out wherever [`NullAtTerminators::established`] holds, and
/// three lines put that answer and this walk in different programs:
///
/// 1. `nullability::null_at_terminators` zeroes a block's whole row unless
///    `block.elements.last()` is [`Element::ArgumentsEvaluated`], which is
///    ADR-0027's ordering condition;
/// 2. [`Allocations::element`] answers that same element by clearing
///    [`Known::pending`], because C17 6.5.2.2 p10 orders a call's arguments
///    before the call;
/// 3. this runs after every element of the block and reports only out of
///    `pending`.
///
/// So where a row can exempt anything the marker is last, `pending` was just
/// emptied, and there is nothing here to report; and where this reports, the
/// free is under an unsequenced operator, the marker is absent, and the row is
/// `false` for every local. The `Callee::Opaque` half is the same `pending`
/// reached through the same elements.
///
/// **Written because it was measured and not because it follows**, since a
/// claim that the code cannot be written another way is checked only by
/// writing it that way: the filter was added here, and the reproducer on #219
/// and the whole
/// workspace suite came back byte for byte the same. The `debug_assert!` below
/// is what keeps that true, because a paragraph does not.
///
/// What would make the filter live is an ordering term asked per local across a
/// block edge rather than per block, which is a lattice dimension and is
/// ADR-0027's own Consequences rather than this function's.
fn used_before(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    analysis: &Allocations<'_>,
    terminator: &Terminator,
    known: &Known,
    null: &NullAtTerminators,
    block: BlockId,
) {
    let Terminator::Call {
        callee,
        arguments,
        destination: _,
        then: _,
        origin,
    } = terminator
    else {
        return;
    };

    // Which of the two questions above this is, and the one callee that asks
    // neither. Written as a match rather than as a comparison so that a fourth
    // callee is answered for here by `error[E0004]` rather than falling into a
    // row decided before it existed. The second half is whether the call may
    // free what it was not handed, which only code this check cannot read can.
    let (frees, beyond_its_arguments) = match analysis.callee(*callee) {
        Callee::Frees => (true, false),
        // `realloc` may free what it was handed and may not, which is what an
        // opaque call is to a read carried to it. Only what it was handed:
        // C17 7.22.3.5 p2 deallocates the old object and nothing else.
        Callee::Reallocates => (false, false),
        Callee::Opaque => (false, true),
        // `malloc` frees nothing and takes no pointer, so a read carried to it
        // is a read this call has nothing to say about; the library functions
        // that return their first argument free nothing either.
        Callee::Allocates | Callee::ReturnsFirst | Callee::Copies => return,
    };

    // The machine behind the paragraph above. Placed here because this is
    // where the two rules would have met: `frees` is decided and `touching` is
    // about to be asked over arguments no exemption has been applied to.
    //
    // An assertion rather than the filter, because the filter is a line no
    // mutation can break, and such a line guards nothing, and because it
    // would go quiet on its own the day the ordering term widens: it would
    // exempt a read without anybody asking whether ADR-0027's four conditions
    // still hold where the exemption had newly arrived.
    //
    // **This junction is one the corpus reaches**, which is what makes the
    // assertion worth its line. Deleting the `known.pending.is_empty()`
    // disjunct panics `a_free_of_a_pointer_proved_null`,
    // `a_local_given_nothing_forgets_the_set_it_freed` and
    // `a_pointer_set_to_nothing_after_a_free_holds_nothing`: all three reach
    // here with an argument this check established null, and an empty
    // `pending` is the only reason nothing is reported about it.
    //
    // **What it will not do is catch the widening it is written for, on the
    // corpus.** Forcing `nullability`'s `ordered` true panics nothing; it
    // fails `a_free_in_an_unsequenced_operand_is_not_exempt` and
    // `..._across_a_call_is_not_exempt`, which go from `error[SC0401]` to exit
    // 0 with nothing said. So the widening is already guarded, one row above
    // what this offers, and what this adds is a debug run on a program the
    // corpus does not have: a read carried through a *second* pointer, so that
    // ADR-0025's refinement does not clean the one being freed.
    //
    // Two further measurements, so that the next reader does not repeat them.
    // Weakening this `any` to `all` breaks nothing. Deleting the assertion
    // leaves `null` and `block` unused, which is two warnings, and errors only
    // because the gate runs clippy with `-D warnings`; deleting the two
    // parameters along with it compiles clean. That is a guard against
    // forgetting rather than against deciding.
    debug_assert!(
        !frees
            || known.pending.is_empty()
            || !arguments
                .iter()
                .any(|argument| established_null(argument, null, block)),
        "a read was carried to a free of a pointer established null: `asked` now reaches this walk"
    );

    let touched = Allocations::touching(arguments.iter(), known);
    let taken: Vec<usize> = named(&touched).collect();
    // **A call this check cannot read may free more than it was handed**:
    // whatever it reaches through what it was handed, closed over what those
    // allocations hold, and whatever something else had made reachable to
    // code it cannot read. The second is per read, because the read's own
    // call exposing a site does not make it reachable to this call before the
    // read ran. After a hatch, anything. Asking the arguments alone left
    // `(x = p[0]) + (release_all(), 0)` over a parameter silent while its
    // swapped spelling reported. See ADR-0042, which has the programs each
    // part is held by and the rules it replaced.
    //
    // **Not for a free or `realloc`**, which free only what they are handed.
    let (own_reach, anything) = if beyond_its_arguments {
        let function = analysis.unit.function(analysis.function);
        let reach = analysis.reach(function, arguments, taken.iter().copied(), known);
        (known.closure(reach), analysis.frees_anything(*callee))
    } else {
        (BTreeSet::new(), false)
    };

    for ((.., place, _), read) in &known.pending {
        // **A read this call's own arguments made is behind it**, so it is
        // skipped here and kept for whatever else in the expression may free:
        // `strlen(strcpy(s, t)) + (free(s), 0)` still carries `strcpy`'s
        // argument to the free. See ADR-0043.
        if inside(read.at, origin.span()) {
            continue;
        }
        let both: Vec<usize> = read
            .sites
            .iter()
            .copied()
            .filter(|site| {
                taken.contains(site)
                    || (beyond_its_arguments
                        && (anything || own_reach.contains(site) || read.reachable.contains(site)))
            })
            .collect();

        if both.is_empty() {
            continue;
        }

        // **Only while every allocation they have in common agrees**, which is
        // `verdict`'s rule about `made` and is here for its reason: naming one
        // of several allocations as *the* one is the may-set mistake about a
        // label, and `allocated here` is the claim.
        let mut made = None;
        for (index, site) in both.iter().enumerate() {
            let from = match known.state[*site] {
                SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                SiteState::Unknown => None,
            };
            made = if index == 0 { from } else { same(made, from) };
        }

        say(
            findings,
            said,
            place,
            Finding {
                function: analysis.function,
                // The code the read would have had where it ran, so that
                // `(a[0] = 0) + (free(a), 0)` stays `SC0402` and its memset
                // spelling is `SC0407`, as each is in the other order.
                kind: match read.read {
                    Read::Dereference => Kind::UseAfterFree,
                    Read::Argument => Kind::ArgumentAfterFree,
                },
                conclusion: Conclusion::Unknown,
                at: read.at,
                // Exactly one free, which is this one: the reads are carried
                // to each free separately, so there is nothing folded here and
                // nothing for the caret to be wrong about. An opaque call has
                // freed nothing this check established, so there is no span to
                // put `freed here` on and no order to explain, which is what
                // keeps C17 6.5.2.2 p10's note off a program with no free in
                // it. A label is a claim, and must not say more than the
                // analysis established.
                freed: frees.then(|| origin.span()),
                made,
                unproven: Some(if frees {
                    Unproven::Unsequenced
                } else {
                    // The reason this is, rather than one lent to it: the
                    // variant's own doc says a call this check cannot read was
                    // handed a pointer, or could reach it because it was
                    // exposed, and may have freed it.
                    Unproven::Disagreement
                }),
            },
        );
    }
}

/// Report every pending read that something exposed while a call this check
/// cannot read was pending.
///
/// **The finding `used_before` makes for a read carried to an opaque call**,
/// for the same reason: the call may free what the read read, and which order
/// runs is C's to leave open. It is asked where a marker is about to clear
/// what is pending, and at a return, rather than after every transfer: a free
/// later in the same expression reports through `used_before` first, with
/// `freed here`, and [`say`] keeps the first report at a caret. See ADR-0042.
fn after_a_call(
    findings: &mut Vec<Finding>,
    said: &mut Vec<(Span, Place, usize)>,
    known: &Known,
    function: FuncId,
) {
    for ((.., place, _), read) in &known.pending {
        if !read.after_call {
            continue;
        }
        // Only while every allocation read agrees, for `used_before`'s reason.
        let mut made = None;
        for (index, site) in read.sites.iter().enumerate() {
            let from = match known.state[*site] {
                SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                SiteState::Unknown => None,
            };
            made = if index == 0 { from } else { same(made, from) };
        }
        say(
            findings,
            said,
            place,
            Finding {
                function,
                kind: match read.read {
                    Read::Dereference => Kind::UseAfterFree,
                    Read::Argument => Kind::ArgumentAfterFree,
                },
                conclusion: Conclusion::Unknown,
                at: read.at,
                freed: None,
                made,
                unproven: Some(Unproven::Disagreement),
            },
        );
    }
}

/// Whether a read at `inner` is part of the arguments of the call at `outer`.
///
/// **Strictly inside**, because in C a call's designator and arguments are
/// written within the call and nothing else is, so every read an argument
/// makes, a call nested there included, has a span inside the call's and no
/// read in another operand does. C17 6.5.2.2 p10's first sentence orders the
/// first kind before the call.
///
/// **An equal span is not inside.** The reads that carry exactly a call's span
/// are its own operand reads and what it is handed, which are recorded after
/// the call has been asked, so nothing is lost here by refusing them. What it
/// buys is the direction of a mistake: were a call and a sibling operand ever
/// given one span, the sibling is reported rather than skipped. See ADR-0043,
/// and `docs/c-family.md` for what this asks of another frontend.
fn inside(inner: Span, outer: Span) -> bool {
    inner != outer
        && inner.file() == outer.file()
        && inner.start() >= outer.start()
        && inner.end() <= outer.end()
}

/// Whether a report at a caret replaces the one already standing there.
///
/// **A proof replaces a suspicion, and nothing else replaces anything.** Where
/// two proofs meet at one caret the first stands: both are true of the same
/// place and there is nothing to choose between them. [`used`] is where a pair
/// at one caret comes from and why one is collapsed at all.
///
/// **Answered per pair rather than by an ordering.** [`Conclusion`] does not
/// derive `Ord` and should not: its three variants are three answers rather
/// than three degrees, and `Safe` is not a weaker `Unsafe`. The pairs that
/// cannot arise say so rather than falling through, because a fallthrough in
/// this file was once reached by "proved" and by "gave up" at once and
/// reported the second as the first.
fn supersedes(standing: Conclusion, new: Conclusion) -> bool {
    match (standing, new) {
        // First, because a wildcard below would absorb them. [`verdict`]
        // answers `None` where there is nothing to report, so no `Finding`
        // carries `Safe` and no verdict reaching here concludes one; written
        // last, `(Conclusion::Unsafe, _)` answered `(Unsafe, Safe)` in silence
        // while this said every impossible pair is declared.
        (Conclusion::Safe, _) => unreachable!("a finding standing at a caret concluded Safe"),
        (_, Conclusion::Safe) => unreachable!("a verdict about a dereference concluded Safe"),
        (Conclusion::Unknown, Conclusion::Unsafe) => true,
        (Conclusion::Unknown, Conclusion::Unknown) | (Conclusion::Unsafe, _) => false,
    }
}

/// Where this element runs, and every place it reads or writes through a
/// pointer there.
///
/// Through a pointer, so a projection: an unprojected place is the local itself
/// and holding a freed pointer is not using it. The span is the element's,
/// because a [`Place`] has none of its own.
pub(crate) fn dereferenced_in_element(element: &Element) -> Option<(Span, Vec<&Place>)> {
    // Every field written out, never `..`, which would let a field added to a
    // variant that already exists walk past an exhaustive match.
    match element {
        Element::Assign(operation) => {
            let mut places = projected(&operation.place);
            places.extend(dereferenced_in_rvalue(&operation.value));
            Some((operation.origin.span(), places))
        }
        // The element that exists so this can see it. `projected` rather than
        // the place itself, even though `Element::Evaluate`'s doc says a
        // producer owes a projection: one rule about what counts as reaching
        // through a pointer, applied everywhere, beats two that agree today.
        Element::Evaluate { place, origin } => Some((origin.span(), projected(place))),
        // Neither marker, nor storage beginning or ending, reads anything
        // through anything.
        Element::Sequenced { origin: _ } | Element::ArgumentsEvaluated { origin: _ } => None,
        Element::StorageLive {
            local: _,
            origin: _,
        } => None,
        Element::StorageDead {
            origin: _,
            local: _,
        } => None,
    }
}

/// The same, for what a terminator reads.
pub(crate) fn dereferenced_in_terminator(terminator: &Terminator) -> Option<(Span, Vec<&Place>)> {
    match terminator {
        Terminator::Call {
            callee: _,
            arguments,
            destination,
            then: _,
            origin,
        } => {
            let mut places: Vec<&Place> = arguments.iter().flat_map(dereferenced_in).collect();
            if let Some(destination) = destination {
                places.extend(projected(destination));
            }
            Some((origin.span(), places))
        }
        // **A condition is read here and not from an element, because a
        // condition that is exactly a place never becomes one.** `if (*p + 1)`
        // computes into a temporary and the `Operation` that does it carries
        // the dereference; `if (*p)` hands the place straight to the
        // terminator. Both are one defect and the difference is whether the
        // expression needed a temporary, which is not something a reader could
        // predict, so this arm is what makes the answer the same for both.
        Terminator::Branch {
            condition,
            then: _,
            otherwise: _,
            origin,
        } => Some((origin.span(), dereferenced_in(condition))),
        Terminator::Goto(_) | Terminator::Return | Terminator::Abnormal { to: _ } => None,
    }
}

/// The same, for what an rvalue reads.
fn dereferenced_in_rvalue(value: &Rvalue) -> Vec<&Place> {
    match value {
        Rvalue::Use(operand) => dereferenced_in(operand),
        Rvalue::Unary { op: _, operand } => dereferenced_in(operand),
        Rvalue::Binary { op: _, lhs, rhs } => {
            let mut places = dereferenced_in(lhs);
            places.extend(dereferenced_in(rhs));
            places
        }
        // **Taking an address is not a dereference**, whatever the place
        // it is taken of looks like. C17 6.5.3.2 p3 is why `&*p` used to be
        // the example: "neither that operator nor the `&` operator is
        // evaluated and the result is as if both were omitted". The lowering
        // applies that clause now, so no C program reaches here with one, and
        // what does reach here is an address of a place a frontend really
        // meant to take. Reporting it would be a use of a freed value in a
        // program that never touched one.
        Rvalue::Address(_) => Vec::new(),
    }
}

/// The same, for one operand.
fn dereferenced_in(operand: &Operand) -> Vec<&Place> {
    match operand {
        Operand::Copy(place) => projected(place),
        Operand::Constant(_) => Vec::new(),
    }
}

/// The place, where it goes through a projection, and nothing where it does not.
fn projected(place: &Place) -> Vec<&Place> {
    if place.projection.is_empty() {
        Vec::new()
    } else {
        vec![place]
    }
}
