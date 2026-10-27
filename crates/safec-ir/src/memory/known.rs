//! The lattice value, [`Known`]: what every local may hold and what is known
//! about every site at one point of a function, and every walk over it that more
//! than one rule asks, from what a local reaches through how many levels to what
//! a call this check cannot read may have touched.
//!
//! The transfer functions change it and the report reads it; neither is here.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::{LocalId, Operand, Place};
use crate::source::{SourceMap, Span};

use super::bits::Bits;
use super::parts::{
    Extent, Held, Levels, PendingRead, Reached, Read, ReadKey, Realloced, SiteState, read_key,
};
use super::{LostReason, derefs, inside, named};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Known {
    /// Per local, what it may hold.
    pub(super) points_to: Vec<Held>,
    /// Per site, what is known about it. Meaningless for a local nothing points
    /// at, which is most of them.
    pub(super) state: Vec<SiteState>,
    /// Per local, whether anything holds its address.
    ///
    /// **A property of the local rather than of what it held when the address
    /// was taken.** Marking only the sites it reached at that instant was
    /// #155: assigning to the local afterwards gave it a fresh site nothing
    /// had lost, so `int **pp = &p; p = malloc(8); *pp = q; *p = 1;` read a
    /// freed pointer in silence. Whatever an escaped local is given later is
    /// no more proved than what it held before, because the write that put it
    /// there is not the only write that can reach it.
    pub(super) escaped: Vec<bool>,
    /// Per local, whether code this check cannot read certainly holds its
    /// address: an argument of such a call certainly named it
    /// ([`Held::certain_target`]). Not an address memory may hold, whether a
    /// call reached that memory or a store put it there, since both are
    /// may-facts.
    ///
    /// **Narrower than [`Self::escaped`]**, which `int **pa = &a;` sets though
    /// nothing outside the function can see `pa`. That one makes every call
    /// distrust what the local holds, the safe direction for it.
    /// [`Self::held_out_of_reach`] reads this one to exempt a holder, where
    /// the wide answer is a silence: a route missed here leaves a holder
    /// counted, which is a report. See ADR-0047.
    pub(super) handed_away: Vec<bool>,
    /// Per site, whether code this check cannot read may reach a pointer to it.
    ///
    /// **Once set, set for as long as the allocation lives**, because nothing
    /// here can say that what an unread callee kept has gone: a pointer handed
    /// to one call may be returned or freed by the next. Every opaque call
    /// unproves every exposed allocation still live, and may return any of
    /// them. See ADR-0039.
    pub(super) exposed: Vec<bool>,
    /// Per site, the sites a pointer stored in that allocation may hold.
    ///
    /// What lets a pointer stored in the heap stay proved until something
    /// that can reach the allocation holding it is handed to code this check
    /// cannot read: `*tab = p; log_line();` leaves `p` alone, because nothing
    /// exposed `tab`. Square in the locals, so a bit a pair, for the reason
    /// [`Held::sites`] gives. See ADR-0039.
    pub(super) inside: Vec<Bits>,
    /// Per site, the locals whose address a pointer stored in that allocation
    /// may hold: [`Self::inside`] for what is not a site.
    ///
    /// **A local's address is an edge, not a site** ([`Held::writes_to`],
    /// ADR-0019), so a store of one recorded nothing and `*t3 = &slot; **t3
    /// = r; free(r); return ***t3;` read nothing through `*t3`. Written by a
    /// store of a value carrying edges, read by [`Known::levels_below`], so
    /// a load out of the allocation carries the edges and a store two levels
    /// down lands in the local. A table of its own because a site and a local
    /// share an index. See ADR-0045.
    pub(super) inside_locals: Vec<Bits>,
    /// Per site, what a `realloc` that returned it was handed, until a branch
    /// on a pointer holding it says whether it succeeded. See [`Realloced`]
    /// and ADR-0039.
    pub(super) realloced: Vec<Option<Realloced>>,
    /// Per site, whether what it contains may include an allocation that is
    /// gone and whose site now names a new one.
    ///
    /// **[`Self::inside`]'s column, where [`Held::lose`] is a local's.** A
    /// loop makes every allocation one `malloc` makes under one site, so `*t2 =
    /// p; free(p);` on one turn and the next turn's `malloc` leave `t2`'s entry
    /// naming the new, live allocation, and a load out of `t2` read it in
    /// silence. Set where the site is reborn, the entry kept for what a call
    /// reaches through it, and read by a
    /// load out of the allocation as a pointer this check stopped following.
    /// Set too where such a load is stored into another allocation, through
    /// [`Held::stale_read`].
    /// Nothing clears it: slots are not told apart, so a store into one may
    /// leave the old pointer in another. See ADR-0045.
    pub(super) stale: Vec<bool>,
    /// Per site, whether it may contain a slot holding an allocation a call
    /// this check cannot read reached through another route and could not
    /// replace there. Set by [`Self::held_out_of_reach`], carried where what
    /// a load out of it holds is stored or copied, and read through
    /// [`Held::unreplaced_read`] by [`Self::handed_below`] alone. Nothing
    /// clears it, since slots are not told apart. See ADR-0047.
    pub(super) unreplaced: Vec<bool>,
    /// Per site, whether a value this check lost may have been stored in it.
    ///
    /// **A lost value names no site**, so the free that releases it cannot
    /// say which allocation went, and nothing links it to where it was
    /// stored before: `*box = u; free(u); r = *box;` with `u` lost to a
    /// rebirth read `*r` in silence (#503). [`Self::lost_may_be_gone`] reads
    /// this to mark every such allocation `stale`. Nothing clears it, as
    /// nothing clears `stale`. See ADR-0045.
    pub(super) holds_lost: Vec<bool>,
    /// Per site, whether it is an allocation `malloc` or `aligned_alloc` made
    /// and nothing has written since, on every path: its contents are
    /// indeterminate (C17 7.22.3.1 p2, 7.22.3.4 p2). An intersection at a join, and
    /// cleared only by [`Self::written`] and [`Self::written_all`], in the
    /// direction that can only silence. A pointer read out of it is
    /// reported where it is read, and is
    /// [`Held::fresh_read`](super::parts::Held::fresh_read). Any store clears
    /// it, a store of an indeterminate pointer included; a free does not.
    /// See ADR-0057. Not "empty":
    /// taking a fresh block's contents as empty is the unsound starting set
    /// #579 met.
    pub(super) fresh: Vec<bool>,
    /// Per site, whether what it contains may include a pointer a caller or
    /// a callee handed over: one [`Held::from_caller`] marks, stored here.
    ///
    /// **The mark lived on locals only**, so a store recorded the sites such
    /// a pointer holds, which are none, and `*box = q; release_all(); r =
    /// *box; return *r;` built where `return *q;` was reported. A load out of
    /// a marked allocation reads caller memory, and a call this check cannot
    /// read makes it [`Self::stale`]. Nothing clears it, for `stale`'s
    /// reason. See ADR-0040.
    ///
    /// **A store is not the only producer.** A pointer parameter's site is
    /// marked where the function starts, since what it points at is the
    /// caller's memory, and a call this check cannot read marks its result's
    /// site once it returns, since that is the callee's (#482).
    pub(super) from_caller: Vec<bool>,
    /// What has been read through a pointer since the last sequence point.
    ///
    /// **Ordered containers because a lattice value has to be canonical, and
    /// the type is what holds that rather than a rule somebody maintains.**
    /// [`crate::dataflow::Analysis::Value`] decides whether the walk has ended
    /// by comparing, so a value holding the same facts in two arrangements
    /// never compares equal and never converges. ADR-0016 measured that as a
    /// hang. Written as sorted vectors this needed four rules kept in step by
    /// hand, and a mutation of each left the whole suite green: a map and a set
    /// have no arrangement to get wrong. [`Place`] carries the `Ord` the map
    /// orders by for no other reason. See ADR-0023.
    pub(super) pending: BTreeMap<ReadKey, PendingRead>,
    /// The calls this check cannot read met since the last sequence point,
    /// keyed as [`ReadKey`] is without the place.
    ///
    /// Carried forwards as [`Self::pending`] is, so that something exposed
    /// later in the same expression can ask whether one of them may run
    /// between it and a read. Cleared by [`Element::Sequenced`](crate::ir::Element::Sequenced) alone: a call
    /// inside another call's arguments is ordered before that call, and not
    /// before its siblings. See ADR-0042.
    pub(super) calls: BTreeMap<(usize, u32, u32), Span>,
    /// What was made reachable to code this check cannot read, since the
    /// last sequence point, while a call in [`Self::calls`] that may run after
    /// it was pending.
    ///
    /// For a read the walk meets after both: C may run the event, then the
    /// call, then the read, and [`Self::noticed`] cannot tell a read that is
    /// not pending yet. [`Self::meeting`] asks this instead. See ADR-0042.
    pub(super) exposed_after_call: BTreeSet<usize>,
    /// Whether a value this check lost has been handed to a call it cannot
    /// read, which may have kept it: the lost counterpart of
    /// [`Self::exposed`].
    ///
    /// **A lost value names no site**, so nothing of it is exposed, and the
    /// result of a later such call, which holds every exposed site, carried
    /// nothing of it: `stash(u); r = fetch(); free(u); v = *r;` read `*r` in
    /// silence where a named `u` was reported (#511). Set where such a call
    /// is a trigger for [`Self::lost_may_be_gone`], and read by the result of
    /// that call and every later one, which is lost. A union at a join, and
    /// never cleared, as `exposed` is not: nothing takes back what a callee
    /// was handed. A lost value an escaped local, or memory it points at,
    /// may hold counts as handed to every such call, as `reach_of` has it for
    /// a named one (#515). See
    /// ADR-0045.
    pub(super) lost_exposed: bool,
    /// Per site, how many bytes the object it stands for takes, as far as
    /// this check read it, and `None` where nothing has made the site on this
    /// path.
    ///
    /// Linear in the locals, beside [`Self::state`] and not in a table square
    /// in them, since it is one answer per allocation. Written where a site is
    /// made, a parameter where the function starts and a call's destination
    /// where it returns, and read by the question whether an access fits the
    /// object it reaches. See ADR-0060.
    pub(super) extent: Vec<Option<Extent>>,
    /// Per site, how many bytes a pointer stored there that names nothing
    /// [`Self::inside`] or [`Self::inside_locals`] records is believed to
    /// point at, at most, at level 0, and what each pointer stored there
    /// believed of its own memory at the levels below, with no bound where
    /// no such pointer was stored. See [`Levels`].
    ///
    /// **What the record cannot say**: `char *c = *cc; *m = c;` stores a
    /// pointer read out of the caller's memory, whose object nothing here
    /// records, and `m` handed on as an `int **` was believed to hold whole
    /// `int`s (#632). **And what the stored pointer arrived as**: `*box =
    /// m;` with `m` a `void **` parameter names `m`'s site, which the record
    /// holds, and left behind on `m` that what `m` points at holds pointers
    /// nobody was asked the width of, so `box` read back through an `int
    /// ***` believed whole `int`s two levels down (#634). A bound, so the
    /// lower of two at a join, and kept when the site is made again, as
    /// `inside` is. See ADR-0060.
    pub(super) stored_at_most: Vec<Levels>,
    /// Per local, how many bytes a pointer a call may write into it is
    /// believed to point at, at most: the narrowest width the parameters
    /// it was handed through believe at its level, and `0` where one was a
    /// `void *`. That is what this check asks of a callee's body, which is
    /// asked at the types it stores through: through a `void *` it may have
    /// stored any pointer at any type. C allows no more through one than
    /// through an `int **` (C17 6.5 p7), so this is a limit of the asking
    /// and not of the language. `None` where no call was handed it.
    ///
    /// **What a callee wrote is believed as it was handed**, not as the
    /// local is declared: `int *v` handed to `put(void **slot, char *c)`
    /// holds a `char`'s address afterwards, asked as a `void *` in `put`'s
    /// body, and was believed a whole `int` (#632). A bound, so the lower of
    /// two at a join. See ADR-0060.
    pub(super) handed_at_most: Vec<Option<u64>>,
    /// Per local, whether nothing may have written it since it was made, on
    /// every path: its value is indeterminate (C17 6.2.4 p6, 6.7.9 p10), and
    /// Annex J.2 lists using it as undefined. That line of J.2 names no
    /// condition on the address; the normative text makes such a read
    /// undefined only through a trap representation (6.2.6.1 p5) where the
    /// address is taken, and this reads J.2 as ADR-0057 does for an
    /// allocation. Set where a local that may hold a pointer is made, at the
    /// function's entry and at its `StorageLive`, never for a parameter, and
    /// cleared only by [`Self::written_local`] and [`Self::replaced`], which a write
    /// that may land there calls: an assignment, a placed store, a call's
    /// result, a call, which writes only the locals in
    /// [`Self::exposed_locals`] (#643), and a store or a library copy this
    /// check cannot pin down, which writes only the locals
    /// [`Self::locals_a_write_may_land_in`] answers (#645). An intersection
    /// at a join.
    ///
    /// **Wrong in both directions, and the two are not alike.** A write this
    /// check does not see leaves the mark set, and a program that did write
    /// the local is reported `Unsafe`, which the reader can see. A may-write
    /// counted as a write clears it, and a read nothing wrote goes silent,
    /// which is why no call, store or library copy writes every escaped
    /// local any more, whatever it was handed or wherever it points.
    ///
    /// **What [`Self::fresh`] is for an allocation, for a local reached by
    /// its address**: a pointer read out of one is reported where it is
    /// read, `int *q = m[0];` with `m` a local array nothing wrote (#640).
    /// A local whose address is never taken is `assigned.rs`'s, which reads
    /// it by name. See ADR-0057.
    pub(super) unwritten: Vec<bool>,
    /// Per local, whether code this check cannot read may hold its address:
    /// what a call may have written, for [`Self::unwritten`], and nothing
    /// else reads it. `lost` and `foreign` are still given to every escaped
    /// local, since a route missed there would be a free in silence
    /// (ADR-0029).
    ///
    /// **Not [`Self::exposed`] for what is not a site**, though it is set by
    /// the same kind of event. A library copy exposes the allocations it is
    /// handed and sets nothing here, because it keeps no address: it writes
    /// only through its first argument, which the `Callee::Copies` arm of
    /// the transfer answers for by itself.
    ///
    /// **Set by [`Self::expose_locals`] at a call this check cannot read**,
    /// over what it can reach. A store this check cannot place sets nothing
    /// here: it records the address in memory such code can reach,
    /// [`Self::landed_where_unknown_code_points`], which the next call's walk
    /// starts from. A union at a join. Never cleared while the local lives,
    /// for the reason [`Self::exposed`] gives: nothing here can say that a
    /// callee has dropped an address it kept, so `stash(m); tick();` may
    /// write `m` at the second call. **Cleared at its `StorageLive`**, where
    /// the local is a new object and an address kept from the old one is
    /// indeterminate (C17 6.2.4 p2): kept across, `stash(m)` in one turn of
    /// a loop made `tick()` in the next a write of the new `m`, and a read
    /// nothing wrote was silent.
    ///
    /// **Wrong in both directions, and the two are not alike.** A route the
    /// walk misses leaves the bit unset, and a program a call did write is
    /// reported `SC0411`, which the reader sees. A bit set where no call can
    /// reach clears a mark, and a read nothing wrote goes silent. See
    /// ADR-0057.
    pub(super) exposed_locals: Vec<bool>,
}

impl Known {
    /// Every site this local may point at.
    pub(super) fn sites_of(&self, local: LocalId) -> impl Iterator<Item = usize> + '_ {
        self.points_to[local.index()].sites()
    }

    /// Stop following whatever this local held.
    pub(super) fn clear(&mut self, local: LocalId) {
        self.points_to[local.index()].clear();
    }

    /// Every site this local may hold, and whether it may hold more.
    ///
    /// The one question both readers ask, so that what a free touches and what
    /// a dereference reads cannot disagree. [`escaped`] says the set may be
    /// stale, which is a fact about this check's knowledge of one local and is
    /// answered here; what an escape does to the allocations themselves is a
    /// fact about the heap and is written on the sites by [`Known::unproved`].
    /// See ADR-0017.
    ///
    /// **A local reaching no site answers nothing, escaped or not.** [`used`](super::report::used)
    /// says nothing about a place it follows no allocation for, however it came
    /// to follow none, and an address taken does not change that: a pointer
    /// this check never had a site for is an indeterminate pointer, which is a
    /// different defect with a check of its own that does not exist yet.
    /// Answering [`Reached::Lost`] here instead put `perhaps after the free` on
    /// a program that frees nothing at all, and made a build that denies
    /// unknown unusable on the output-parameter idiom. `an_escaped_local_that_reaches_no_site_at_all`
    /// is that program and is the guard.
    ///
    /// It used to be `int *p; int **pp = &p; *pp = malloc(4); *p = 1;`, which
    /// ADR-0019 made this check follow: `p` now reaches the site the write put
    /// there, so the rule above no longer covers it and the escape reports it
    /// anyway. The cost that rule was written to avoid is paid on that program
    /// regardless, by a route that knows what it is talking about.
    ///
    /// [`Allocations::touching`](super::transfer::Allocations::touching) wants the opposite answer for an empty set and
    /// writes its own, which is why the rule is not written here.
    ///
    /// [`escaped`]: Known::escaped
    pub(super) fn reached_by(&self, local: LocalId) -> Vec<Reached> {
        let mut reached: Vec<Reached> = self.sites_of(local).map(Reached::Site).collect();

        // **Two facts, one answer, and the emptiness condition belongs to only
        // one of them.** A local that *had* a site and lost the name for it is
        // holding something unnameable whether or not it holds anything else,
        // which is what [`Reached::Lost`] is for and is ADR-0018. A local whose
        // address escaped is distrusted only about the sites it has: reaching
        // none of them makes it a pointer this check never followed, and
        // answering here put `perhaps after the free` on programs that free
        // nothing at all, which is ADR-0017.
        //
        // One `push` rather than two conditions, because two would answer
        // `Lost` twice for a local that is both. That is inert, since
        // [`verdict`] joins the reasons and two `Other`s are `Other`, and a
        // reader should not have to work that out.
        // **It replaces the members rather than joining them.** What is known
        // is about the set, and its members were each left `SiteState::Unknown`
        // by the free that could not say which one it took. Answering both
        // folds one of those in beside the proof, and `verdict` needs nothing
        // unknown to prove anything, so the proof goes. Written the other way
        // first and measured: `a_branch_that_allocates_either_way` dropped to a
        // warning, which is the rule eating the marking it made. See ADR-0020.
        //
        // **It replaces, and it does not return.** The rules below are about
        // this local too and they still apply: a set this local freed says
        // nothing about whether something else has written a different pointer
        // into it since. Returning here made `free(p); opaque(&p); free(p);`
        // over a two-site `p` a proved double free, which is the false proof
        // this rule exists to stop, arriving through a door it had just
        // opened.
        if let Some(freed) = self.points_to[local.index()].freed {
            reached.clear();
            reached.push(Reached::SetFreed(freed));
        }

        if self.points_to[local.index()].lost
            || (self.escaped[local.index()] && !reached.is_empty())
        {
            reached.push(Reached::Lost(LostReason::Other));
        }
        // **A load's sites are what was stored where it was read from**, which
        // may not be all it holds: the report may doubt from them and never
        // prove. See ADR-0045.
        if self.points_to[local.index()].loaded && !reached.is_empty() {
            reached.push(Reached::Partial);
        }
        // **And so is a set beside a local's address**: on the path that gave
        // it `&x`, it points at `x` and at none of its sites, so the sites
        // being freed proves nothing about every path. `if (c) q = p; else q
        // = &x; free(p); return *q;` was a proved use after free that the
        // `else` path does not commit. Found by review of ADR-0048, whose
        // join opened a second way to it.
        if self.points_to[local.index()].writes_to.any() && !reached.is_empty() {
            reached.push(Reached::Partial);
        }

        reached
    }

    /// What the value of `place` may point at, where it is handed to a call or
    /// returned: a local's own sites, or, for a place of dereferences, what its
    /// deepest level holds, marked as possibly incomplete.
    ///
    /// **One answer for the report and for the read carried forwards**, so that
    /// `release(*tab)` is asked at the call and at a later free about the same
    /// thing, and so that it is asked what `q = *tab; release(q);` is asked.
    /// See ADR-0042 and ADR-0045.
    pub(super) fn handed_reached(&self, place: &Place) -> Vec<Reached> {
        if place.projection.is_empty() {
            self.reached_by(place.local)
        } else {
            self.reached_below(place.local, derefs(place))
        }
    }

    /// What a pointer handed to a call points at may hold that may have been
    /// freed: the allocations [`Self::stored_below`] says it may contain one
    /// level in, through `&a` or through what this function stored in its own
    /// memory, that are freed or unproven.
    ///
    /// **Not one only [`SiteState::Reachable`] through the address handed**,
    /// because taking an address makes a live allocation that (ADR-0017), so
    /// asking about it doubted `use2(&a)` over every live pointer. One this
    /// check saw may have been freed, `Unknown`, is asked however it is
    /// reached: freed on one path, or handed to `release(a)`, and then
    /// `use2(&a)` was silent (ADR-0047). A freed one is asked whichever way it
    /// is reached, and a reachable one through this function's own memory is,
    /// since nothing about the call made it so: `*t = a; release(a); use2(t);`
    /// was silent. And a reachable one is asked where the address handed
    /// passes through a pointer read out of an allocation
    /// [`Self::unreplaced`] marks: `c = *h; use2(&c);` after a call that
    /// reached what `*h` holds through another route. **Never a proof**:
    /// [`Reached::Partial`] is always beside them, since the callee may only
    /// write there. See ADR-0042.
    ///
    /// **And a pointer this check lost, where the memory pointed at is
    /// [`Self::stale`]**: it held an allocation a loop has since made again
    /// at the same site, so what it holds may be the old one, freed, while
    /// the site reads `Live` for the new one. A box filled on one turn and
    /// freed through on the next was returned, stored out or handed to a call
    /// in silence, where a load of the same box was doubted (#651).
    ///
    /// **A place of dereferences is asked from the level its load would
    /// hold**, by [`Self::handed_level`], so `use2(*k)` and `q = *k;
    /// use2(q);` are one question. Before, only a plain local was asked one
    /// level in, and `free(a); *k = &a; use2(*k);` built where the second
    /// spelling was refused. See ADR-0045.
    pub(super) fn handed_below(&self, place: &Place) -> Vec<Reached> {
        let Some((sites, locals)) = self.handed_level(place) else {
            return Vec::new();
        };
        // **Only what the address alone reaches.** A load out of memory
        // carries the locals it may be as edges too, so `m = *h;` with `*h`
        // holding either `&slot` or `x` reaches `x`'s contents through
        // memory and `slot`'s through an edge, and exempting everything
        // `slot` holds silenced a freed pointer `x` held as well. Found by
        // review.
        let through_memory: BTreeSet<usize> = sites
            .iter()
            .copied()
            .flat_map(|container| self.inside[container].ones().collect::<Vec<_>>())
            .collect();
        let through_address: BTreeSet<usize> = locals
            .iter()
            .flat_map(|&target| self.points_to[target].sites())
            .filter(|site| !through_memory.contains(site))
            .collect();
        let read_unreplaced = locals
            .iter()
            .any(|&target| self.points_to[target].unreplaced_read);
        let mut reached: Vec<Reached> = self
            .level_below(&sites, &locals)
            .0
            .into_iter()
            .filter(|&site| match self.state[site] {
                SiteState::Freed { .. } => true,
                SiteState::Unknown | SiteState::Unheld => true,
                SiteState::Reachable => !through_address.contains(&site) || read_unreplaced,
                SiteState::Live(_) => false,
            })
            .map(Reached::Site)
            .collect();
        if sites.iter().any(|&container| self.stale[container]) {
            reached.push(Reached::Lost(LostReason::Other));
        }
        if !reached.is_empty() {
            reached.push(Reached::Partial);
        }
        reached
    }

    /// What a dereference `depth + 1` levels below `local` reads: what
    /// [`Self::stored_below`] says the level above it may contain, marked as
    /// possibly incomplete for the reason a load is.
    ///
    /// **`**tab` is one place with two `Deref`s**, and was judged on `tab`'s
    /// own sites alone, so a use after free one level down said nothing:
    /// `*tab = p; release(*tab); return **tab;`. Each level below the first is
    /// asked **after** the ones above it rather than in place of them, which
    /// is what [`used`](super::report::used) does: asked instead, `free(tab); return **tab;` went
    /// from a proof to silence. See ADR-0045.
    pub(super) fn reached_below(&self, local: LocalId, depth: usize) -> Vec<Reached> {
        let mut reached: Vec<Reached> = self
            .stored_below(local, depth)
            .into_iter()
            .map(Reached::Site)
            .collect();
        if !reached.is_empty() {
            reached.push(Reached::Partial);
        }
        // A level read through a marked allocation may be one this check
        // stopped following. See ADR-0045.
        if self.stale_below(local, depth) {
            reached.push(Reached::Lost(LostReason::Other));
        }
        // And below a pointer to a local this check lost, at any level, which
        // is `**t2` and `*t2` handed on, as a load of `*t2` is. See ADR-0045.
        if self.lost_through(local, depth) {
            reached.push(Reached::Lost(LostReason::Other));
        }
        reached
    }

    /// Whether a local a read `depth` dereferences below `local` passes
    /// through is one this check stopped following, so that the read is lost
    /// where a direct read of the local is. Its sites alone, which is what
    /// [`Self::level_below`] carries across, would read as followed.
    ///
    /// **At every level, not only the first.** The walk steps through a local
    /// at any level, so `q = **t3` with `*t3 = &t2` reads through `t2` as `u =
    /// *t3; q = *u;` does, and asking the first level alone left the one-place
    /// spelling silent where the other was doubted. Found by review. See
    /// ADR-0045.
    pub(super) fn lost_through(&self, local: LocalId, depth: usize) -> bool {
        self.locals_read_through(local, depth)
            .into_iter()
            .any(|target| self.points_to[target].lost)
    }

    /// The locals a read `depth` dereferences below `local` passes through:
    /// those the local's own edges name, then those each level of
    /// [`Self::level_below`] reaches, short of the last.
    fn locals_read_through(&self, local: LocalId, depth: usize) -> BTreeSet<usize> {
        let mut sites: BTreeSet<usize> = self.points_to[local.index()].sites().collect();
        let mut locals: BTreeSet<usize> = self.written_through(local).into_iter().collect();
        let mut passed = BTreeSet::new();
        for level in 0..depth {
            passed.extend(locals.iter().copied());
            if level + 1 < depth {
                (sites, locals) = self.level_below(&sites, &locals);
            }
        }
        passed
    }

    /// Whether a local a read through `local` passes through lost an
    /// allocation that may be gone: [`Held::stale_read`] read along the walk,
    /// as [`Self::lost_through`] reads `lost`, so a load through it carries
    /// what a load of the local itself would. See ADR-0045.
    ///
    /// **Reading `lost` here instead changes no answer, measured**, though it
    /// is the wider rule: a local whose address is taken has its allocation
    /// left unproven by the escape (ADR-0017), so a rebirth of it always
    /// counts as gone and sets the bit, and one lost to a call writing
    /// through its address is doubted at the read by that allocation's state.
    pub(super) fn stale_through(&self, local: LocalId, depth: usize) -> bool {
        self.locals_read_through(local, depth)
            .into_iter()
            .any(|target| self.points_to[target].stale_read)
    }

    /// Whether a local a read `depth` dereferences below `local` passes
    /// through holds memory a caller or a callee handed over,
    /// [`Held::from_caller`], so that `pr = &r; s = *pr;` carries what `s =
    /// r;` would. Read along the walk [`Self::lost_through`] takes, for its
    /// reason: the mark is on the local, and the sites alone would read as
    /// this function's own. Without it the read through the address was
    /// believed after a later call that may free what `r` holds, for a
    /// parameter's memory as for a callee's (#394). See ADR-0040.
    pub(super) fn caller_through(&self, local: LocalId, depth: usize) -> bool {
        self.locals_read_through(local, depth)
            .into_iter()
            .any(|target| self.points_to[target].from_caller)
    }

    /// What a pointer handed to a call holds: the allocations it may point
    /// at and the locals whose address it may be. For a plain local, what it
    /// holds and its address edges; for a place of dereferences, the level a
    /// load of it would hold, which is what `q = *k;` puts in `q`. Nothing
    /// for a local this check lost, whose contents are not named. See
    /// ADR-0045.
    fn handed_level(&self, place: &Place) -> Option<(BTreeSet<usize>, BTreeSet<usize>)> {
        if place.projection.is_empty() {
            self.level_zero(place.local)
        } else {
            Some(self.levels_below(place.local, derefs(place)))
        }
    }

    /// Whether a pointer read `depth` dereferences below `local` may be one
    /// [`Self::stale`] says this check stopped following: whether any
    /// allocation the chain reads through, at any level, is marked.
    ///
    /// Walked as [`Self::stored_below`] walks what the levels contain, so the
    /// two answer the same places. Not for a lost local, whose load is
    /// answered by [`Self::stored`]. See ADR-0045.
    pub(super) fn stale_below(&self, local: LocalId, depth: usize) -> bool {
        self.marked_below(local, depth, &self.stale)
    }

    /// Whether any allocation a pointer read `depth` dereferences below
    /// `local` reads through, at any level, is set in `marks`, a per-site
    /// column: [`Self::stale`] or [`Self::from_caller`].
    ///
    /// **One walk for both**, so that `r = **bb;` and `b = *bb; r = *b;` are
    /// asked about the same allocations: asked about the local's own sites
    /// alone, the first read `bb`'s allocation and never the one `*bb` names,
    /// and built in silence where the second was doubted. Found by review.
    pub(super) fn marked_below(&self, local: LocalId, depth: usize, marks: &[bool]) -> bool {
        let Some((mut sites, mut locals)) = self.level_zero(local) else {
            return false;
        };
        for _ in 0..depth {
            if sites.iter().any(|&site| marks[site]) {
                return true;
            }
            (sites, locals) = self.level_below(&sites, &locals);
        }
        false
    }

    /// What a pointer read through `local` reads through first: the
    /// allocations it holds and the locals whose address it holds.
    ///
    /// Nothing for a local this check lost, since what it holds is not named,
    /// and a load through it is answered by [`Self::stored`] for what a call
    /// reaches. No case tells that test apart from its absence, measured: a
    /// lost local's sites, read beside the marker, would only add doubts.
    /// **A local that is itself a load is followed**: what it holds is a
    /// lower bound, so what its allocations contain is one too, and every
    /// reader of it is a load read beside [`Reached::Partial`]. So a chain of
    /// loads through locals is followed, however long. See ADR-0045.
    fn level_zero(&self, local: LocalId) -> Option<(BTreeSet<usize>, BTreeSet<usize>)> {
        let held = &self.points_to[local.index()];
        if held.lost {
            return None;
        }
        Some((
            held.sites().collect(),
            self.written_through(local).into_iter().collect(),
        ))
    }

    /// One dereference further: what the allocations may contain
    /// ([`Self::inside`], [`Self::inside_locals`]) and what the locals hold,
    /// their sites and the locals their own address edges name.
    ///
    /// **A local is stepped through as an allocation is**, the read half of
    /// what a write through its address does (ADR-0019): `t2 = &slot; *t2 =
    /// p;` puts `p` in `slot`, and `*t2` reads it. A pointer to `slot`
    /// leads to what `slot` holds, at any level: `**t3` with `*t3 = &slot`,
    /// and `**t3` with `t3 = &t2; t2 = &slot`, both of which read nothing
    /// before. Each level is two finite sets, so the walk ends. The one walk
    /// for every reader, so the places a load reads and the places a mark is
    /// found cannot drift. See ADR-0045.
    fn level_below(
        &self,
        sites: &BTreeSet<usize>,
        locals: &BTreeSet<usize>,
    ) -> (BTreeSet<usize>, BTreeSet<usize>) {
        let mut next_sites = BTreeSet::new();
        let mut next_locals = BTreeSet::new();
        for &container in sites {
            next_sites.extend(self.inside[container].ones());
            next_locals.extend(self.inside_locals[container].ones());
        }
        for &held in locals {
            next_sites.extend(self.points_to[held].sites());
            next_locals.extend(self.points_to[held].writes_to.ones());
        }
        (next_sites, next_locals)
    }

    /// What a pointer read `depth` dereferences below `local` may be, and the
    /// locals whose address it may be: the walk [`Self::level_below`] takes,
    /// `depth` times from [`Self::level_zero`], with what memory this check
    /// cannot place may hold added at each step, as a load reads it. See
    /// ADR-0045.
    pub(super) fn levels_below(
        &self,
        local: LocalId,
        depth: usize,
    ) -> (BTreeSet<usize>, BTreeSet<usize>) {
        // A lost local is followed no further, except into memory this check
        // cannot place: it names no site, and what such memory holds is still
        // what is exposed.
        let (mut sites, mut locals) = match self.level_zero(local) {
            Some(level) => level,
            None if depth > 0 && !self.exposed_through(local).is_empty() => {
                (BTreeSet::new(), BTreeSet::new())
            }
            None => return (BTreeSet::new(), BTreeSet::new()),
        };
        for level in 0..depth {
            (sites, locals) = self.level_below(&sites, &locals);
            // **What memory this check cannot place may hold**, beside what
            // `Known::inside` recorded. At the first dereference, `level ==
            // 0`, only when `local` is a load or a pointer a call wrote. At
            // every later one whatever `local` is, because the pointer
            // dereferenced there was read out of memory and is a load: `q =
            // **ppp;` reads what `t = *ppp; q = *t;` reads through `t`.
            // Through `Known::inside` alone, `**ppp = m; free(m); q = **ppp;
            // return *q;` was believed at `*q` (#486). It reads more than a
            // chain of this function's own allocations can hold, a doubt the
            // reader sees, and the same one the chain written through locals
            // already gave. See ADR-0045.
            let reached: Vec<usize> = if level == 0 {
                self.exposed_through(local)
            } else {
                self.reachable_now().into_iter().collect()
            };
            // And the locals whose address that memory may hold, which a
            // store there recorded: `*a = &x;` through a load `a` puts
            // `x` where `t = *b;` through another load may read it.
            for &site in &reached {
                locals.extend(self.inside_locals[site].ones());
            }
            sites.extend(reached);
        }
        (sites, locals)
    }

    /// What a dereference of `local` may read beyond what was recorded: every
    /// exposed site, where `local` is itself a load or a pointer a call wrote,
    /// and so points into memory this check cannot place.
    ///
    /// Such memory holds anything code this check cannot read may have put
    /// there, which is what exposed says, and anything an unplaced store put
    /// there, which exposes what it carries. Without it, `*t = m; free(m); q =
    /// *t; return *q;` with `t` a load read back nothing, and the use of freed
    /// memory was believed (#484). `Held::from_caller` is not asked as well,
    /// because every producer of it sets one of these beside it.
    ///
    /// **Asked in [`Self::levels_below`], the one walk every reader of a load
    /// takes**, so that the spellings of one read cannot answer differently:
    /// a load assigned, the copy `memcpy` makes, and a place of several
    /// dereferences, `return **t;`. Asked only of the load and the copy, the
    /// first two were reported and the third was believed, found by review.
    /// See ADR-0045.
    ///
    /// **What is reachable, not only what is marked**: a store into exposed
    /// memory records what it stores and marks nothing until the next call,
    /// so what a parameter's memory holds was not read, and `t = *ppp; *w =
    /// m; free(m); q = *t; return *q;` with `f(&px, &x)` was believed (#489).
    /// See [`Self::reachable_now`].
    fn exposed_through(&self, local: LocalId) -> Vec<usize> {
        let from = &self.points_to[local.index()];
        if from.loaded || from.foreign {
            self.reachable_now().into_iter().collect()
        } else {
            Vec::new()
        }
    }

    /// What a pointer read `depth` dereferences below `local` may be: each
    /// level is what the allocations and locals of the level above may
    /// contain, from [`Self::level_zero`].
    ///
    /// **A chain written as one place is the chain written through locals**:
    /// `q = **t3` is `q2 = *t3; q = *q2;`, and each step reads [`Self::inside`]
    /// as a load through a load does, so the two spellings of a **read** cannot
    /// be answered differently, and a **store** written as one place, `**t3 =
    /// r`, is recorded where the level above may be, so it is here to be read.
    /// Every level is a lower bound read beside
    /// [`Reached::Partial`], and steps through a local whose address is
    /// stored as through an allocation ([`Self::level_below`]). See ADR-0045.
    pub(super) fn stored_below(&self, local: LocalId, depth: usize) -> BTreeSet<usize> {
        self.levels_below(local, depth).0
    }

    /// What the pointers in the memory `depth` dereferences below `local`
    /// are believed to point at, by level, index 0 for those pointers
    /// themselves: what `local` believes of its memory that far down, and
    /// what was stored in each site every level on the way may be, each
    /// taken from the level it was stored at. `depth` is at least one.
    ///
    /// **Every level on the way, and not only the last**: `int *b = **pi;`
    /// reads what `*box = m;` stored in `box`'s site one level above the
    /// memory `b` is read from, and asking only the sites `*pi` may point at
    /// read nothing there (#634).
    ///
    /// **And each local on the way, by what its own value believes**: a
    /// local whose address is gone through is memory holding that value, so
    /// it holds below its own level what the value carries. `void ***pt =
    /// &t; ... int **a = *pi;` with `t` a load out of a `void ***` read `a`
    /// believing whole `int`s two levels down, where the sites alone said
    /// nothing (#634). What the local's value points at itself is asked by
    /// the local's own fit where it is read, so its own level is no bound
    /// here. `depth_cap` is [`Allocations::deepest_pointer`](super::transfer::Allocations::deepest_pointer),
    /// which the list that puts it one level down is cut to.
    ///
    /// **The one walk a load, a departure and a copy take**, so that the
    /// spellings of one read cannot believe different things. See
    /// [`Levels`] and ADR-0060.
    pub(super) fn memory_levels(&self, local: LocalId, depth: usize, depth_cap: usize) -> Levels {
        let mut levels = self.points_to[local.index()]
            .contents_at_most
            .below(depth - 1);
        for level in 0..depth {
            let (sites, locals) = self.levels_below(local, level);
            for site in sites {
                levels.bound_by(&self.stored_at_most[site].below(depth - 1 - level));
            }
            for slot in locals {
                let held = self.points_to[slot]
                    .contents_at_most
                    .stored_under(None, depth_cap);
                levels.bound_by(&held.below(depth - 1 - level));
            }
        }
        levels
    }

    /// Record that this place was read through, where the read reaches an
    /// allocation.
    ///
    /// **A place reaching no site records nothing, and that is a size rather
    /// than a rule.** Measured: recording one anyway changes no answer, because
    /// what [`used_before`](super::report::used_before) compares is sites and an entry with none can never
    /// meet a free's. So this is skipped to keep the value small, and the
    /// asymmetry between an empty set's two readers, for whom it means
    /// opposite things, is held by the comparison rather than by this
    /// line: a dereference of a pointer this check never followed says nothing
    /// here for the same reason it says nothing in [`used`](super::report::used), which is that
    /// there is no allocation to say it about.
    ///
    /// The sites arrive ascending, because [`Held::sites`] walks a row of a
    /// table in order, and stay that way.
    ///
    /// It takes what [`dereferenced_in_element`](super::dereferenced_in_element) answers rather than one place,
    /// so that what the walk carries forwards and what [`used`](super::report::used) reports on are
    /// decided by one function with two callers. One rule written in two places
    /// drifts apart inside the change that touches one of them.
    pub(super) fn met(&mut self, read: Option<(Span, Vec<&Place>)>) {
        let Some((at, dereferenced)) = read else {
            return;
        };

        for place in dereferenced {
            let reached = self.reached_by(place.local);
            self.meeting(at, place, Read::Dereference, &reached);
        }
    }

    /// One place of one element, for [`Self::met`], or one argument of one
    /// call, for [`Allocations::terminator`](super::transfer::Allocations#method.terminator).
    ///
    /// **`reached` is the caller's**, because the two differ for one place: a
    /// dereference of `*tab` reads through `tab`'s own sites, and `*tab` handed
    /// to a call hands on what they contain ([`Self::handed_reached`]).
    pub(super) fn meeting(&mut self, at: Span, place: &Place, read: Read, reached: &[Reached]) {
        let sites: BTreeSet<usize> = named(reached).collect();

        if sites.is_empty() {
            return;
        }

        // What is reachable already, before the call this read belongs to has
        // done anything: a parameter's allocation from entry, or one stored
        // into an exposed allocation. Starting empty silences a pointer
        // parameter handed to a call before a later one.
        let reachable = self.reachable_now();
        let reachable: BTreeSet<usize> = sites
            .iter()
            .copied()
            .filter(|site| reachable.contains(site))
            .collect();

        // One entry per key: `*p = *p;` reads through one place twice at one
        // span, and two entries would be one report said twice.
        let entry = self
            .pending
            .entry(read_key(at, place, read))
            .or_insert(PendingRead {
                at,
                sites: BTreeSet::new(),
                read,
                reachable: BTreeSet::new(),
                after_call: false,
            });
        entry.sites.extend(sites);
        // A read met after an event that made one of its sites reachable
        // while a call was pending: C may run the event, the call, then this.
        // It is no call's argument and the event's, both walked before it.
        if entry
            .sites
            .iter()
            .any(|site| self.exposed_after_call.contains(site))
        {
            entry.after_call = true;
        }
        entry.reachable.extend(reachable);
    }

    /// Every site code this check cannot read may reach now.
    ///
    /// **The closure, and not [`Self::exposed`] read as it stands.** That is
    /// closed only inside [`Self::expose`], which runs at a call, so a store of
    /// `a` into an allocation already exposed records `a` in [`Self::inside`]
    /// and leaves its bit unset until the next call. Its readers run after
    /// that call and are right to trust it; a pending read is recorded, and
    /// told what became reachable, between calls.
    pub(super) fn reachable_now(&self) -> BTreeSet<usize> {
        self.closure(
            (0..self.exposed.len())
                .filter(|&site| self.exposed[site])
                .collect(),
        )
    }

    /// `sites`, and everything a pointer stored in one of them may hold, and
    /// so on.
    pub(super) fn closure(&self, sites: Vec<usize>) -> BTreeSet<usize> {
        let mut closed: BTreeSet<usize> = sites.iter().copied().collect();
        let mut pending = sites;
        while let Some(site) = pending.pop() {
            for held in self.inside[site].ones() {
                if closed.insert(held) {
                    pending.push(held);
                }
            }
        }
        closed
    }

    /// Record that a store this check cannot place may have put `carried`
    /// in any memory code it cannot read can name: every exposed allocation
    /// and every allocation reachable from one, since a pointer this check
    /// cannot place may point at any of them.
    ///
    /// Reachable rather than exposed, because a store into exposed memory
    /// records what it stores there and does not mark it: `*a = t;` leaves
    /// `t` unmarked. One function for the store and for the copy `memcpy`
    /// makes, so the two spellings cannot land in different places. See
    /// ADR-0044.
    ///
    /// **And the locals whose address it carries**, in what each landing may
    /// contain, which is not exposing them: a load out of that memory reaches
    /// the local, and a write through the load lands in it. `*ppp = &x; t =
    /// *r; *t = m; free(m); q = x; return *q;` under `f(&a, &a)` read `x` as
    /// the null it started as and was believed, found by review. ADR-0047
    /// keeps a local's address out of what is exposed, so that a holder is
    /// still asked; this records where it may be and exempts nobody.
    pub(super) fn landed_where_unknown_code_points(&mut self, carried: &[usize], locals: &[usize]) {
        for landing in self.reachable_now() {
            for &site in carried {
                self.inside[landing].set(site);
            }
            for &local in locals {
                self.inside_locals[landing].set(local);
            }
        }
    }

    /// Record that a call this check cannot read was handed the address of
    /// each of `locals`: every local whose address it can find from them, at
    /// any depth, is [`Self::exposed_locals`] from here on.
    ///
    /// **From everything it can reach, not only what it was handed**: every
    /// allocation [`Self::reachable_now`] names and every local already
    /// exposed, since a callee
    /// may keep an address and use it at a later call, `stash(m); tick();`,
    /// and a store into memory already handed over records the address there
    /// for the next call to find, `stash_box(box); *box = m; tick();`. Then
    /// [`Self::level_below`], the walk every load takes, until neither set
    /// grows: through what an allocation contains, and through what a local
    /// holds, since a callee handed `&p` can read `p` and write through it.
    /// Two finite sets that only grow, so it ends. Not [`Self::beneath`],
    /// which follows locals alone: an address stored in an allocation is
    /// found only by walking the sites beside them. See ADR-0057.
    ///
    /// **After [`Self::expose`], at the same call**, which is what puts the
    /// allocations the call was handed into `reachable_now`: asked before it,
    /// `*box = m; fill_box(box);` left `m` unwritten.
    pub(super) fn expose_locals(&mut self, locals: Vec<usize>) {
        let mut seeds: BTreeSet<usize> = locals.into_iter().collect();
        seeds.extend((0..self.exposed_locals.len()).filter(|&local| self.exposed_locals[local]));
        for local in self.locals_closed_from(self.reachable_now(), seeds) {
            self.exposed_locals[local] = true;
        }
    }

    /// Every local code this check cannot read may name now: the walk
    /// [`Self::expose_locals`] takes, from the same seeds, without marking
    /// anything. What a pointer such code gave this function, or memory it
    /// can reach, may hold. **Now, not when the pointer was read**: an
    /// address stored there after a pointer was read out of that memory is
    /// counted as one the pointer may be, which silences a read of a local
    /// array the pointer could not reach (#648). See ADR-0057.
    pub(super) fn named_by_unknown_code(&self) -> BTreeSet<usize> {
        let exposed = (0..self.exposed_locals.len()).filter(|&local| self.exposed_locals[local]);
        self.locals_closed_from(self.reachable_now(), exposed.collect())
    }

    /// `locals`, and every local whose address may be found from them or
    /// from `sites` through [`Self::level_below`], at any depth. One walk for
    /// the call that exposes and the write that asks, so the two cannot find
    /// different locals. Two finite sets that only grow, so it ends.
    fn locals_closed_from(
        &self,
        mut sites: BTreeSet<usize>,
        mut locals: BTreeSet<usize>,
    ) -> BTreeSet<usize> {
        loop {
            let (next_sites, next_locals) = self.level_below(&sites, &locals);
            let before = (sites.len(), locals.len());
            sites.extend(next_sites);
            locals.extend(next_locals);
            if (sites.len(), locals.len()) == before {
                return locals;
            }
        }
    }

    /// Every local a write this check cannot place may land in, for
    /// [`Self::unwritten`], where the write goes through `dereferences`
    /// dereferences of `local`:
    ///
    /// - at none, `local` itself, `m[0] = p`;
    /// - at one, the locals `local`'s address edges name, `*t = x`;
    /// - at more, the locals the level above may be, `**ppq = x`;
    /// - and every local code this check cannot read may name,
    ///   [`Self::named_by_unknown_code`], where `local` or a pointer the walk
    ///   passes through on the way down was read out of memory, lost, or
    ///   written by such code, since each of those may hold an address this
    ///   function never recorded.
    ///
    /// A store passes its place's [`derefs`], which is zero for a place that
    /// is not dereferences alone; a library copy passes one more than its
    /// destination argument has, since it writes where that argument points.
    ///
    /// **Only a level the walk passes through decides the last part**, not
    /// how deep the write is: `memcpy(*pp, s, 8)` with `*pp` certainly `a`
    /// cannot land in an array whose address was stored in the caller's
    /// memory, and counting it so silenced a read of that array `main`
    /// reported. Found by review.
    ///
    /// **Two parts no case holds, measured.** A place with no dereference is
    /// never a store from C: the lowering writes `m[0] = p` through `&m[0]`,
    /// and builds an `Index` only under an address, so landing nowhere at
    /// zero leaves the suite green. And the pointers the last part is for
    /// already carry in their edges the addresses that memory such code
    /// reaches was recorded to hold, so dropping it leaves the suite green
    /// too; it stands for an address that recording misses, where its
    /// absence would be a false report.
    ///
    /// **A pointer naming no local lands in none.** One moved off an array by
    /// arithmetic has lost its edge (#639), so `*(&m[1] - 1) = p` leaves
    /// `m[0]` reported as nothing wrote it. That report is certain and false:
    /// `--allow-unknown` does not lower it, and a free of what the array
    /// holds is not doubted after it. ADR-0057 records why it is kept, and
    /// #647 what would remove it. The store door and a library copy both ask
    /// this, so the two cannot answer one pointer differently.
    pub(super) fn locals_a_write_may_land_in(
        &self,
        local: LocalId,
        dereferences: usize,
    ) -> Vec<usize> {
        let mut lands: BTreeSet<usize> = match dereferences {
            0 => BTreeSet::from([local.index()]),
            1 => self.written_through(local).into_iter().collect(),
            _ => self.levels_below(local, dereferences - 1).1,
        };
        if self.through_unnamed(local, dereferences) {
            lands.extend(self.named_by_unknown_code());
        }
        lands.into_iter().collect()
    }

    /// Whether a write through `dereferences` dereferences of `local` goes
    /// through a pointer that may hold an address this function never
    /// recorded: `local` itself, or a level the write reads its way through
    /// above the one written, read out of memory, lost, or written by code
    /// this check cannot read. A level that is an allocation counts as
    /// memory. One answer for where a write may land and for whether it
    /// leaves this function's sight, so the two cannot drift. See ADR-0057
    /// and ADR-0041.
    pub(super) fn through_unnamed(&self, local: LocalId, dereferences: usize) -> bool {
        let unnamed = |held: &Held| held.loaded || held.lost || held.foreign;
        unnamed(&self.points_to[local.index()])
            || (0..dereferences.saturating_sub(1)).any(|level| {
                let (sites, locals) = self.levels_below(local, level);
                !sites.is_empty() || locals.iter().any(|&held| unnamed(&self.points_to[held]))
            })
    }

    /// Tell every pending read that `by` did not make that `routes` are now
    /// reachable to code this check cannot read.
    ///
    /// Called where something makes a site reachable, a call or a store, so
    /// that a read which ran before it remembers: C leaves the two
    /// unsequenced, so the one that made it reachable may have run first.
    /// `by` is the call doing it, and a read it owns is not told, because a
    /// call's arguments are ordered before its body. See
    /// [`PendingRead::reachable`].
    ///
    /// **What this event reaches, and not what is exposed by now.** A site the
    /// read's own call exposed is exposed from then on, so telling each read
    /// everything exposed at each later event told it about its own call, and
    /// `(keep(a) != 0) + release_all()` was refused. An event's routes are
    /// its own whether or not the site was exposed already, so a second route
    /// to a site the own call exposed is still told.
    pub(super) fn noticed(
        &mut self,
        sources: &SourceMap,
        by: Option<Span>,
        routes: &BTreeSet<usize>,
    ) {
        // **And whether a call already pending may run between this and the
        // read.** It may unless the read belongs to it or to `by`, or it is
        // in `by`'s arguments: C17 6.5.2.2 p10 orders a call's arguments
        // before its body, so none of those can come after `by` and before
        // the read. See [`PendingRead::after_call`].
        let calls: Vec<Span> = self.calls.values().copied().collect();
        if calls
            .iter()
            .any(|&call| by != Some(call) && !by.is_some_and(|by| inside(sources, call, by)))
        {
            self.exposed_after_call.extend(routes);
        }
        for entry in self.pending.values_mut() {
            let own_by = by.is_some_and(|by| entry.at == by || inside(sources, entry.at, by));
            if !own_by && entry.sites.iter().any(|site| routes.contains(site)) {
                let freer = calls.iter().any(|&call| {
                    by != Some(call)
                        && !(entry.at == call || inside(sources, entry.at, call))
                        && !by.is_some_and(|by| inside(sources, call, by))
                });
                if freer {
                    entry.after_call = true;
                }
            }
        }
        for entry in self.pending.values_mut() {
            // Its own if it is the pointer the call was handed, whose span is
            // the call's, or a dereference written in its arguments, whose
            // span is inside it. See ADR-0043 for why inside means that.
            let own = by.is_some_and(|by| entry.at == by || inside(sources, entry.at, by));
            if own {
                continue;
            }
            let now: Vec<usize> = entry
                .sites
                .iter()
                .copied()
                .filter(|site| routes.contains(site))
                .collect();
            entry.reachable.extend(now);
        }
    }

    /// Every local a write through this one may land in.
    ///
    /// Empty means this check does not know where such a write goes, which is
    /// not the same as knowing it goes nowhere. ADR-0019 records why the answer
    /// to not knowing is to do nothing.
    pub(super) fn written_through(&self, local: LocalId) -> Vec<usize> {
        (0..self.points_to.len())
            .filter(|target| self.points_to[local.index()].writes_to.get(*target))
            .collect()
    }

    /// Every local reachable from `from` through [`Held::writes_to`], at any
    /// depth, `from` included: what a callee handed the address of one of
    /// them can reach by following addresses. A visited bit per local, so a
    /// cycle of addresses or a dense graph of them is walked once each,
    /// rather than once per path. See ADR-0044.
    pub(super) fn beneath(&self, from: impl IntoIterator<Item = usize>) -> Vec<usize> {
        let mut seen = vec![false; self.points_to.len()];
        let mut todo: Vec<usize> = from.into_iter().collect();
        let mut found = Vec::new();
        while let Some(local) = todo.pop() {
            if std::mem::replace(&mut seen[local], true) {
                continue;
            }
            found.push(local);
            let held = &self.points_to[local];
            todo.extend(held.writes_to.ones());
        }
        found
    }

    /// Hand this site to a new allocation.
    ///
    /// **A site names one allocation at a time, and this is where it stops
    /// naming the last one.** A site is a local, so a loop through the same
    /// call files each allocation it makes under one name. Every other local
    /// that still pointed here was following the allocation an earlier turn
    /// made, and `state` below is about the one this call just made: handing
    /// them that proof reported the wrong line and was silent about the right
    /// one. See ADR-0018, which has the program.
    ///
    /// The destination is excluded because the caller has already rebuilt its
    /// row, and the site it holds now is this allocation rather than the one
    /// being displaced.
    ///
    /// **Every field named, never `..`.** A fact filed against a site is a
    /// fact about whatever the site named when it was written, so a field
    /// added to [`Known`] has to answer here for what happens when the site
    /// starts naming something else. `error[E0027]` is what asks, as it does
    /// for a variant's fields wherever a match does not write `..`.
    pub(super) fn reborn(&mut self, site: usize, made: Option<Span>) {
        let Known {
            points_to,
            state,
            escaped: _,
            // A local's, as `escaped` is, and a site being reborn is not.
            handed_away: _,
            exposed,
            inside,
            // Kept, as `inside` is: what the old allocation may hold stays
            // in the row. See ADR-0045.
            inside_locals: _,
            realloced,
            stale,
            // Kept, as `stale` is: the slots of the new allocation are not
            // told apart from the old one's.
            unreplaced: _,
            // Kept, as `stale` is: the slots of the new allocation are not
            // told apart from the old one's.
            holds_lost: _,
            // Not fresh, whatever it was: the caller says whether the new
            // allocation is, since only it knows which function made it.
            fresh,
            // Kept, as the entry and `stale` are: the slots of the new
            // allocation are not told apart from the old one's. No case
            // tells this from clearing it, measured: every other local
            // holding the site is lost a few lines below, so a load out of
            // it is doubted either way.
            from_caller: _,
            pending,
            calls: _,
            exposed_after_call,
            // About no site.
            lost_exposed: _,
            // Left to the caller, which writes what the new allocation takes
            // on the line after this, since only it knows what made it.
            extent: _,
            // Kept, as `inside` is: the slots of the new allocation are not
            // told apart from the old one's, and a bound kept only doubts.
            stored_at_most: _,
            // About a local, not a site.
            handed_at_most: _,
            // About a local as well: the call that makes the site writes the
            // local it is named by, which the call's destination says.
            unwritten: _,
            // A local's, as `escaped` is, and a site being reborn is not.
            exposed_locals: _,
        } = self;

        // **A local that loses an allocation that is gone may hold one**, so
        // a store of it marks where it lands, as a load out of a marked
        // allocation does. Not one that loses a live allocation: a list built
        // in a loop loses its head every turn to a node nothing freed, and
        // marking that doubted every walk of it. See ADR-0045.
        // **What a `realloc` remembered about this site goes**, whether the
        // site is the allocation it returned or one it was handed: the name
        // now belongs to a new allocation, and the branch that would have
        // settled the old fact is about another. See ADR-0039.
        realloced[site] = None;
        fresh[site] = false;
        for fact in realloced.iter_mut() {
            if fact
                .as_ref()
                .is_some_and(|fact| fact.old.iter().any(|&(old, _)| old == site))
            {
                *fact = None;
            }
        }
        // **`Reachable` is gone here**, as every doubt is: a call that may have
        // freed the old allocation through what it reached writes that, and
        // counting it live let a pointer stored last turn be read as the new
        // allocation. Found by review. See ADR-0047.
        let gone = !matches!(state[site], SiteState::Live(_));
        for (other, held) in points_to.iter_mut().enumerate() {
            if other != site {
                if gone && held.sites.get(site) {
                    held.stale_read = true;
                }
                held.lose(site);
            }
        }

        // **What other allocations contain of this site is marked, when the
        // allocation it named is gone.** The entry now names the new one,
        // which is live, so a pointer stored last turn and freed was read as
        // live; the mark makes a load out of the container one this check
        // stopped following, which is the column's `lose` above. Only when
        // gone: while the old allocation is live a read of it is not a use
        // after free, and marking it doubted a loop that keeps last turn's
        // allocation, measured. See ADR-0045.
        //
        // **The entry itself is kept.** `Unknown` is gone on one path only,
        // and what a call handed the container reaches is read off the entry:
        // dropping it left a pointer the old allocation held live across a
        // call that may free it, which built where `main` refused it. Found by
        // review; `a_call_handed_a_container_of_a_reborn_site_reaches_what_it_held`.
        for container in 0..inside.len() {
            if gone && inside[container].get(site) {
                stale[container] = true;
            }
        }
        state[site] = SiteState::Live(made);
        // A new allocation has not been handed to anybody. What an opaque
        // call's destination may still be is the call transfer's to say,
        // after this. See ADR-0039.
        //
        // **What the old allocation held stays in the row**, though the new
        // one holds nothing yet. The old one is still out there, held by a
        // local that has just lost its name for it or by another allocation,
        // and a pointer read out of it reaches what it held: `release(old);
        // return *p;` with `*old = p` stored last turn built in silence while
        // this row was cleared. The row is a may-set, so a stale entry costs
        // a report and never a proof. See ADR-0040. The column is the other
        // half, below the state's own write.
        exposed[site] = false;

        // **A read of the allocation this site used to name is not a read of
        // the one it names now.** Left standing, a free of the new allocation
        // later in the same full expression would be reported against a read
        // of the old one, which is a caret on a line that read something else.
        // No C program reaches this, because two calls in one full expression
        // are two call sites and a site is the local a call writes into; a
        // frontend whose calls share one can, and this is the answer
        // `error[E0027]` asked for when the field was added. `reachable`
        // goes with `sites`, and for the same reason no case holds it:
        // dropping its line leaves the whole workspace green.
        for entry in pending.values_mut() {
            entry.sites.remove(&site);
            entry.reachable.remove(&site);
        }
        // The exposure was of the allocation this site used to name.
        exposed_after_call.remove(&site);
        pending.retain(|_, entry| !entry.sites.is_empty());
    }

    /// Nothing this local holds is proved, once anything holds its address.
    ///
    /// Called wherever a local is *given* something, so that the escape
    /// outlives the sites it was recorded against. Towards `Unknown` and never
    /// away from it, which is why getting this wrong costs a false positive
    /// rather than a silence: a may-set cannot prove anything from one member
    /// and this only ever takes proof away.
    ///
    /// **What a local is given, and not what is done to what it holds.** A
    /// `free` writes `SiteState::Freed` over a site whatever the local's
    /// escape says, so `int **pp = &p; free(p); *pp = q; *p = 1;` is a proved
    /// `Unsafe` about a program with no defect in it. That is not this rule
    /// arriving late, it is the escape and the free recorded in one slot and
    /// overwriting each other; it predates this and is issue #161.
    ///
    /// **This is only half of what an escape means, and the half about the
    /// heap.** What it means about the local's own set is answered at the
    /// report by [`Self::reached_by`], which is why nothing here is about
    /// whether the escaped local itself can be trusted. See ADR-0017.
    ///
    /// An index rather than a [`LocalId`], because [`Self::settle`] walks the
    /// rows of a side table and `LocalId` cannot be built from one.
    pub(super) fn unproved(&mut self, local: usize) {
        if !self.escaped[local] {
            return;
        }

        // **A live allocation, and not a freed one.** Whoever holds the
        // address may free what the local holds, which is a doubt about an
        // allocation still live; one already freed stays freed, as no call
        // un-frees one. Making it `Unknown` too forgot the free, so `free(a);
        // use2(&a);` could not be told from a live `a` handed by address. See
        // ADR-0017. And it is `Reachable` rather than `Unknown`, since nothing
        // here may have freed it yet, which is what a call by address asks.
        // See ADR-0047.
        for site in self.points_to[local].sites() {
            self.state[site].doubted();
        }
    }

    /// Code this check cannot read may reach `sites`, and so everything a
    /// pointer stored in any exposed allocation may hold, and so on.
    ///
    /// **The closure is taken over every exposed site, not only the ones this
    /// call marks.** A join can leave an allocation exposed on one arm and
    /// holding a pointer on the other, and after it the pair is exposed and
    /// holds the pointer while the pointer is not exposed; a closure that
    /// started from the new marks alone left that pointer proved after the
    /// next call. Every reader of [`Self::exposed`] runs after this, so this
    /// is the one place the invariant has to hold: an invariant on a lattice
    /// value has to hold after the join. See ADR-0039.
    ///
    /// `by` is the call exposing them, or `None` for anything else, and says
    /// which pending reads are told. See [`Self::noticed`].
    pub(super) fn expose(
        &mut self,
        sources: &SourceMap,
        sites: impl IntoIterator<Item = usize>,
        by: Option<Span>,
    ) {
        let sites: Vec<usize> = sites.into_iter().collect();
        let routes = self.closure(sites.clone());
        self.noticed(sources, by, &routes);
        for site in sites {
            self.exposed[site] = true;
            self.written(site);
        }
        // One closure, shared with what a pending read is told, so that the
        // two cannot disagree about what an exposed allocation reaches.
        for site in self.reachable_now() {
            self.exposed[site] = true;
            self.written(site);
        }
    }

    /// Whether `local` holds a pointer read out of fresh memory that nothing
    /// may have replaced: [`Held::fresh_read`](super::parts::Held::fresh_read),
    /// and not a local whose address escaped, since code this check cannot
    /// read may have written a determinate pointer into it since. The one
    /// answer a copy of it and the suppression at a free ask, so that the
    /// doubt a free of what `fill(&t)` wrote raises is kept. See ADR-0057.
    pub(super) fn holds_fresh_read(&self, local: usize) -> bool {
        self.points_to[local].fresh_read && !self.escaped[local]
    }

    /// Something may have written `site`, so its contents are no longer
    /// known to be indeterminate. Exposed memory is written by whatever code
    /// this check cannot read may do with it. See [`Self::fresh`].
    pub(super) fn written(&mut self, site: usize) {
        self.fresh[site] = false;
    }

    /// Something may have written any allocation: a write this check cannot
    /// place, or a library call it reads only by name. See [`Self::fresh`].
    pub(super) fn written_all(&mut self) {
        self.fresh.fill(false);
    }

    /// Something may have written `local`, so its value is no longer known
    /// to be indeterminate. See [`Self::unwritten`].
    pub(super) fn written_local(&mut self, local: usize) {
        self.unwritten[local] = false;
    }

    /// Every exposed allocation still live is unproven, because the call being
    /// made may free it. A proved free stays proved: no call un-frees an
    /// allocation. See ADR-0039. Unproven as [`SiteState::Reachable`], since
    /// the call reaches it only through what was exposed. See ADR-0047.
    pub(super) fn unproved_exposed(&mut self) {
        // A call that may free what is exposed may free an old allocation a
        // `realloc` remembered, so the fact goes too. See ADR-0039.
        let exposed = self.exposed.clone();
        self.forget_reallocs_touching(|site| exposed[site]);
        for (state, &exposed) in self.state.iter_mut().zip(&self.exposed) {
            if exposed {
                state.doubted();
            }
        }
    }

    /// A call this check cannot read may have freed what it reached only
    /// through an address, and cannot have replaced a holder whose address it
    /// cannot reach: so a site still [`SiteState::Reachable`] that such a
    /// local holds, and still reads after the call, is `Unknown`.
    ///
    /// `b = a; release_ref(&b); use2(&a);` is the program. The call may free
    /// the allocation through `b` and put a new one there, and cannot put
    /// anything in `a`, so `use2(&a)` may be handed a dangling pointer. A
    /// holder whose address code this check cannot read certainly holds, by
    /// [`Self::handed_away`], is one the call may have replaced, so it leaves
    /// the site as it was. Not [`Self::escaped`]: `int **pa = &a;` sets that
    /// with nothing outside the function able to see `pa`, and exempting `a`
    /// for it was silent about the program above with that line added.
    ///
    /// **Only a holder live after the call**, by `live_after`. A copy nothing
    /// reads again holds the site as well, and counting it refused `b = a;
    /// grow(&b); grow(&b);` and `b = a; grow(&a); use2(&a);`, which C
    /// defines.
    ///
    /// **And a slot in memory holding it marks the allocation it is in.** The
    /// call cannot replace a slot it did not reach either, and a slot has no
    /// liveness to say whether it is read again, so turning the site
    /// `Unknown` for one refused `*h = a; grow(&a); use2(&a);` with `*h`
    /// never read. Each allocation that may contain the site is marked
    /// [`Self::unreplaced`] instead, and a pointer later read out of it is
    /// asked about where it is handed by address, which
    /// [`Self::handed_below`] does. Every such allocation, not only those the
    /// call did not reach: a load out of one it reached is doubted already.
    /// Only for a site this leaves `Reachable`, since a site `Unknown` or
    /// freed is asked about whoever holds it. See ADR-0047.
    pub(super) fn held_out_of_reach(&mut self, live_after: impl Fn(usize) -> bool) {
        for site in 0..self.state.len() {
            if self.state[site] != SiteState::Reachable {
                continue;
            }
            for container in 0..self.inside.len() {
                if self.inside[container].get(site) {
                    self.unreplaced[container] = true;
                }
            }
            let out_of_reach = (0..self.points_to.len()).any(|holder| {
                !self.handed_away[holder]
                    && live_after(holder)
                    && self.points_to[holder].sites().any(|held| held == site)
            });
            if out_of_reach {
                self.state[site].may_be_freed();
            }
        }
    }

    /// Record that a call this check cannot read was handed `handed`: the
    /// local whose address an argument certainly carries is
    /// [`Self::handed_away`] from here on.
    ///
    /// **Only a local the argument certainly names.** The fact exempts a
    /// holder, so a local the argument may only be the address of is one the
    /// call may never have been handed: with `if (c) q = &a; else q = &b;`,
    /// or `*k = &a; *k = &b;`, marking both exempted `a` after `grow(q)` or
    /// `grow(*k)` and `use2(&a)` built over a pointer `grow` may have freed
    /// through `b`. So a plain local counts where its address edges are one
    /// local and all of what it may point at ([`Held::certain_target`]),
    /// and a place of dereferences never does:
    /// what memory holds is a lower bound, so one local read there is not
    /// one local certainly. Every route missed leaves a holder counted by
    /// [`Self::held_out_of_reach`], which costs a report rather than a
    /// silence: `grow(*k); grow(*k);` is asked at the second call.
    ///
    /// **Not what memory the call can reach may hold**, for the same reason:
    /// what a slot was given stays in [`Self::inside_locals`] after the slot
    /// is given something else, so `*t = &a; *t = 0; stash(t);` exempted `a`
    /// though `stash` saw only null. The in-out idiom through memory a call
    /// can reach is asked for it. See ADR-0047.
    pub(super) fn handed_to_a_call(&mut self, handed: &[Operand]) {
        for argument in handed {
            let Operand::Copy(place) = argument else {
                continue;
            };
            if !place.projection.is_empty() {
                continue;
            }
            if let Some(target) = self.points_to[place.local.index()].certain_target() {
                self.handed_away[target] = true;
            }
        }
    }

    /// Forget every `realloc` fact naming an old allocation `touched` says
    /// something else may have freed since, so a branch on the result can no
    /// longer give it back as live. See ADR-0039.
    pub(super) fn forget_reallocs_touching(&mut self, touched: impl Fn(usize) -> bool) {
        for fact in self.realloced.iter_mut() {
            if fact
                .as_ref()
                .is_some_and(|fact| fact.old.iter().any(|&(old, _)| touched(old)))
            {
                *fact = None;
            }
        }
    }

    /// What a free of a value this check lost does to every other lost
    /// value: any of them may be the one that went.
    ///
    /// **Every one, because none can be told from another.** A lost value
    /// names no site, so a copy of it made before the free, `w = u; free(u);
    /// *box = w;`, and an allocation it was stored in before the free,
    /// `*box = u; free(u);`, are no more or less likely to be what went than
    /// the local handed to `free`. So every lost local carries
    /// [`Held::stale_read`] from here, and a store of it marks where it
    /// lands, and every allocation [`Self::holds_lost`] marks is `stale`, so
    /// a load out of it is one this check stopped following. Marking too much
    /// is a doubt; marking only the local handed over left the other three
    /// spellings read in silence (#503). See ADR-0045.
    pub(super) fn lost_may_be_gone(&mut self) {
        for held in self.points_to.iter_mut() {
            // A value read out of memory a lost value may be in may be it.
            // See `Held::lost_read`.
            if held.lost_read {
                held.lost = true;
            }
            if held.lost {
                held.stale_read = true;
            }
        }
        for (stale, &holds) in self.stale.iter_mut().zip(&self.holds_lost) {
            if holds {
                *stale = true;
            }
        }
    }

    /// What a call that may free the caller's memory does to it: a call this
    /// check cannot read, or a free of a pointer that may be the caller's.
    ///
    /// **What the caller stored is out of sight from here on.** A pointer
    /// read out of memory a parameter points at may be one the caller stashed
    /// where this call frees it, and no site names it for the call to reach,
    /// so it is one this check stopped following, and may be gone (ADR-0045).
    /// **And so is what this function stored of it**, so a load out of such
    /// an allocation afterwards is one too, whether or not the call can reach
    /// the allocation: what it may free is the caller's. **And every exposed
    /// allocation still live is unproven**, since the caller can see it. One
    /// function for both callers, so the two cannot answer differently. See
    /// ADR-0040.
    pub(super) fn callers_memory_may_be_freed(&mut self) {
        for held in self.points_to.iter_mut() {
            if held.from_caller {
                held.lost = true;
                held.stale_read = true;
            }
        }
        for (stale, &from_caller) in self.stale.iter_mut().zip(&self.from_caller) {
            if from_caller {
                *stale = true;
            }
        }
        self.unproved_exposed();
    }

    /// Every site a call could reach: what its arguments name, and what every
    /// local whose address escaped holds, since the call may have been handed
    /// that address now, earlier, or by another route. A local an argument
    /// points at is one of those: its address was taken to point at it. See
    /// ADR-0039.
    ///
    /// **An escaped local that may hold a pointer read out of memory, or one
    /// it lost, reaches what is stored**, since a callee handed its address can
    /// read that pointer out of it and nothing here names what it is. See
    /// ADR-0040.
    pub(super) fn reach_of(&self, named: impl Iterator<Item = usize>) -> Vec<usize> {
        let mut reach: Vec<usize> = named.collect();
        let mut unnamed = false;
        for (local, &escaped) in self.escaped.iter().enumerate() {
            if escaped {
                reach.extend(self.points_to[local].sites());
                unnamed |= self.points_to[local].loaded || self.points_to[local].lost;
            }
        }
        if unnamed {
            reach.extend(self.stored());
        }
        reach
    }

    /// Per site, whether anything in this value may still name it: a local's
    /// sites, what an allocation contains, an exposure, a call left pending
    /// beside one, and a read carried forwards.
    /// Every site, where a local was written by a call this check cannot
    /// read, or is lost.
    ///
    /// **It over-counts, and must**: a site answered as held by nothing takes
    /// its state from the other side of a join, so one held by something
    /// missed here is one whose free could be forgotten. See ADR-0048.
    pub(super) fn held_sites(&self) -> Vec<bool> {
        let sites = self.state.len();
        // **Every holder stays, whatever a search found** (#481): one left
        // out that some program needs is a free forgotten in silence. Three
        // fail no case when left out, measured on the whole workspace, and
        // each says below what is believed to cover it and which programs
        // were tried: `exposed_after_call`, and a pending read's `sites` and
        // `reachable`. This early return is told apart in both directions.
        // A `realloc` fact's `old` is not a holder, by argument rather than
        // by search (#494): the fact acts on a site only at a branch on what
        // the `realloc` returned, and a site nothing here holds can never be
        // named again, so its state on that arm decides no answer. Holding
        // it doubted a use and a free of `p` after `if (c) p = realloc(p,
        // 8);`. Mutation: hold it again;
        // `a_pointer_reallocated_on_one_arm_is_not_doubted_after_its_null_test`
        // reports. What that asks of the join is in `Allocations::join`.
        //
        // **This early return is for a pointer a call wrote, not for a load
        // or a lost local** (#495). A use is read from the site's state,
        // and on the arm that freed and nulled `q` nothing holds `q`'s site,
        // so without this it is `Unheld` and takes the other arm's `Live`.
        // For a pointer a call wrote this is where the doubt comes from: the
        // first level of a use never asks `foreign` (ADR-0017). Mutation:
        // leave it out;
        // `a_use_through_a_pointer_a_call_wrote_on_one_arm_is_doubted` goes
        // quiet. A lost value read back out of memory is not here: #501, #503
        // and #505 mark it gone at every `free`, `realloc` and call this check
        // cannot read that may release one, so the return held every site
        // for nothing and doubted a free in a loop beside a local lost to its
        // rebirth (#517). Mutation: ask `lost` again;
        // `a_free_in_a_loop_is_not_doubted_beside_a_local_lost_to_its_rebirth`
        // reports at the free. A load is neither, and holding every site
        // beside one doubted `if (c) { free(p); p = 0; }` beside any
        // unrelated load. Mutation: ask `loaded` again;
        // `a_pointer_freed_and_nulled_on_one_arm_is_not_doubted_beside_a_load`
        // reports. The cost that stays: one written local anywhere holds
        // every site, so the same idiom is doubted beside `get(&u)`,
        // `a_pointer_freed_and_nulled_on_one_arm_is_doubted_beside_a_pointer_a_call_wrote`.
        // See ADR-0048.
        if self.points_to.iter().any(|held| held.foreign) {
            return vec![true; sites];
        }
        let mut held = vec![false; sites];
        for local in &self.points_to {
            for site in local.sites() {
                held[site] = true;
            }
        }
        for contents in &self.inside {
            for site in contents.ones() {
                held[site] = true;
            }
        }
        // `exposed_after_call` can be all that holds a site at an edge: in
        // `release_all() + (c ? (e = a, e = 0, 1) : 0)` with `e`'s address
        // taken, the arm stores `a` in `e` and overwrites it before the join.
        // Then nothing that can name the site after the join holds it on that
        // arm, so its state there decides no answer. Tried, with no answer
        // changing: that program, and the same with `&&` in place of `?:`.
        for (site, held) in held.iter_mut().enumerate() {
            *held |= self.exposed[site] || self.exposed_after_call.contains(&site);
        }
        // A pending read's `sites` can be all that holds a site at an edge:
        // `(x = *a) + (c ? consume(a) : 0)`, with the read lowered before the
        // branch and `a` read no more. A free or a call that crosses the read
        // tells it when it runs, which is believed to leave nothing for the
        // site's state at the join to decide. Tried, with no answer
        // changing: that program, and the same with `free(a)` in the arm.
        // Its `reachable` was never all that held a site in any program
        // tried.
        for read in self.pending.values() {
            for &site in read.sites.iter().chain(&read.reachable) {
                held[site] = true;
            }
        }
        held
    }

    /// Every site a pointer stored in some allocation may hold.
    ///
    /// What a pointer read out of memory this check cannot say which may be:
    /// wherever it was read from, it was stored there, and a store into an
    /// allocation is recorded in [`Self::inside`] or exposed at once, and the
    /// exposed half is read by [`Self::levels_below`] rather than here, for a
    /// load through memory this check cannot place (#484). One read
    /// out of a local through its address is not here and need not be: that
    /// local escaped, so what it holds is in every call's reach already. See
    /// ADR-0040.
    pub(super) fn stored(&self) -> Vec<usize> {
        (0..self.inside.len())
            .filter(|&held| self.inside.iter().any(|row| row.get(held)))
            .collect()
    }

    /// [`Self::unproved`] for every local at once.
    ///
    /// **A join gives a local sites without assigning to it.** An arm that
    /// took the address and an arm that allocated meet here, and the merged
    /// value held an escaped local reaching a site this check had proved live:
    /// `if (c) { pp = &p; *pp = q; } else { p = malloc(8); } free(q); free(p);`
    /// was silent at `--safety strict` on a double free. The
    /// union of `escaped` alone does not do it, because nothing downstream of
    /// a join reads the bit unless the local is written again.
    ///
    /// **This is why the list of places is closed rather than long.** A value
    /// changes in `Analysis::on_entry`, `join`, `element` and `terminator` and
    /// nowhere else, so covering the last three covers every one: nothing has
    /// escaped where a function starts.
    pub(super) fn settle(&mut self) {
        for local in 0..self.escaped.len() {
            self.unproved(local);
        }
    }

    /// Everything an escaped local this writer could have reached holds may
    /// have been replaced.
    ///
    /// A writer this check cannot follow may write through any address that
    /// has escaped, and nothing here can say which local that reaches: the
    /// address may have been stashed anywhere on the way. So every escaped
    /// local it could have reached holds something this check cannot name,
    /// which is what [`Held::lost`] says and is ADR-0018's fact through a
    /// second door.
    ///
    /// **Which locals that is belongs to the caller**, because the two writers
    /// know different amounts. A call this check cannot read may write any
    /// type, since nothing here reads a callee's body, and answers `true` for
    /// every local; a write through a pointer writes one type and says so, and
    /// ADR-0031 is why that narrowing is the difference between a rule that
    /// costs what it should and one that costs everything. Deciding it here
    /// instead would make one of the two answer a question written for the
    /// other, and one judgement point reached by two checks inherits a rule
    /// written for one of them.
    ///
    /// **The bit alone is not the point.** [`Self::reached_by`] already
    /// answers [`Reached::Lost`] for an escaped local that holds a site, so no
    /// report about the local itself moves. What moves is what a later `free`
    /// of it is entitled to write on the sites, which are shared with every
    /// other local holding them. See ADR-0029.
    ///
    /// **A local holding no site is left alone, and that is the reason the
    /// condition is not just the escape.** Its `lost` bit is read with no
    /// emptiness condition, so marking it would answer [`Reached::Lost`] for a
    /// pointer this check never followed: `int *p; get(&p); *p = 1;` is the
    /// output-parameter idiom, and ADR-0017 declined to report it on purpose.
    /// There is nothing to lose by leaving it, because a local holding no site
    /// shares none with anybody, and a `free` of it is reported by
    /// [`Allocations::touching`](super::transfer::Allocations::touching)'s own rule whatever this says.
    ///
    /// **Left alone here, not for good.** A call this check cannot read
    /// marks what it wrote as handed over, [`Held::from_caller`], and the
    /// next such call makes it lost by that route, so the idiom is quiet
    /// only until a later call that may free what it was handed (#394).
    ///
    /// **Which it may have written is a second answer, `may_write`**, about
    /// [`Self::unwritten`] alone. A store this check cannot pin down answers
    /// it with the locals its place may land in, of the type it stores; a
    /// call answers it with what it may reach,
    /// [`Self::exposed_locals`], which is narrower than every escaped local,
    /// so `int **p = m; tick(); q = *p;` reads `m` as nothing wrote it
    /// (#643). Two parameters rather than a second function, so that a new
    /// caller cannot give one answer and forget the other. See ADR-0057.
    pub(super) fn replaced(
        &mut self,
        may_hold: impl Fn(usize) -> bool,
        may_write: impl Fn(usize) -> bool,
    ) {
        for local in 0..self.escaped.len() {
            if may_hold(local)
                && self.escaped[local]
                && self.points_to[local].sites().next().is_some()
            {
                self.points_to[local].lost = true;
            }
            // **Whatever it holds**, since the writer may have put a pointer
            // to memory this check does not model there. A local holding no
            // site is given this and not `lost`, which is the output-parameter
            // idiom above: the report does not read it. See ADR-0044.
            if may_hold(local) && self.escaped[local] {
                self.points_to[local].foreign = true;
            }
            // **And it may have been written**, which is what a write this
            // check cannot read is: `int *q; get(&q); *pq` is the
            // output-parameter idiom, and believing `q` unwritten after it
            // reported a correct program. See `Self::unwritten`. The escape
            // decides nothing here, measured: a mark is read only of a local
            // whose address is taken, which is escaped, so dropping it leaves
            // the suite green. It is kept so that `may_write` answers for the
            // same locals `may_hold` does.
            if may_write(local) && self.escaped[local] {
                self.unwritten[local] = false;
            }
        }
    }
}
