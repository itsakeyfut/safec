//! The lattice value, [`Known`]: what every local may hold and what is known
//! about every site at one point of a function, and every walk over it that more
//! than one rule asks, from what a local reaches through how many levels to what
//! a call this check cannot read may have touched.
//!
//! The transfer functions change it and the report reads it; neither is here.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::{LocalId, Operand, Place};
use crate::source::Span;

use super::parts::{Held, PendingRead, Reached, Read, ReadKey, Realloced, SiteState, read_key};
use super::{derefs, inside, named};

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
    /// exposed `tab`. Square in the locals, which is [`Held::sites`]' condition
    /// for a packed bitset. See ADR-0039.
    pub(super) inside: Vec<Vec<bool>>,
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
    pub(super) inside_locals: Vec<Vec<bool>>,
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
    /// Per site, whether what it contains may include a pointer a caller or
    /// a callee handed over: one [`Held::from_caller`] marks, stored here.
    ///
    /// **The mark lived on locals only**, so a store recorded the sites such
    /// a pointer holds, which are none, and `*box = q; release_all(); r =
    /// *box; return *r;` built where `return *q;` was reported. A load out of
    /// a marked allocation reads caller memory, and a call this check cannot
    /// read makes it [`Self::stale`]. Nothing clears it, for `stale`'s
    /// reason. See ADR-0040.
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
        // [`verdict`] reads `Lost` as a flag, and a reader should not have to
        // work that out.
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
            reached.push(Reached::Lost);
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
        if self.points_to[local.index()]
            .writes_to
            .iter()
            .any(|&edge| edge)
            && !reached.is_empty()
        {
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
            .flat_map(|container| {
                self.inside[container]
                    .iter()
                    .enumerate()
                    .filter(|&(_, &in_it)| in_it)
                    .map(|(site, _)| site)
                    .collect::<Vec<_>>()
            })
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
                SiteState::Unknown => true,
                SiteState::Reachable => !through_address.contains(&site) || read_unreplaced,
                SiteState::Live(_) => false,
            })
            .map(Reached::Site)
            .collect();
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
            reached.push(Reached::Lost);
        }
        // And below a pointer to a local this check lost, at any level, which
        // is `**t2` and `*t2` handed on, as a load of `*t2` is. See ADR-0045.
        if self.lost_through(local, depth) {
            reached.push(Reached::Lost);
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
            next_sites.extend(
                self.inside[container]
                    .iter()
                    .enumerate()
                    .filter(|&(_, &in_it)| in_it)
                    .map(|(site, _)| site),
            );
            next_locals.extend(
                self.inside_locals[container]
                    .iter()
                    .enumerate()
                    .filter(|&(_, &in_it)| in_it)
                    .map(|(target, _)| target),
            );
        }
        for &held in locals {
            next_sites.extend(self.points_to[held].sites());
            next_locals.extend(
                self.points_to[held]
                    .writes_to
                    .iter()
                    .enumerate()
                    .filter(|&(_, &edge)| edge)
                    .map(|(target, _)| target),
            );
        }
        (next_sites, next_locals)
    }

    /// What a pointer read `depth` dereferences below `local` may be, and the
    /// locals whose address it may be: the walk [`Self::level_below`] takes,
    /// `depth` times from [`Self::level_zero`]. See ADR-0045.
    pub(super) fn levels_below(
        &self,
        local: LocalId,
        depth: usize,
    ) -> (BTreeSet<usize>, BTreeSet<usize>) {
        let Some((mut sites, mut locals)) = self.level_zero(local) else {
            return (BTreeSet::new(), BTreeSet::new());
        };
        for _ in 0..depth {
            (sites, locals) = self.level_below(&sites, &locals);
        }
        (sites, locals)
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
            for (held, &in_it) in self.inside[site].iter().enumerate() {
                if in_it && closed.insert(held) {
                    pending.push(held);
                }
            }
        }
        closed
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
    pub(super) fn noticed(&mut self, by: Option<Span>, routes: &BTreeSet<usize>) {
        // **And whether a call already pending may run between this and the
        // read.** It may unless the read belongs to it or to `by`, or it is
        // in `by`'s arguments: C17 6.5.2.2 p10 orders a call's arguments
        // before its body, so none of those can come after `by` and before
        // the read. See [`PendingRead::after_call`].
        let calls: Vec<Span> = self.calls.values().copied().collect();
        if calls
            .iter()
            .any(|&call| by != Some(call) && !by.is_some_and(|by| inside(call, by)))
        {
            self.exposed_after_call.extend(routes);
        }
        for entry in self.pending.values_mut() {
            let own_by = by.is_some_and(|by| entry.at == by || inside(entry.at, by));
            if !own_by && entry.sites.iter().any(|site| routes.contains(site)) {
                let freer = calls.iter().any(|&call| {
                    by != Some(call)
                        && !(entry.at == call || inside(entry.at, call))
                        && !by.is_some_and(|by| inside(call, by))
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
            let own = by.is_some_and(|by| entry.at == by || inside(entry.at, by));
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
            .filter(|target| self.points_to[local.index()].writes_to[*target])
            .collect()
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
            // Kept, as the entry and `stale` are: the slots of the new
            // allocation are not told apart from the old one's. No case
            // tells this from clearing it, measured: every other local
            // holding the site is lost a few lines below, so a load out of
            // it is doubted either way.
            from_caller: _,
            pending,
            calls: _,
            exposed_after_call,
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
                if gone && held.sites[site] {
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
            if gone && inside[container][site] {
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
    pub(super) fn expose(&mut self, sites: impl IntoIterator<Item = usize>, by: Option<Span>) {
        let sites: Vec<usize> = sites.into_iter().collect();
        let routes = self.closure(sites.clone());
        self.noticed(by, &routes);
        for site in sites {
            self.exposed[site] = true;
        }
        // One closure, shared with what a pending read is told, so that the
        // two cannot disagree about what an exposed allocation reaches.
        for site in self.reachable_now() {
            self.exposed[site] = true;
        }
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
                if self.inside[container][site] {
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
    /// beside one, a read carried forwards, and a `realloc`'s remembered fact.
    /// Every site, where a local may name one this check cannot: lost,
    /// loaded, or written by a call it cannot read.
    ///
    /// **It over-counts, and must**: a site answered as held by nothing takes
    /// its state from the other side of a join, so one held by something
    /// missed here is one whose free could be forgotten. See ADR-0048.
    pub(super) fn held_sites(&self) -> Vec<bool> {
        let sites = self.state.len();
        if self
            .points_to
            .iter()
            .any(|held| held.lost || held.loaded || held.foreign)
        {
            return vec![true; sites];
        }
        let mut held = vec![false; sites];
        for local in &self.points_to {
            for site in local.sites() {
                held[site] = true;
            }
        }
        for contents in &self.inside {
            for (site, &inside) in contents.iter().enumerate() {
                held[site] |= inside;
            }
        }
        for (site, held) in held.iter_mut().enumerate() {
            *held |= self.exposed[site] || self.exposed_after_call.contains(&site);
        }
        for read in self.pending.values() {
            for &site in read.sites.iter().chain(&read.reachable) {
                held[site] = true;
            }
        }
        for fact in self.realloced.iter().flatten() {
            for &(old, _) in &fact.old {
                held[old] = true;
            }
        }
        held
    }

    /// Every site a pointer stored in some allocation may hold.
    ///
    /// What a pointer read out of memory this check cannot say which may be:
    /// wherever it was read from, it was stored there, and a store into an
    /// allocation is recorded in [`Self::inside`] or exposed at once. One read
    /// out of a local through its address is not here and need not be: that
    /// local escaped, so what it holds is in every call's reach already. See
    /// ADR-0040.
    pub(super) fn stored(&self) -> Vec<usize> {
        (0..self.inside.len())
            .filter(|&held| self.inside.iter().any(|row| row[held]))
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
    pub(super) fn replaced(&mut self, may_hold: impl Fn(usize) -> bool) {
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
        }
    }
}
