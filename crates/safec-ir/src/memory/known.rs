//! The lattice value, [`Known`]: what every local may hold and what is known
//! about every site at one point of a function, and every walk over it that more
//! than one rule asks, from what a local reaches through how many levels to what
//! a call this check cannot read may have touched.
//!
//! The transfer functions change it and the report reads it; neither is here.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::{LocalId, Operand, Place, TyId};
use crate::source::{SourceMap, Span};

use super::index::{Loc, LocalIdx, Site, Table};
use super::parts::{
    At, Cell, Earliest, Extent, Held, Init, Kept, Levels, PendingRead, PlaceInit, Reached, Read,
    ReadKey, Realloced, SiteState, Width, read_key,
};
use super::store::{Certain, Strong, WrittenLocal};
use super::{LostReason, named};
use crate::reads::derefs;
use crate::source::inside;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Known {
    /// Per local, what it may hold.
    pub(super) points_to: Table<LocalIdx, Held>,
    /// Per site, what is known about it. Meaningless for a local nothing points
    /// at, which is most of them.
    pub(super) state: Table<Site, SiteState>,
    /// Per local, whether anything holds its address.
    ///
    /// **A property of the local rather than of what it held when the address
    /// was taken.** Marking only the sites it reached at that instant was
    /// #155: assigning to the local afterwards gave it a fresh site nothing
    /// had lost, so `int **pp = &p; p = malloc(8); *pp = q; *p = 1;` read a
    /// freed pointer in silence. Whatever an escaped local is given later is
    /// no more proved than what it held before, because the write that put it
    /// there is not the only write that can reach it.
    pub(super) escaped: Table<LocalIdx, bool>,
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
    pub(super) handed_away: Table<LocalIdx, bool>,
    /// Per site, whether code this check cannot read may reach a pointer to it.
    ///
    /// **Once set, set for as long as the allocation lives**, because nothing
    /// here can say that what an unread callee kept has gone: a pointer handed
    /// to one call may be returned or freed by the next. Every opaque call
    /// unproves every exposed allocation still live, and may return any of
    /// them. See ADR-0039.
    ///
    /// **Per local, whether such code may hold its address**: what a call
    /// may have written, for [`Self::written`], and nothing else reads that
    /// half. `lost` and `foreign` are still given to every escaped
    /// local, since a route missed there would be a free in silence
    /// (ADR-0029).
    ///
    /// **A library copy sets no local here**, though it exposes the
    /// allocations it is handed, because it keeps no address: it writes
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
    pub(super) exposed: Table<Loc, bool>,
    /// Per site, what its memory holds: the cell, a [`Held`] read as the
    /// value a pointer stored there may be, and [`Held::none`] where nothing
    /// was stored (#780). A store pours into it by [`Cell::absorb`]; a load
    /// reads it in [`Self::load_into`]. What each field a cell uses says:
    ///
    /// **`names`**: the sites a pointer stored in that allocation may hold,
    /// and the locals whose address it may be.
    ///
    /// What lets a pointer stored in the heap stay proved until something
    /// that can reach the allocation holding it is handed to code this check
    /// cannot read: `*tab = p; log_line();` leaves `p` alone, because nothing
    /// exposed `tab`. Square in the locals, so a bit a pair, for the reason
    /// [`Held::names`] gives. See ADR-0039.
    ///
    /// **A local's address is an edge, not a site** ([`Held::locals`],
    /// ADR-0019), so a store of one recorded nothing and `*t3 = &slot; **t3
    /// = r; free(r); return ***t3;` read nothing through `*t3`. Written by a
    /// store of a value carrying edges, read by [`Known::levels_below`](super::known::Known::levels_below), so
    /// a load out of the allocation carries the edges and a store two levels
    /// down lands in the local. See ADR-0045.
    ///
    /// **`believed` and `contents_at_most`**: how wide what is stored there
    /// is believed, level 0 and the levels below, as
    /// [`Cell::stored_levels`] reads them.
    /// How many bytes a pointer stored there is believed to point at, at
    /// most, at level 0, and what each pointer stored there believed of its
    /// own memory at the levels below, with no bound where nothing recorded
    /// one. See [`Levels`].
    ///
    /// **Every pointer stored, and not only one the record cannot name**: a
    /// local's address or an allocation at the width of what it names
    /// ([`Held::recorded_width`]), since a load reading memory where nothing
    /// is recorded believes nothing, and `*box = &q; int *t = *box; *t = 1;`
    /// would be doubted (ADR-0061).
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
    /// `names` is. See ADR-0060.
    ///
    /// **`stale_read`**:
    /// Whether what it contains may include an allocation that is
    /// gone and whose site now names a new one.
    ///
    /// **The column of `names`, where [`Held::lose`] is a local's.** A
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
    ///
    /// **`unreplaced_read`**:
    /// Whether it may contain a slot holding an allocation a call
    /// this check cannot read reached through another route and could not
    /// replace there. Set by [`Known::held_out_of_reach`](super::known::Known::held_out_of_reach), carried where what
    /// a load out of it holds is stored or copied, and read through
    /// [`Held::unreplaced_read`] by [`Known::handed_below`](super::known::Known::handed_below) alone. Nothing
    /// clears it, since slots are not told apart. See ADR-0047.
    ///
    /// **`lost`**:
    /// Whether a value this check lost may have been stored in it.
    ///
    /// **A lost value names no site**, so the free that releases it cannot
    /// say which allocation went, and nothing links it to where it was
    /// stored before: `*box = u; free(u); r = *box;` with `u` lost to a
    /// rebirth read `*r` in silence (#503). [`Known::lost_may_be_gone`](super::known::Known::lost_may_be_gone) reads
    /// this to mark every such allocation `stale_read`. Nothing clears it, as
    /// nothing clears `stale_read`. See ADR-0045.
    ///
    /// **`from_caller`**:
    /// Whether what it contains may include a pointer a caller or
    /// a callee handed over: one [`Held::from_caller`] marks, stored here.
    ///
    /// **The mark lived on locals only**, so a store recorded the sites such
    /// a pointer holds, which are none, and `*box = q; release_all(); r =
    /// *box; return *r;` built where `return *q;` was reported. A load out of
    /// a marked allocation reads caller memory, and a call this check cannot
    /// read makes it `stale_read`. Nothing clears it, for `stale_read`'s
    /// reason. See ADR-0040.
    ///
    /// **A store is not the only producer.** A pointer parameter's site is
    /// marked where the function starts, since what it points at is the
    /// caller's memory, and a call this check cannot read marks its result's
    /// site once it returns, since that is the callee's (#482).
    pub(super) contents: Table<Site, Cell>,
    /// Per site, what a `realloc` that returned it was handed, until a branch
    /// on a pointer holding it says whether it succeeded. See [`Realloced`]
    /// and ADR-0039.
    pub(super) realloced: Table<Site, Option<Realloced>>,
    /// **Whether each place was written, one record per place, whole or per
    /// slot** ([`PlaceInit`], ADR-0092 rule 3), its whole answer what is said
    /// below of a place.
    ///
    /// Per site, whether it is an allocation `malloc` or `aligned_alloc` made
    /// and nothing has written since: [`Init::Unwritten`] on every path, and
    /// its contents are indeterminate (C17 7.22.3.1 p2, 7.22.3.4 p2). Joined
    /// by [`Init::joined`]. Marked written by [`Self::certainly_written`],
    /// where a store proved one place, by
    /// [`Self::believed_written_by_unknown_code`], at the boundary; every
    /// other door weakens it with `Init::may_write` or `Init::doubt`, or
    /// restores what it was, [`Self::may_write_all_sites`] among them
    /// (ADR-0074 rule 5). A pointer read out of it is
    /// reported where it is read, and is
    /// [`Held::fresh_read`](super::parts::Held::fresh_read). Any store clears
    /// it, a store of an indeterminate pointer included; a free does not.
    /// See ADR-0057. Not "empty":
    /// taking a fresh block's contents as empty is the unsound starting set
    /// #579 met.
    ///
    /// **[`Init::Maybe`] where it may still be so on some path**: set at a
    /// forward join where one side left the site fresh and the other did not. A load out
    /// of it is the doubt `Allocations::reads_partly` reports, as one out of
    /// an unwritten local is (#668): `if (k) *m = *pp; r =
    /// *m;` with `m` from `malloc` built in silence, where the same without
    /// the `if` is certain (#721). **Not at the back edge of a loop shown to
    /// run at least once**, so such a loop that fills an allocation is not
    /// doubted for a path that skips the body, as for a local; after a loop
    /// that may run zero times it is a doubt (#783, ADR-0064). Cleared
    /// wherever `fresh` is. A test that always holds, `if (d != 0) *d =
    /// *s;`, is a doubt it cannot tell from one
    /// that may fail, the cost #668 recorded for a local. See ADR-0057.
    ///
    /// **Per local whose address is taken, the same question**: whether
    /// nothing may have written it since it was made, on every path: its value is indeterminate (C17 6.2.4 p6, 6.7.9 p10), and
    /// Annex J.2 lists using it as undefined. That line of J.2 names no
    /// condition on the address; the normative text makes such a read
    /// undefined only through a trap representation (6.2.6.1 p5) where the
    /// address is taken, and this reads J.2 as ADR-0057 does for an
    /// allocation. Set where a local is made, whatever its type, at the
    /// function's entry and at its `StorageLive`, never for a parameter
    /// (ADR-0073 rule 2), and
    /// cleared only by [`Self::written_local`] and [`Self::replaced`], which a write
    /// that may land there calls: an assignment, a placed store, a call's
    /// result, a call, which writes only the locals in
    /// [`Self::exposed`] (#643), and a store or a library copy this
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
    /// **What this table says of an allocation, it says of a local reached
    /// by its address**: a pointer read out of one is reported where it is
    /// read, `int *q = m[0];` with `m` a local array nothing wrote (#640).
    /// A local whose address is never taken is `assigned.rs`'s, which reads
    /// it by name. See ADR-0057.
    ///
    /// **[`Init::Maybe`], for a local kept whole, where it may still be
    /// unwritten though it is not certainly.** An array kept per slot answers
    /// this slot by slot.
    ///
    /// **Set by a write that may have missed it**: an array longer than
    /// [`ELEMENTS_KEPT`] written through a pointer moved off its start, which
    /// may reach any element (C17 6.5.6 p8), so its write is neither a write
    /// of every element nor of none (#639). A read at the array's start
    /// afterwards is doubted rather than proved, by
    /// `Allocations::reads_partly`.
    ///
    /// **And by a join of a path where the local is unwritten with one where
    /// it was written** (#668): `if (c()) q = &x;` then `*pq` with `pq = &q`
    /// read an indeterminate `q` on one path in silence, since
    /// its proof joined by intersection. A doubt, read
    /// through a pointer and by name, `report::unwritten_read`. Not at the
    /// back edge of a loop shown to run at least once, where the path that
    /// skips the body is one no execution takes, which leaves a read in the
    /// body before the body writes it silent too (#670); what either side
    /// already doubted is kept there. At any other loop's back edge it is the
    /// doubt, since that loop may run zero times (#783, ADR-0064). An array
    /// kept per element gains no such doubt at any join, its elements
    /// carrying it. Written by a write of the whole local; a write of part
    /// of a local kept whole leaves it `Init::Maybe` instead (#826,
    /// `built::write_effects`). Unwritten again at `StorageLive`. See
    /// ADR-0057.
    ///
    /// **Per slot, for a local array of at most [`ELEMENTS_KEPT`] innermost
    /// elements**, [`Kept::Slots`]; every other local is kept whole.
    ///
    /// **So that a write to one element leaves another unwritten** (#667):
    /// `m[0] = &x; q = m[1];` read an `m[1]` nothing wrote, in silence, since
    /// the write to `m[0]` cleared the one record `m` had. A write through a
    /// pointer whose element is known writes that element alone; one whose
    /// element is not known may have missed each element, [`Init::may_write`],
    /// since no element is then certainly unwritten and each may still be
    /// (ADR-0094 rule 3). Unwritten again at `StorageLive`. See
    /// `built::write_effects` and ADR-0057.
    ///
    /// **[`Init::Maybe`] for an element that may be unwritten on some path,
    /// rather than a mark on the array** (#668), so that `if (c()) m[0] = &x;
    /// q = m[1];` stays the certain `SC0411` `main` gives, while `m[0]` read
    /// there is doubted. Joined per element as [`Init::joined`] says, so the
    /// back edge of a loop shown to run creates no doubt, as for
    /// [`Self::written`]. See ADR-0057 and ADR-0064.
    ///
    /// **Per slot, for a block of a constant size of two to
    /// [`ELEMENTS_KEPT`] pointers**: `b[0] = x;` with `b` from `malloc(16)`
    /// writes the first and leaves the second (#873). Every other block is
    /// kept whole. **A write of the whole block reaches every slot**, as
    /// [`PlaceInit`] says, where it once dropped the slots beside a stamp of
    /// the whole they were made from, which fifteen writers had to pair by
    /// hand (#894). A write through a pointer whose element is not known may
    /// have missed every slot (ADR-0094 rule 3), and a store narrower than a
    /// block kept whole may have missed the rest of it (#918's review). Kept
    /// inline, whole or per slot, with where its unwritten bytes came from
    /// (#752). See ADR-0081.
    pub(super) written: Table<Loc, PlaceInit>,
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
    pub(super) exposed_after_call: BTreeSet<Site>,
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
    ///
    /// **And per local, its size on this target**, in the same table: one
    /// kind of place (#779). [`Extent::Exactly`] where the local has a size
    /// and `None` where it has none, written where the function starts and
    /// never after, since a local is the one object its name stands for. What
    /// [`Held::recorded_width`] reads a local's address at.
    pub(super) extent: Table<Loc, Option<Extent>>,
    /// Per local, how many bytes a pointer a call may write into it is
    /// believed to point at, at most: the narrowest width the parameters
    /// it was handed through believe at its level, and `0` where one was a
    /// `void *` or bytes (#675). That is what this check asks of a callee's
    /// body, which is asked at the types it stores through: through a
    /// `void *` it may have stored any pointer at any type, and through bytes
    /// by `memcpy`. C allows no more through one than through an `int **`
    /// (C17 6.5 p7), so this is a limit of the asking and not of the
    /// language. No bound where no call was handed it.
    ///
    /// **What a callee wrote is believed as it was handed**, not as the
    /// local is declared: `int *v` handed to `put(void **slot, char *c)`
    /// holds a `char`'s address afterwards, asked as a `void *` in `put`'s
    /// body, and was believed a whole `int` (#632). A bound, so the lower of
    /// two at a join. See ADR-0060.
    ///
    /// **Level 0 is that width, and every level after it what the types a
    /// call was handed it through say of its value's memory**, one level
    /// further down each time, as [`Held::contents_at_most`] reads them: one
    /// [`Levels`] for both, since the two were bounds joined alike (ADR-0093
    /// rule 2). Every level `0` where it was
    /// handed through a `void *` or as bytes, either of which may store any
    /// pointer at any type (#675), and so is every local that call reaches
    /// below either (#677). One put there after the call, and written by a
    /// later one, is not, which is #679.
    ///
    /// **So that what a callee wrote is believed as it was handed at every
    /// level** (#636): `put(pv)` with `pv = &v` and `put` taking `void ***`
    /// left `v` an `int ***` believing whole `int`s two levels down, so
    /// `use(v)` with `use` writing `**(*p)` handed a one-byte `char`'s
    /// address on in silence. A bound, so the lower of two at every level
    /// and at a join. See ADR-0060.
    pub(super) handed_at_most: Table<LocalIdx, Levels>,
    /// Per local array, whether code this check cannot read was handed it
    /// only through a pointer moved off its start, so that a call writing it
    /// later writes it only in part (#667). A union at a join, and cleared at
    /// `StorageLive`. See `built::write_effects`.
    pub(super) exposed_in_part: Table<LocalIdx, bool>,
    /// Per local array, whether a pointer that reaches it only by #639's
    /// rule, [`Held::array_edge`], was stored in memory, so that a pointer
    /// read back out of memory that names it may be that one (#667). A load
    /// keeps no element, and without this it wrote the array whole:
    /// `box[0] = m + 1; f = box[0]; *f = &x;` left `m[0]` believed written.
    /// Per array rather than per container, so a load from any memory that
    /// names the array is asked as if it may be that pointer: a doubt the
    /// reader sees, never a proof. A union at a join, and cleared at
    /// `StorageLive`. See `built::write_effects`.
    pub(super) stored_in_part: Table<LocalIdx, bool>,
    /// Per site, whether a pointer to it may have been stored where code
    /// outside this function may read it, by a store or a library copy whose
    /// `store::Store` leaves (#684): into a caller's memory through a parameter, into memory or a
    /// local such code can name, or anywhere a pointer this function never
    /// recorded may point.
    ///
    /// **So that a free of it is asked what such code is left holding**
    /// (#650): `*pp = x; free(x);` handed the caller a freed pointer in
    /// silence, since the store was of a live one and a parameter's memory is
    /// no site this check models, so nothing at the function's exit can be
    /// asked what it holds. `report::reported` asks it at the free, as a
    /// doubt under `SC0414`: the other order, `free(x); *pp = x;`, is the store
    /// door's. A may-fact, so a union at every join, and cleared where the site
    /// is made again (`Self::reborn`); never cleared by a later store, since
    /// this check does not follow what one place in such memory holds (#656).
    /// See ADR-0041. That is [`Earliest::now`].
    ///
    /// **And [`Earliest::ever`], the earliest write that let it leave at
    /// all**, whatever its type read, kept where the site is made again, as
    /// `left_at` is. The exit asks one whose [`Self::held_levels`] says
    /// pointers by then, so a block held as `int **` only after it left
    /// through a `void *` is asked as one held so before: `*out = v; int **b
    /// = v;` was silent where `int **b = v; *out = v;` is `SC0411`, found by
    /// review. See ADR-0086. **One value for the two**, so that what writes
    /// one writes both (#934).
    pub(super) stored_out: Table<Site, Earliest>,
    /// Per site, the write that let it leave holding pointers, where one
    /// may have: a store or a library copy into memory that leaves, of a
    /// pointer to it or to a block that holds one, as far down as the
    /// written type reads pointers (ADR-0076, ADR-0085).
    ///
    /// **Asked at the function's exit, not at the store**: the caller reads
    /// the block after the function returns, and `*out = b; *b = x;` fills it
    /// before then (#852). A may-fact, so a union where paths meet, keeping
    /// the earlier store's span so the report has one caret. **Kept where
    /// the site is made again**, since the caller may hold the allocation it
    /// named before; see `Self::reborn`.
    pub(super) left_at: Table<Site, Option<Span>>,
    /// Per site, the most levels a local it was held in reads pointers at
    /// (`Allocations::levels_holding_pointers` of the local's declared type):
    /// `int **b = malloc(8);` records `1` for the block, and a block only
    /// ever held as a `void *`, `char *` or `int *` records `0`.
    ///
    /// **On the allocation, not on the value**, so a `void *` level the
    /// block is reached through later still finds it: `void *r(void) { int
    /// **b = malloc(8); return b; }` hands the caller a block it reads
    /// pointers out of once it converts the result back, and was silent,
    /// since a `void *` is read as holding no pointer (#902). Read by
    /// [`Self::held_below`] and by the `return`'s ask. A may-fact, joined by
    /// the larger, and kept where the site is made again, as `left_at` is.
    /// See ADR-0086.
    pub(super) held_levels: Table<Site, usize>,
    /// Per site, the earliest library copy that may have written part of a
    /// pointer into it: one whose length is shorter than a pointer, or ends
    /// part way through one (`Allocations::copied_pointers`), whatever either
    /// side's type said then. **Or into a block it was copied or
    /// `realloc`ed from**, whose bytes this one now holds: the record is
    /// carried to the new block (#905, ADR-0088). **A local's too**, by
    /// `Loc`, so a stack buffer between two copies, or a partial copy made
    /// straight into one, carries it on (#908, ADR-0090).
    ///
    /// **Asked where the block is later held as memory of pointers**: where
    /// it leaves so (the exit's question, beside `left_at`) and where it is
    /// handed to a parameter whose type holds pointers. The copy is the
    /// defect only if pointers are read out of the block, which the copy
    /// itself cannot know when nothing typed has reached the block yet:
    /// `memcpy(m, &i, 4); void **q = m; *pp = q;` built silent (#832). A
    /// may-fact, joined keeping the earlier span, and kept where the site is
    /// made again, as `left_at` is: [`Earliest::ever`]. See ADR-0087.
    ///
    /// **And [`Earliest::now`], for the allocation the site names now**,
    /// cleared where the site is made again: what a read inside the function
    /// asks (ADR-0088 rules 3 and 4), since it reads the block the site
    /// names now, not one an earlier turn made and a caller may still hold.
    /// Asked by the kept one, a loop that allocated, read a whole pointer,
    /// and only then copied part of one into the block was doubted at the
    /// copy every turn, found by review. One value for the two (#934).
    pub(super) part_written: Table<Loc, Earliest>,
    /// Per local, each type a call this check cannot read reached it as on
    /// the walk a departure takes, where that type is untyped somewhere
    /// below: `void`, `char` or `int` bytes, a pointer whose chain ends in
    /// `void` or `char`, or `None` below a type that stops being a pointer.
    /// The same table says the same of memory, per site.
    ///
    /// **So that a later call bounds what is below it then** (#679): such a
    /// callee may keep the address and store any pointer through it at any
    /// later call, and `keep(&v); v = &w; other(); use1(w);` handed a
    /// one-byte allocation on in silence, where `v = &w` before `keep(&v)`
    /// was doubted. Every call this check cannot read walks again from each
    /// local at each type recorded here, and bounds what it reaches as a call
    /// handed it so. **The type and not only the fact**, since `keep(void
    /// **)` handed `&v` reaches `v` as a pointer, and only the type says what
    /// is below it is untyped: a mark alone left that route silent, found by
    /// review. Not a chain that ends in `int`, which a well-typed `int **`
    /// out-parameter is: walking from those doubted locals a may-set puts
    /// below a typed callee's memory at every later call. A union at a join,
    /// never cleared while the local lives, and cleared at its
    /// `StorageLive`, as [`Self::exposed`] is.
    ///
    /// **Wrong in both directions, and the two are not alike.** A mark
    /// missed leaves a local below it believed whole, and a program is
    /// silent. A mark set where no call kept anything doubts what is put
    /// below that local after any later call, which the reader sees. See
    /// ADR-0060.
    ///
    /// **And per site, each type such a call reached the memory it names
    /// as**, in the same table: one kind of place (#779). `keep(m); *m = &w;
    /// other();` may write `w`. Kept where the site is made again
    /// (`Self::reborn`), as `names` is, since the old allocation's address
    /// may still be held and the slots of the two are not told apart.
    pub(super) handed_untyped: Table<Loc, Vec<Option<TyId>>>,
}

impl Known {
    /// Every site this local may point at.
    pub(super) fn sites_of(&self, local: LocalId) -> impl Iterator<Item = Site> + '_ {
        self.points_to[local].sites()
    }

    /// Note on `site` a copy at `copy` that may have written part of a
    /// pointer into it, keeping the earlier of what was there: one body for
    /// the copy that writes it and the copy and `realloc` that carry it, so
    /// the three join it alike. See [`Self::part_written`] and ADR-0088.
    pub(super) fn note_part_written(&mut self, sources: &SourceMap, site: Loc, copy: Span) {
        self.part_written[site].note(sources, copy);
    }

    /// What was stored through a pointer naming any of `places` is no longer
    /// what a read through it reads, since a write of part of a pointer
    /// landed over it there. See `Held::stored_through`.
    pub(super) fn written_over(&mut self, places: &[Loc]) {
        for held in self.points_to.iter_mut() {
            let names = held.sites().any(|site| places.contains(&Loc::Heap(site)))
                || held
                    .locals()
                    .any(|local| places.contains(&Loc::Local(local)));
            if names {
                held.stored_through = false;
            }
        }
    }

    /// Carry what `from` recorded to every site in `to`, as a copy or a
    /// `realloc` moves the bytes: each record from its own, so that what a
    /// read here asks comes only from blocks made since (ADR-0088).
    pub(super) fn carry_part_written(&mut self, sources: &SourceMap, from: &[Loc], to: &[Loc]) {
        let mut carried = Earliest::none();
        for &block in from {
            carried.join(sources, self.part_written[block]);
        }
        for &site in to {
            self.part_written[site].join(sources, carried);
        }
    }

    /// Stop following whatever this local held, because its lifetime began
    /// or ended or nothing reads it again: no write, which is
    /// [`Self::replace`]'s, and so no [`Strong`] asked for.
    ///
    /// **Every field named, never `..`**: this is the door a local's
    /// lifetime ends by, and a field added to [`Known`] answers here for what
    /// that does to it, as it does in the join and in [`Self::reborn`]. See
    /// ADR-0091.
    pub(super) fn clear(&mut self, local: LocalId) {
        let Known {
            points_to,
            // **A site's facts are about the allocation it names**, which
            // outlives the local it is named by: another holder may still
            // reach it. The same for every field keyed by a site below.
            state: _,
            // Distrust of what the local holds, the side that doubts
            // (ADR-0017). Not touched here, and nothing has asked whether a
            // lifetime ending may drop it.
            escaped: _,
            // Not touched here, and nothing has asked whether a lifetime
            // ending may drop it.
            handed_away: _,
            // A local's is cleared where its next lifetime begins
            // (`Self::storage_live`); a site's lives as long as its block.
            exposed: _,
            contents: _,
            realloced: _,
            // What reads the local next is its next lifetime, which
            // `Self::storage_live` makes unwritten, every slot of it, or
            // nothing.
            written: _,
            // About reads and calls in flight, not about a local's storage.
            pending: _,
            calls: _,
            exposed_after_call: _,
            lost_exposed: _,
            // A local's size, the same in every lifetime.
            extent: _,
            // Not touched here, and nothing has asked whether a lifetime
            // ending may drop the bound.
            handed_at_most: _,
            // Cleared where the next lifetime begins (`Self::storage_live`).
            exposed_in_part: _,
            stored_in_part: _,
            // These four are about a site, not a local: what left the
            // function and what a block was held as are facts about the
            // block, which a local's lifetime ends nothing of.
            stored_out: _,
            left_at: _,
            held_levels: _,
            part_written,
            // A local's is cleared where its next lifetime begins
            // (`Self::storage_live`).
            handed_untyped: _,
        } = self;
        points_to[local].clear();
        // **And what a copy wrote into its storage**: nothing outside the
        // function can hold a local's storage once its lifetime is over, so
        // the reason a block's record is kept does not reach it. Kept, a
        // buffer declared in a loop body carried last turn's partial copy
        // into this turn's. See ADR-0090.
        part_written[local] = Earliest::none();
    }

    /// A local's lifetime begins (`StorageLive`): a new object, holding
    /// nothing and written nowhere.
    ///
    /// **Every field named, never `..`**, as in [`Self::clear`], which runs
    /// first and answers for what it clears. See ADR-0091.
    pub(super) fn storage_live(&mut self, local: LocalId) {
        self.clear(local);
        let Known {
            points_to,
            // A site's, about the block it names, as in `Self::clear`.
            state: _,
            // Not touched here, as in `Self::clear`.
            escaped: _,
            handed_away: _,
            exposed,
            contents: _,
            realloced: _,
            written,
            // About reads and calls in flight, not about a local's storage.
            pending: _,
            calls: _,
            exposed_after_call: _,
            lost_exposed: _,
            // A local's size, the same in every lifetime.
            extent: _,
            // Not touched here, as in `Self::clear`.
            handed_at_most: _,
            exposed_in_part,
            stored_in_part,
            stored_out: _,
            left_at: _,
            held_levels: _,
            // Cleared by `Self::clear` above.
            part_written: _,
            handed_untyped,
        } = self;
        // **A new object, indeterminate again** (C17 6.2.4 p6), so an
        // array written late in one turn of a loop is unwritten at
        // the start of the next. See `Known::written`. **Whatever its
        // type**, as at entry (ADR-0073 rule 2): a `char` buffer
        // written late in one turn and copied out early in the next
        // copied what the new object never had. Found by review.
        // Every slot of it, with nothing written in part and nothing handed
        // over. See `Known::written`.
        written[local].set(Init::Unwritten);
        exposed_in_part[local] = false;
        stored_in_part[local] = false;
        // **And no call holds its address yet**: one kept from an
        // earlier lifetime became indeterminate when that one ended
        // (C17 6.2.4 p2), so a call is no longer a write of it. Kept
        // across, `stash(m)` in one turn of a loop made `tick()` in
        // the next a write of a new `m`, and a read nothing wrote was
        // silent. See `Known::exposed` of a local.
        exposed[local] = false;
        // Nor was it handed to one untyped, for the same reason.
        // See `Known::handed_untyped`. No case tells this from
        // keeping the mark, measured: what the mark reaches is
        // written only where it is exposed, and that is cleared just
        // above.
        handed_untyped[local].clear();
        // **Nor does any pointer recorded as able to name it**, for
        // the same reason: a pointer a call wrote in an earlier turn
        // named the old object, so a write through it is no write of
        // the new one (#661). See `Held::named_at_read`.
        for held in points_to.iter_mut() {
            held.named_at_read.remove(LocalIdx::of(local));
        }
        // **Nor does a pointer that held its address**: the address
        // is indeterminate now, so a pointer copied in an earlier
        // turn is no write of the new object, `*b = m;
        // memcpy(&t, src, 8);` across turns (#664). The edge
        // is removed and nothing more. Marking the pointer `lost` as
        // well was tried and rejected: a write through a lost pointer
        // lands in every local such code names, which turned a
        // certain `SC0411` `main` gave into doubts, and it doubted
        // `arr[0] = &x; *arr[0]` every turn. A read through such a
        // pointer is undefined and is still silent, #665. An
        // allocation's record of the address is left: removing it
        // decides no case, measured, once the pointer is not marked.
        // See ADR-0057.
        for held in points_to.iter_mut() {
            held.unname_local(LocalIdx::of(local));
        }
    }

    /// The local `at` names holds `value` and nothing it held before: **the
    /// strong update, not [`Self::replaced`]**, which a write that may land in
    /// several escaped locals takes, and which replaces nothing. The one
    /// replacement of a local's whole value, which only a [`Strong`] can ask
    /// for. Every other write that may land in a local adds to what it held,
    /// [`Self::target_union`]. See ADR-0028 and ADR-0063.
    pub(super) fn replace(&mut self, at: Strong, value: Held) {
        self.points_to[at.local()] = value;
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
    /// [`Allocations::touching`](super::allocations::Allocations::touching) wants the opposite answer for an empty set and
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
        if let Some(freed) = self.points_to[local].freed {
            reached.clear();
            reached.push(Reached::SetFreed(freed));
        }

        if self.points_to[local].beyond().lost() || (self.escaped[local] && !reached.is_empty()) {
            reached.push(Reached::Lost(LostReason::Other));
        }
        // **A load's sites are what was stored where it was read from**, which
        // may not be all it holds: the report may doubt from them and never
        // prove. See ADR-0045.
        if self.points_to[local].beyond().loaded() && !reached.is_empty() {
            reached.push(Reached::Partial);
        }
        // **And so is a set beside a local's address**: on the path that gave
        // it `&x`, it points at `x` and at none of its sites, so the sites
        // being freed proves nothing about every path. `if (c) q = p; else q
        // = &x; free(p); return *q;` was a proved use after free that the
        // `else` path does not commit. Found by review of ADR-0048, whose
        // join opened a second way to it.
        if self.points_to[local].may_be_a_locals_address() && !reached.is_empty() {
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
    /// freed, at any level: the allocations [`Self::stored_below`] says it may
    /// contain one level in, through `&a` or through what this function
    /// stored in its own memory, that are freed or unproven, and so on below
    /// that, by the same rule (#654). See ADR-0042.
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
    /// passes through a pointer read out of an allocation whose cell is
    /// `unreplaced_read`: `c = *h; use2(&c);` after a call that
    /// reached what `*h` holds through another route. **Never a proof**:
    /// [`Reached::Partial`] is always beside them, since the callee may only
    /// write there. See ADR-0042.
    ///
    /// **And a pointer this check lost, where the memory pointed at is
    /// a cell's `stale_read`**: it held an allocation a loop has since made again
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
        let through_memory: BTreeSet<Site> = sites
            .iter()
            .copied()
            .flat_map(|container| self.contents[container].sites().collect::<Vec<_>>())
            .collect();
        let through_address: BTreeSet<Site> = locals
            .iter()
            .flat_map(|&target| self.points_to[target].sites())
            .filter(|site| !through_memory.contains(site))
            .collect();
        let read_unreplaced = locals
            .iter()
            .any(|&target| self.points_to[target].beyond().unreplaced_read());
        let (first, first_locals) = self.level_below(&sites, &locals);
        let mut reached: Vec<Reached> = first
            .iter()
            .copied()
            .filter(|&site| match self.state[site] {
                SiteState::Freed { .. } => true,
                SiteState::Unknown | SiteState::Unheld => true,
                SiteState::Reachable => !through_address.contains(&site) || read_unreplaced,
                SiteState::Live(_) => false,
            })
            .map(Reached::Site)
            .collect();
        // **And every level below the first, by the first level's rule**,
        // until neither set grows, which two finite sets that only grow must
        // reach. A box of a box holding a freed pointer was asked at no door
        // (#654). What is freed or unproven counts wherever it is reached; a
        // `Reachable` one counts where it is what an allocation on the way
        // holds, which is what the first level's `through_address` split
        // amounts to, and not where only a local's address reaches it.
        // Counting `Reachable` however it was reached doubted twelve corpus
        // programs for nothing; not counting it at all left a pointer a call
        // may have freed, two boxes in, silent. Each local is walked once, not
        // again at every level below it.
        // The memory walked through is read for `stale_read` at every level.
        let mut walked = sites.clone();
        let mut below = first.clone();
        let mut seen_locals = locals.clone();
        seen_locals.extend(first_locals.iter().copied());
        let mut frontier = (first, first_locals);
        loop {
            walked.extend(frontier.0.iter().copied());
            let in_memory: BTreeSet<Site> = frontier
                .0
                .iter()
                .flat_map(|&container| self.contents[container].sites().collect::<Vec<_>>())
                .collect();
            let (next_sites, next_locals) = self.level_below(&frontier.0, &frontier.1);
            let new_sites: BTreeSet<Site> = next_sites.difference(&below).copied().collect();
            let new_locals: BTreeSet<LocalIdx> =
                next_locals.difference(&seen_locals).copied().collect();
            if new_sites.is_empty() && new_locals.is_empty() {
                break;
            }
            for &site in &new_sites {
                let kept = match self.state[site] {
                    SiteState::Freed { .. } | SiteState::Unknown | SiteState::Unheld => true,
                    SiteState::Reachable => in_memory.contains(&site),
                    SiteState::Live(_) => false,
                };
                if kept {
                    reached.push(Reached::Site(site));
                }
            }
            below.extend(new_sites.iter().copied());
            seen_locals.extend(new_locals.iter().copied());
            frontier = (new_sites, new_locals);
        }
        if walked
            .iter()
            .any(|&container| self.contents[container].stale_read())
        {
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
            .any(|target| self.points_to[target].beyond().lost())
    }

    /// The locals a read `depth` dereferences below `local` passes through:
    /// those the local's own edges name, then those each level of
    /// [`Self::level_below`] reaches, short of the last.
    fn locals_read_through(&self, local: LocalId, depth: usize) -> BTreeSet<LocalIdx> {
        let mut sites: BTreeSet<Site> = self.points_to[local].sites().collect();
        let mut locals: BTreeSet<LocalIdx> = self.written_through(local).into_iter().collect();
        let mut passed = BTreeSet::new();
        for level in 0..depth {
            passed.extend(locals.iter().copied());
            if level + 1 < depth {
                (sites, locals) = self.level_below(&sites, &locals);
            }
        }
        passed
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
            .any(|target| self.points_to[target].beyond().is_from_caller())
    }

    /// What a pointer handed to a call holds: the allocations it may point
    /// at and the locals whose address it may be. For a plain local, what it
    /// holds and its address edges; for a place of dereferences, the level a
    /// load of it would hold, which is what `q = *k;` puts in `q`. Nothing
    /// for a local this check lost, whose contents are not named. See
    /// ADR-0045.
    fn handed_level(&self, place: &Place) -> Option<(BTreeSet<Site>, BTreeSet<LocalIdx>)> {
        if place.projection.is_empty() {
            self.level_zero(place.local)
        } else {
            Some(self.levels_below(place.local, derefs(place)))
        }
    }

    /// Whether a pointer read `depth` dereferences below `local` may be one
    /// a cell's `stale_read` says this check stopped following: whether any
    /// allocation the chain reads through, at any level, is marked.
    ///
    /// Walked as [`Self::stored_below`] walks what the levels contain, so the
    /// two answer the same places. Not for a lost local, whose load is
    /// answered by [`Self::stored`]. See ADR-0045.
    pub(super) fn stale_below(&self, local: LocalId, depth: usize) -> bool {
        self.marked_below(local, depth, |contents| contents.stale_read())
    }

    /// Whether any allocation a pointer read `depth` dereferences below
    /// `local` reads through, at any level, is set in `marks`, a per-site
    /// column: a cell's `stale_read` or a cell's `from_caller`.
    ///
    /// **One walk for both**, so that `r = **bb;` and `b = *bb; r = *b;` are
    /// asked about the same allocations: asked about the local's own sites
    /// alone, the first read `bb`'s allocation and never the one `*bb` names,
    /// and built in silence where the second was doubted. Found by review.
    pub(super) fn marked_below(
        &self,
        local: LocalId,
        depth: usize,
        marked: impl Fn(&Cell) -> bool,
    ) -> bool {
        self.cells_on_the_way(local, depth)
            .into_iter()
            .any(|site| marked(&self.contents[site]))
    }

    /// Every allocation a pointer read `depth` dereferences below `local`
    /// reads through, at any level: what [`Self::marked_below`] asks, and
    /// what [`Self::load_into`] takes its marks from.
    fn cells_on_the_way(&self, local: LocalId, depth: usize) -> BTreeSet<Site> {
        let mut passed = BTreeSet::new();
        let Some((mut sites, mut locals)) = self.level_zero(local) else {
            return passed;
        };
        for _ in 0..depth {
            passed.extend(sites.iter().copied());
            (sites, locals) = self.level_below(&sites, &locals);
        }
        passed
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
    fn level_zero(&self, local: LocalId) -> Option<(BTreeSet<Site>, BTreeSet<LocalIdx>)> {
        let held = &self.points_to[local];
        if held.beyond().lost() {
            return None;
        }
        Some((
            held.sites().collect(),
            self.written_through(local).into_iter().collect(),
        ))
    }

    /// One dereference further: what the allocations may contain
    /// (a cell's `names`) and what the locals hold,
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
        sites: &BTreeSet<Site>,
        locals: &BTreeSet<LocalIdx>,
    ) -> (BTreeSet<Site>, BTreeSet<LocalIdx>) {
        let mut next_sites = BTreeSet::new();
        let mut next_locals = BTreeSet::new();
        for &container in sites {
            next_sites.extend(self.contents[container].sites());
            next_locals.extend(self.contents[container].locals());
        }
        for &held in locals {
            next_sites.extend(self.points_to[held].sites());
            next_locals.extend(self.points_to[held].locals());
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
    ) -> (BTreeSet<Site>, BTreeSet<LocalIdx>) {
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
            // a cell's `names` recorded. At the first dereference, `level ==
            // 0`, only when `local` is a load or a pointer a call wrote. At
            // every later one whatever `local` is, because the pointer
            // dereferenced there was read out of memory and is a load: `q =
            // **ppp;` reads what `t = *ppp; q = *t;` reads through `t`.
            // Through a cell's `names` alone, `**ppp = m; free(m); q = **ppp;
            // return *q;` was believed at `*q` (#486). It reads more than a
            // chain of this function's own allocations can hold, a doubt the
            // reader sees, and the same one the chain written through locals
            // already gave. See ADR-0045.
            let reached: Vec<Site> = if level == 0 {
                self.exposed_through(local)
            } else {
                self.reachable_now().into_iter().collect()
            };
            // And the locals whose address that memory may hold, which a
            // store there recorded: `*a = &x;` through a load `a` puts
            // `x` where `t = *b;` through another load may read it.
            for &site in &reached {
                locals.extend(self.contents[site].locals());
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
    fn exposed_through(&self, local: LocalId) -> Vec<Site> {
        let from = &self.points_to[local];
        if from.beyond().read_or_written_unseen() {
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
    /// `q = **t3` is `q2 = *t3; q = *q2;`, and each step reads a cell's `names`
    /// as a load through a load does, so the two spellings of a **read** cannot
    /// be answered differently, and a **store** written as one place, `**t3 =
    /// r`, is recorded where the level above may be, so it is here to be read.
    /// Every level is a lower bound read beside
    /// [`Reached::Partial`], and steps through a local whose address is
    /// stored as through an allocation ([`Self::level_below`]). See ADR-0045.
    pub(super) fn stored_below(&self, local: LocalId, depth: usize) -> BTreeSet<Site> {
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
    /// nothing (#634). **Its own level too, from the two fields a store of
    /// its value reads**: what a load believes, what a call wrote at what it
    /// was handed as or as its type, and any other value at the width of
    /// what it names (ADR-0061). The pointer read out of the local is that value, and a load
    /// asks the local's own fit where it is read, but a copy stores it into
    /// memory, which a later read believes: `memcpy(m, &c, 8)` with `c` a
    /// `char *` load, read back through an `int **`, was believed whole
    /// where `*cm = c;` was doubted (#711). **Not the store's other
    /// condition**, that its destination may hold a pointer: a byte read out
    /// of memory is a load too, so `memcpy(m, &b, 1)` with `b` such a `char`,
    /// read back as a pointer, is doubted, where `*cm = b;` into a `char *`
    /// records no bound. A pointer read out of bytes a `char` filled is not
    /// shown to be whole, and since ADR-0061 no bound at a load is no belief
    /// either. Found by review. `depth_cap` is [`Allocations::deepest_pointer`](super::allocations::Allocations::deepest_pointer),
    /// which the list that puts it one level down is cut to.
    ///
    /// **The one walk a load, a departure and a copy take**, so that the
    /// spellings of one read cannot believe different things. See
    /// [`Levels`], ADR-0060 and ADR-0061.
    pub(super) fn memory_levels(&self, local: LocalId, depth: usize, depth_cap: usize) -> Levels {
        let mut levels = self.points_to[local].contents_at_most().below(depth - 1);
        for level in 0..depth {
            let (sites, locals) = self.levels_below(local, level);
            for site in sites {
                // **A store that recorded no width at the level read** is
                // `Nothing` there, which absorbs whatever any other record
                // says (#767). See `Cell::poured`.
                levels.bound_by(&self.contents[site].stored_levels().below(depth - 1 - level));
            }
            for slot in locals {
                let own = &self.points_to[slot];
                // The local's own value at the width a store of it records,
                // by the one rule. See `Self::stored_width`.
                let first = self.stored_width(own, Some(slot));
                let held = own.contents_at_most().stored_under(first, depth_cap);
                levels.bound_by(&held.below(depth - 1 - level));
            }
        }
        levels
    }

    /// How wide `held` is recorded as pointing where it is put in memory,
    /// held in `held_in` where it is a local's value: **the one rule a store
    /// and a load of a local's own level read**, so that a copy of the local
    /// carries what a store of it would (#804).
    ///
    /// A load at what it believes, and nothing where it believes nothing,
    /// since a load reading through nothing recorded believes nothing
    /// (ADR-0061). A pointer a call this check cannot read wrote at what it
    /// is believed, or where a join dropped that, at what its local was
    /// handed as (#715), and as its type where it was handed nothing, as
    /// ADR-0017 believes an output; a join with a null keeps the value's
    /// (#719). **Any other value at the width of what it names**: `int *x =
    /// malloc(4); int **pp = &x; int *t = *pp;` reads `t` as `x`'s
    /// allocation is wide, and a null, naming nothing, bounds nothing.
    pub(super) fn stored_width(&self, held: &Held, held_in: Option<LocalIdx>) -> Width {
        if held.beyond().loaded() {
            Width::of_bound(Some(held.believed().unwrap_or(0)))
        } else if held.beyond().foreign() {
            match held.widths.at(0) {
                believed @ (Width::AtMost(_) | Width::AsTyped) => believed,
                Width::Unbounded | Width::Nothing => {
                    match held_in.map(|local| self.handed_at_most[local].at(0)) {
                        Some(handed @ (Width::AtMost(_) | Width::AsTyped)) => handed,
                        Some(Width::Unbounded | Width::Nothing) | None => Width::AsTyped,
                    }
                }
            }
        } else {
            Width::of_bound(held.recorded_width(&self.extent))
        }
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
    /// It takes what [`dereferenced_in_element`](crate::reads::dereferenced_in_element) answers rather than one place,
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
    /// call, for [`Allocations::terminator`](super::allocations::Allocations#method.terminator).
    ///
    /// **`reached` is the caller's**, because the two differ for one place: a
    /// dereference of `*tab` reads through `tab`'s own sites, and `*tab` handed
    /// to a call hands on what they contain ([`Self::handed_reached`]).
    pub(super) fn meeting(&mut self, at: Span, place: &Place, read: Read, reached: &[Reached]) {
        let sites: BTreeSet<Site> = named(reached).collect();

        if sites.is_empty() {
            return;
        }

        // What is reachable already, before the call this read belongs to has
        // done anything: a parameter's allocation from entry, or one stored
        // into an exposed allocation. Starting empty silences a pointer
        // parameter handed to a call before a later one.
        let reachable = self.reachable_now();
        let reachable: BTreeSet<Site> = sites
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

    /// Every site code this check cannot read may reach now through the
    /// memory exposed to it. **Not what a local whose address it holds
    /// points at**, which it can also reach and this does not walk:
    /// [`Self::sites_named_by_unknown_code`] is both (#655).
    ///
    /// **The closure, and not [`Self::exposed`] read as it stands.** That is
    /// closed only inside [`Self::expose`], which runs at a call, so a store of
    /// `a` into an allocation already exposed records `a` in a cell's `names`
    /// and leaves its bit unset until the next call. Its readers run after
    /// that call and are right to trust it; a pending read is recorded, and
    /// told what became reachable, between calls.
    pub(super) fn reachable_now(&self) -> BTreeSet<Site> {
        self.closure(
            self.exposed
                .sites()
                .filter(|&site| self.exposed[site])
                .collect(),
        )
    }

    /// `sites`, and everything a pointer stored in one of them may hold, and
    /// so on.
    pub(super) fn closure(&self, sites: Vec<Site>) -> BTreeSet<Site> {
        let mut closed: BTreeSet<Site> = sites.iter().copied().collect();
        let mut pending = sites;
        while let Some(site) = pending.pop() {
            for held in self.contents[site].sites() {
                if closed.insert(held) {
                    pending.push(held);
                }
            }
        }
        closed
    }

    /// The blocks `sites` name and the blocks they hold, `levels` levels in
    /// all: `sites` themselves at the first, what is stored in them at the
    /// second, and so on, each block once. `usize::MAX` follows what is held
    /// until nothing new is.
    ///
    /// **Or as many as a block's own record says**, where that is more: each
    /// block is followed with the larger of the levels its parent left it and
    /// its [`Self::held_levels`], so a `void *` level, which reads as no
    /// level at all, still reaches a block held as `int **`, and zero levels
    /// still reaches a block whose record says pointers. See ADR-0086.
    ///
    /// **Each block with the most levels any path reaches it with**, widest
    /// first: a block reached through a shallow parent before a deep one kept
    /// the shallow budget, and what it held below went unasked, found by
    /// review. A block's budget never rises along a path except by its own
    /// record, so the first time it is taken off the queue is its widest.
    ///
    /// **One walk for every door a block leaves by**: the store, the copy
    /// and the `return` each ask it with the levels their type reads
    /// pointers at, so one cannot follow a level the others stop at. See
    /// ADR-0085.
    ///
    /// **Bounded by the number of sites as well as by `levels`**: each block
    /// taken off the queue is one not seen before, so no walk takes more, and
    /// a block that holds itself, `*p = p;`, copied out through a `void *`,
    /// ends here even if the filter on what was seen is lost. Without the
    /// bound, losing it hung on that program with nothing to read.
    /// Mutation: drop the filter on what was seen and keep the bound; the
    /// case
    /// `a_block_that_holds_itself_copied_out_through_a_void_pointer_is_walked_once`
    /// still ends.
    pub(super) fn held_below(&self, sites: Vec<Site>, levels: usize) -> Vec<Site> {
        let mut seen: Vec<Site> = Vec::new();
        let mut queue: std::collections::BinaryHeap<(usize, Site)> = sites
            .into_iter()
            .map(|site| (levels.max(self.held_levels[site]), site))
            .collect();
        let mut taken = 0;
        while let Some((budget, site)) = queue.pop() {
            if taken == self.contents.len() {
                break;
            }
            if budget == 0 || seen.contains(&site) {
                continue;
            }
            taken += 1;
            seen.push(site);
            queue.extend(
                self.contents[site]
                    .sites()
                    .map(|held| ((budget - 1).max(self.held_levels[held]), held)),
            );
        }
        seen
    }

    /// Record that a store this check cannot place may have put what it
    /// carries in any memory code it cannot read can name: every exposed
    /// allocation and every allocation reachable from one, since a pointer
    /// this check cannot place may point at any of them.
    ///
    /// Reachable rather than exposed, because a store into exposed memory
    /// records what it stores there and does not expose it: `*a = t;` leaves
    /// `t` unexposed. One function for the store and for the copy `memcpy`
    /// makes, so the two spellings cannot land in different places. See
    /// ADR-0044.
    ///
    /// **Each landing is given `poured`**, the record the door pours into the
    /// containers it can name, with every mark and the bound: a landing is
    /// another place the same store may land, and may hold what it carried
    /// as much as a named container does. Given the sites and locals alone, a
    /// lost value stored through a pointer that may alias other reachable
    /// memory was marked `lost` only where the store named, so its free
    /// left a pointer read back by the other name believed (#705).
    ///
    /// Mutation: give the landings `lost` taken `false`, and
    /// `a_lost_value_stored_through_an_unplaced_pointer_and_freed_is_doubted_where_read_back_by_another_name`
    /// goes silent at the read; `stale_read` taken `false`, and its `memcpy`
    /// spelling does, since the copy's source is a lost local; the bound
    /// dropped, and
    /// `a_narrow_pointer_stored_through_an_unplaced_pointer_is_doubted_where_read_back_wider_by_another_name`
    /// believes a `char` read back as an `int` whole, found by review.
    /// **The caller's and the unreplaced marks decide no case found,
    /// measured**: a landing is memory code this check cannot read can reach,
    /// which every call such code makes already doubts, and each adds a doubt
    /// to what such a call doubts. They are given all the same, so a landing
    /// does not have to be argued about field by field.
    ///
    /// **And the locals whose address it carries**, in what each landing may
    /// contain, which is not exposing them: a load out of that memory reaches
    /// the local, and a write through the load lands in it. `*ppp = &x; t =
    /// *r; *t = m; free(m); q = x; return *q;` under `f(&a, &a)` read `x` as
    /// the null it started as and was believed, found by review. ADR-0047
    /// keeps a local's address out of what is exposed, so that a holder is
    /// still asked; this records where it may be and exempts nobody.
    pub(super) fn landed_where_unknown_code_points(&mut self, poured: &Cell) {
        let landings: Vec<Site> = self.reachable_now().into_iter().collect();
        self.pour(&landings, poured, &[]);
    }

    /// Whether `local` certainly holds a null pointer as far as this value
    /// says: it names no allocation, and nothing else, by
    /// [`Self::names_no_more_than_its_sites`]. Such a pointer is null or
    /// indeterminate, and handing an indeterminate one to a library function
    /// is undefined already, so reading it as null reports nothing a correct
    /// program does. A pointer null on one path only names what the other
    /// path names, since a join unions them, and is not this. See
    /// `parts::Leaves::TheOld`.
    ///
    /// Mutation: drop the test of `sites`, and
    /// `a_pointer_read_out_of_what_realloc_of_a_live_block_returns_is_not_reported`
    /// is `SC0411`.
    pub(super) fn names_nothing(&self, local: LocalId) -> bool {
        !self.points_to[local].holds_a_site() && self.names_no_more_than_its_sites(local)
    }

    /// Whether every allocation `local` may name is unwritten, [`Self::written`], it names
    /// at least one, and it names nothing else, by
    /// [`Self::names_no_more_than_its_sites`]. `realloc` handed it copies
    /// indeterminate bytes into what it returns, which is fresh then, and if
    /// the pointer is null after all it is fresh by C17 7.22.3.5 p3 too.
    ///
    /// **Of the slots `slots` of each, where they are known**, the slots a
    /// library copy reads ([`PlaceInit::over`]); of the whole otherwise,
    /// which is what `realloc` reads.
    ///
    /// Mutation: answer `false`, and
    /// `a_pointer_read_out_of_what_realloc_of_a_block_nothing_wrote_returns_is_reported`
    /// goes silent.
    pub(super) fn names_only_fresh(&self, local: LocalId, slots: Option<(u64, u64)>) -> bool {
        let held = &self.points_to[local];
        held.holds_a_site()
            && held
                .sites()
                .all(|site| self.written[site].over(slots).is_unwritten())
            && self.names_no_more_than_its_sites(local)
    }

    /// Whether any allocation `local` may name may be unwritten,
    /// [`Self::written`]: what `realloc` or a library copy handed it may
    /// copy is then indeterminate in part, whatever else it names (#724).
    /// Of the slots `slots`, as [`Self::names_only_fresh`] asks.
    pub(super) fn may_name_fresh(&self, local: LocalId, slots: Option<(u64, u64)>) -> bool {
        self.points_to[local]
            .sites()
            .any(|site| self.written[site].over(slots).may_be_unwritten())
    }

    /// Whether what `local` may hold is the allocations it names and nothing
    /// else: no local's address, and nothing this check lost, read out of
    /// memory, or that a caller or code this check cannot read may have
    /// written there. Not `Held::writes_elsewhere`, which starts `true` as
    /// the answer that gives up and so holds of a null local too.
    ///
    /// **The tests guard one another**: a pointer read out of memory is
    /// `loaded` and, out of a caller's, `from_caller` as well, and one a call
    /// this check cannot read may have written is `lost` and `foreign`, so
    /// each dropped alone decides no case, measured. Dropped together,
    /// `a_pointer_realloc_is_handed_that_may_name_what_this_check_cannot_is_not_read_as_fresh`
    /// is `SC0411` about two correct programs.
    fn names_no_more_than_its_sites(&self, local: LocalId) -> bool {
        let held = &self.points_to[local];
        !held.may_be_a_locals_address() && held.beyond().is_nothing()
    }

    /// Pour `poured` into each of `containers`, and mark every array in
    /// `in_part` as one a pointer reaching it only by #639's rule was stored
    /// into memory for, whether or not there is a container: a store into a
    /// local through its address is read back without its element too. See
    /// [`Self::stored_in_part`].
    ///
    /// **The one way a door that moves a value into memory writes it**: the
    /// store, the library copy, `realloc` and a store this check cannot
    /// place. The store, the copy and the unplaced store each start from
    /// [`Held::none`] and set what a cell records, its names, its bound and
    /// its marks; `realloc` pours the old allocation's cell whole. What each
    /// door carries is taken in by
    /// [`Cell::absorb`](super::parts::Cell::absorb), as the join takes it,
    /// which names every field of `Held`, so a field added is
    /// `error[E0027]` there until it says what memory keeps of it. A place where an event
    /// rather than a value sets one mark (a site reborn, a slot a call could
    /// not replace, a lost value gone, caller memory at entry) sets that
    /// field directly, and so does a call this check cannot read where it
    /// writes into what it was handed, which records the sites alone.
    pub(super) fn pour(&mut self, containers: &[Site], poured: &Cell, in_part: &[LocalIdx]) {
        for &container in containers {
            self.contents[container].absorb(poured);
        }
        for &array in in_part {
            self.stored_in_part[array] = true;
        }
    }

    /// What a load through `local`, `depth` dereferences deep, takes from the
    /// memory it reads, onto `held`: the sites it may hold, at an offset
    /// nobody said; the locals whose address it may be, as edges, and
    /// whether it may be a pointer that reached an array only by #639's
    /// rule; the element it names, where it is read out of one local's value
    /// that names one local and nothing else; and the stale, unreplaced,
    /// lost and caller marks. Answers
    /// whether it named any local, which `Allocations::read_through` asks.
    ///
    /// **One function for both loads**, `Allocations::read_through` for a
    /// load assigned to a local and `built::built_from` for one that reaches
    /// an operator or a call as an operand, which each kept a copy of what
    /// follows before #687. `is_array` and `from_caller` are the two answers
    /// the caller has from the function and the analysis. See ADR-0040 and
    /// ADR-0045.
    ///
    /// `depth` is at least one: a load reads through a dereference, and the
    /// level a load reads its value out of is `depth - 1`. Both callers hold
    /// that: `read_through` by its own test, and `built_from` because it
    /// takes as a load only a place with at least one dereference.
    pub(super) fn load_into(
        &self,
        held: &mut Held,
        local: LocalId,
        depth: usize,
        is_array: &dyn Fn(LocalIdx) -> bool,
        from_caller: bool,
    ) -> bool {
        let (sites, locals) = self.levels_below(local, depth);
        for site in sites {
            held.hold(site, At::Unknown);
        }
        // **And the locals whose address it may be**, as edges, so a write
        // through it lands there: `m = *t3; *m = r;` with `*t3 = &slot`. Not
        // all of them, since the set is a lower bound. See ADR-0045.
        let named_a_local = !locals.is_empty();
        for target in locals {
            held.name_local(target);
            // And it may be a pointer that reached an array only by #639's
            // rule. See `Known::stored_in_part`.
            if self.stored_in_part[target] {
                held.array_edge = true;
            }
        }
        // Or read out of a local that holds one, which no store put there:
        // `e = m + 1; f = *pe;` with `pe = &e`. Not an array, whose record
        // gathers every store into it, an earlier turn's too: a store into
        // one marks `stored_in_part`, which a turn clears. Found by review of
        // #667.
        if self
            .levels_below(local, depth - 1)
            .1
            .into_iter()
            .any(|read_from| !is_array(read_from) && self.points_to[read_from].array_edge)
        {
            held.array_edge = true;
        }
        // **And the element it names, where it is read out of one local and
        // nothing else**: the value read is that local's, so `f = *pe;` with
        // `pe = &e` and `e = m` names `m[0]` as `f = e;` does, and so does
        // `memcpy(&g, &e, 8)`, a load of the same. Left unnamed, a write
        // through it was a write of the whole array, and `m[1]`, which
        // nothing wrote, was believed written (#706). Not an array read out
        // of, whose record gathers every store into any of its elements, nor
        // a block, which keeps no element (`Cell::absorb`), nor more than
        // one local, whose elements this would have to join.
        //
        // **And only where that local's value names one local and nothing
        // else**, which `!is_array` above has made sure is not an array
        // read out of whole: an element is an index into each array a pointer may
        // name, and a write through it is taken as a write of that element
        // in all of them (`built::write_effects`). With `e` either `m` or
        // `n`, or handed to a call this check cannot read, the element would
        // write `m[0]` for certain on a path where `n[0]` was written, and a
        // read of `m[0]` nothing wrote went silent. Found by review.
        let (from_sites, from_locals) = self.levels_below(local, depth - 1);
        let from_locals: Vec<LocalIdx> = from_locals.into_iter().collect();
        if let (true, [read_from]) = (from_sites.is_empty(), from_locals.as_slice()) {
            let value = &self.points_to[*read_from];
            let mut names = value.locals();
            let one_array = matches!((names.next(), names.next()), (Some(_), None))
                && !value.holds_a_site()
                && value.beyond().is_nothing();
            if !is_array(*read_from) && one_array {
                held.element = value.element;
            }
        }
        // What every block it reads through, at any level, gives what it
        // reads, by the one rule a load out of memory has. See
        // `Cell::taken_by_a_load` and `Self::cells_on_the_way`.
        for site in self.cells_on_the_way(local, depth) {
            self.contents[site].taken_by_a_load(held);
        }
        // Read through a pointer to a local this check lost, which may be
        // gone: the stricter of the two rules a load and a library copy once
        // kept apart, so that a copy can be a load (#777). Taking it for
        // every load moved no case, measured. See ADR-0045.
        if self.lost_through(local, depth) {
            held.lost_and_may_be_gone();
        }
        if from_caller {
            held.handed_over();
        }
        named_a_local
    }

    /// Record that a call this check cannot read was handed the address of
    /// each of `locals`: every local whose address it can find from them, at
    /// any depth, is [`Self::exposed`] from here on.
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
    pub(super) fn expose_locals(&mut self, locals: Vec<LocalIdx>) {
        let mut seeds: BTreeSet<LocalIdx> = locals.into_iter().collect();
        seeds.extend(self.exposed.locals().filter(|&local| self.exposed[local]));
        for local in self.closed_from(self.reachable_now(), seeds).locals {
            self.exposed[local] = true;
        }
    }

    /// Every local code this check cannot read may name now: the walk
    /// [`Self::expose_locals`] takes, from the same seeds, without marking
    /// anything. What a pointer such code gave this function, or memory it
    /// can reach, may hold. **Now, not when a pointer was read**, so a write
    /// through a pointer read out of memory, or one such code wrote, asks
    /// what this answered as it was read or written,
    /// [`Held::named_at_read`]: asked at the write, an address
    /// stored after the read was counted as one the pointer may be, which
    /// silenced a read of a local array it could not reach (#648). See
    /// ADR-0057.
    pub(super) fn locals_named_by_unknown_code(&self) -> BTreeSet<LocalIdx> {
        self.unknown_code_reaches().locals
    }

    /// Every allocation code this check cannot read may reach now: the same
    /// walk as [`Self::locals_named_by_unknown_code`], answered in sites. That is
    /// [`Self::reachable_now`] and what an exposed local points at, at any
    /// depth: `keepp(&slot); slot = b;` puts `b`'s allocation where such code
    /// can find it, though nothing exposed it (#655). See ADR-0041.
    pub(super) fn sites_named_by_unknown_code(&self) -> BTreeSet<Site> {
        self.unknown_code_reaches().sites
    }

    /// The walk [`Self::locals_named_by_unknown_code`] and
    /// [`Self::sites_named_by_unknown_code`] share, from what is exposed and
    /// every exposed local, so the sites and the locals cannot come from two
    /// different walks.
    fn unknown_code_reaches(&self) -> Closed {
        let exposed = self.exposed.locals().filter(|&local| self.exposed[local]);
        self.closed_from(self.reachable_now(), exposed.collect())
    }

    /// `sites` and `locals`, and every site and local whose address may be
    /// found from them through [`Self::level_below`], at any depth. One walk
    /// for the call that exposes and the write and the store that ask, so
    /// they cannot find different memory. Two finite sets that only grow, so
    /// it ends.
    fn closed_from(&self, mut sites: BTreeSet<Site>, mut locals: BTreeSet<LocalIdx>) -> Closed {
        loop {
            let (next_sites, next_locals) = self.level_below(&sites, &locals);
            let before = (sites.len(), locals.len());
            sites.extend(next_sites);
            locals.extend(next_locals);
            if (sites.len(), locals.len()) == before {
                return Closed { sites, locals };
            }
        }
    }

    /// Add to what each of `landed` records as nameable,
    /// [`Held::named_at_read`], what code this check cannot read may name
    /// now, where a store it cannot place may have written that local.
    ///
    /// **After the store exposed what it carries**, which
    /// [`Self::replaced`] runs before: the value it may have put in such a
    /// local may be an address the store itself just made nameable, so
    /// `keep(&t); pq = *c; *pq = m; *t = 0;` with `pq` maybe `&t` recorded a
    /// set without `m` and refused a correct program with a certain
    /// `SC0411`. Found by review (#661). Only a local the store marked
    /// `foreign`, since that is what reads this set as of its write.
    pub(super) fn named_after_a_store(&mut self, landed: &[LocalIdx]) {
        let named = self.locals_named_by_unknown_code();
        for &local in landed {
            if self.points_to[local].beyond().foreign() {
                for &each in &named {
                    self.points_to[local].named_at_read.set(each);
                }
            }
        }
    }

    /// Every local a write this check cannot place may land in, for
    /// [`Self::written`], where the write goes through `dereferences`
    /// dereferences of `local`:
    ///
    /// - at none, `local` itself, `m[0] = p`;
    /// - at one, the locals `local`'s address edges name, `*t = x`;
    /// - at more, the locals the level above may be, `**ppq = x`;
    /// - and every local code this check cannot read may name,
    ///   [`Self::locals_named_by_unknown_code`], where `local` or a pointer the walk
    ///   passes through on the way down was read out of memory, lost, or
    ///   written by such code, since each of those may hold an address this
    ///   function never recorded. **As of the read where only `local`
    ///   itself was read out of memory, or written by such code**,
    ///   [`Held::named_at_read`]: asked at the write, it counted locals whose
    ///   address left only after the read, so `t = *pp; *b = m; *t = 0;` was
    ///   taken to write `m` and an indeterminate `m[0]` was read in silence
    ///   (#648), and `get(&t); *b = m; *t = 0;` the same (#661). A level
    ///   between `local` and the one written is read at the write, and a
    ///   lost local records no point at which it was fixed, so those still
    ///   take what is named now.
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
    /// zero leaves the suite green. And the last part for a level between
    /// `local` and the one written and for a lost pointer leaves it green
    /// dropped too, for each alone and both together. It stands for an
    /// address the edges miss, where its absence would be a false `SC0411`
    /// (#648).
    ///
    /// **Why the edges are not enough for a pointer read out of memory.**
    /// Memory a call wrote records nothing of what it wrote, so in
    /// `keep(&slot, box); slot = m; t = *box[0]; *t = 0;` the write has no
    /// edge to `m` though `t` may be what `slot` held; landing only where
    /// the edges say refused that correct program with a certain `SC0411`.
    /// What was named as the pointer was read is what tells it from the
    /// read above. It costs a table square in the locals, which
    /// `memory::per_block` counts. See ADR-0057.
    ///
    /// **A pointer naming no local lands in none.** One moved off a local by
    /// arithmetic keeps its edge to it, an array (#639) or a local that is
    /// not one, read as an array of one (#647), so `*(&m[1] - 1) = p` writes
    /// `m`, in part, [`Self::written`], and `&slot + 1` and back to
    /// `slot` writes `slot`. The store door and a library copy both ask this,
    /// so the two cannot answer one pointer differently.
    pub(super) fn locals_a_write_may_land_in(
        &self,
        local: LocalId,
        dereferences: usize,
    ) -> Vec<LocalIdx> {
        let mut lands: BTreeSet<LocalIdx> = match dereferences {
            0 => BTreeSet::from([LocalIdx::of(local)]),
            1 => self.written_through(local).into_iter().collect(),
            _ => self.levels_below(local, dereferences - 1).1,
        };
        let held = &self.points_to[local];
        // `Beyond::unrecorded_address` with `loaded` and `foreign` taken
        // apart: keep the two in step.
        if self.unnamed_below(local, dereferences) || held.beyond().lost() {
            lands.extend(self.locals_named_by_unknown_code());
        } else if held.beyond().read_or_written_unseen() {
            lands.extend(held.named_at_read.ones());
        }
        lands.into_iter().collect()
    }

    /// Whether a write through `dereferences` dereferences of `local` goes
    /// through a pointer that may hold an address this function never
    /// recorded: `local` itself, or a level the write reads its way through
    /// between `local` and the one written, read out of memory, lost, or
    /// written by code this check cannot read. A level that is an
    /// allocation counts as memory. Whether a write leaves this function's
    /// sight asks this whole; where it may land asks [`Self::unnamed_below`]
    /// and the pointer's own marks apart, because a pointer itself read out
    /// of memory is answered as of its read (#648), and one a call wrote as
    /// of the call (#661). See ADR-0057 and ADR-0041.
    pub(super) fn through_unnamed(&self, local: LocalId, dereferences: usize) -> bool {
        self.points_to[local].beyond().unrecorded_address()
            || self.unnamed_below(local, dereferences)
    }

    /// The second half of [`Self::through_unnamed`]: whether a level
    /// between `local` and the one written may hold an address this function
    /// never recorded. Apart, because such a level is read at the write and
    /// `local` was read earlier, so [`Self::locals_a_write_may_land_in`]
    /// answers the two as of different points (#648).
    fn unnamed_below(&self, local: LocalId, dereferences: usize) -> bool {
        (0..dereferences.saturating_sub(1)).any(|level| {
            let (sites, locals) = self.levels_below(local, level);
            !sites.is_empty()
                || locals
                    .iter()
                    .any(|&held| self.points_to[held].beyond().unrecorded_address())
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
        routes: &BTreeSet<Site>,
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
            let now: Vec<Site> = entry
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
    pub(super) fn written_through(&self, local: LocalId) -> Vec<LocalIdx> {
        self.points_to
            .indices()
            .filter(|target| self.points_to[local].names_local(*target))
            .collect()
    }

    /// Every local reachable from `from` through [`Held::locals`], at any
    /// depth, `from` included: what a callee handed the address of one of
    /// them can reach by following addresses. A visited bit per local, so a
    /// cycle of addresses or a dense graph of them is walked once each,
    /// rather than once per path. See ADR-0044.
    pub(super) fn beneath(&self, from: impl IntoIterator<Item = LocalIdx>) -> Vec<LocalIdx> {
        let mut seen: Table<LocalIdx, bool> = Table::filled(false, self.points_to.len());
        let mut todo: Vec<LocalIdx> = from.into_iter().collect();
        let mut found = Vec::new();
        while let Some(local) = todo.pop() {
            if std::mem::replace(&mut seen[local], true) {
                continue;
            }
            found.push(local);
            let held = &self.points_to[local];
            todo.extend(held.locals());
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
    pub(super) fn reborn(&mut self, site: Site, made: Option<Span>) {
        let Known {
            points_to,
            state,
            escaped: _,
            // A local's, as `escaped` is, and a site being reborn is not.
            handed_away: _,
            exposed,
            contents,
            realloced,
            // Not fresh, whatever it was, and kept whole: the caller says
            // whether the new allocation is fresh, and makes its slots, since
            // only it knows which function made it and its size.
            written,
            pending,
            calls: _,
            exposed_after_call,
            // About no site.
            lost_exposed: _,
            // Left to the caller, which writes what the new allocation takes
            // on the line after this, since only it knows what made it. A
            // local's size is not about the site.
            extent: _,
            // About a local, not a site.
            handed_at_most: _,
            // About a local as well.
            exposed_in_part: _,
            stored_in_part: _,
            // A local's, as `escaped` is, and a site being reborn is not.
            // **Since it was made, cleared**: about the allocation the site
            // named, whose address may have been stored out, and the new
            // one's has not been. **Ever, kept**, for the reason `left_at`
            // is. See `Known::stored_out`.
            stored_out,
            // **Kept, unlike `stored_out`**: the caller may still hold the
            // allocation the site named before, and whether that was written
            // is what the exit asks. Cleared, a block one turn of a loop let
            // leave unwritten went silent when the next turn allocated again,
            // found by review; kept, a block stored out and filled every turn
            // is doubted at an early return, a doubt the reader sees. See
            // `Known::left_at`.
            left_at: _,
            // Kept, for the reason `left_at` is. See `Known::held_levels`.
            held_levels: _,
            // **Since it was made, cleared; ever, kept**, for the reason
            // `left_at` is: the new allocation has had nothing copied into
            // it, and the caller may hold the old one. See
            // `Known::part_written`.
            part_written,
            // Not a local's, which a site being reborn is not about, and
            // kept for the site, as `names` is: a call may still hold the old
            // allocation's address, and the slots of the new one are not told
            // apart from the old one's. Found by review.
            handed_untyped: _,
        } = self;
        // **What the site's own cell keeps when it names a new
        // allocation**, field by field. See `Cell::kept_when_reborn`.
        contents[site].kept_when_reborn();
        stored_out[site].made_again();
        part_written[site].made_again();

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
        written[site] = PlaceInit::whole_of(Init::Written);
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
        for (other, held) in points_to.enumerated_mut() {
            if other != site.local() {
                if gone && held.has_site(site) {
                    held.lost_a_site_that_may_be_gone();
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
        for row in contents.iter_mut() {
            if gone && row.has_site(site) {
                row.mark_stale_read();
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
    pub(super) fn unproved(&mut self, local: LocalIdx) {
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
        sites: impl IntoIterator<Item = Site>,
        by: Option<Span>,
    ) {
        let sites: Vec<Site> = sites.into_iter().collect();
        let routes = self.closure(sites.clone());
        self.noticed(sources, by, &routes);
        for site in sites {
            self.exposed[site] = true;
            self.believed_written_by_unknown_code(site);
        }
        // One closure, shared with what a pending read is told, so that the
        // two cannot disagree about what an exposed allocation reaches.
        for site in self.reachable_now() {
            self.exposed[site] = true;
            self.believed_written_by_unknown_code(site);
        }
    }

    /// **Exposed memory is believed written** by whatever code this check
    /// cannot read may do with it: `p = malloc(n); fill(p); q = *p;` is the
    /// output-parameter idiom, and reading it as maybe unwritten, `may_write`
    /// in place of this, doubted it across the corpus, measured. A belief at
    /// the boundary, not
    /// a landing this check proved, so it does not take a
    /// [`Certain`] and is named apart from
    /// [`Self::certainly_written`], by the author's decision (#778).
    fn believed_written_by_unknown_code(&mut self, site: Site) {
        self.written[site].set(Init::Written);
    }

    /// Whether `local` holds a pointer read out of fresh memory that nothing
    /// may have replaced: [`Held::fresh_read`](super::parts::Held::fresh_read),
    /// and not a local whose address escaped, since code this check cannot
    /// read may have written a determinate pointer into it since. The one
    /// answer a copy of it and the suppression at a free ask, so that the
    /// doubt a free of what `fill(&t)` wrote raises is kept. See ADR-0057.
    pub(super) fn holds_fresh_read(&self, local: LocalIdx) -> bool {
        self.points_to[local].fresh_read && !self.escaped[local]
    }

    /// Whether a store to `place`, a place of dereferences, may land where code
    /// outside this function may read it, by what the value reaches. Half of
    /// `store::Store::leaves`, which adds where the write's landing is unnamed
    /// or unplaced (#802), and that one answer is what the store door,
    /// `report::stored_away`, and what [`Self::stored_out`] records both read,
    /// so the two orders of a store and a free agree (#650):
    ///
    /// - anywhere at all, where the pointer or a level on the way down may hold
    ///   an address this function never recorded, [`Known::through_unnamed`]:
    ///   `**ppp = x`, and `*pp = x` with `pp` read out of a parameter's memory;
    /// - anywhere at all, where the pointer names nothing this check followed,
    ///   neither an allocation nor a local: one moved off a local that is not
    ///   an array by a non-zero constant (ADR-0019) may be any of them, and
    ///   taking it to land nowhere silenced a free;
    /// - an allocation such code can reach, [`Known::sites_named_by_unknown_code`]:
    ///   one exposed to it, which a parameter's memory is from the function's
    ///   start (ADR-0040), or one a local whose address it holds points at, at
    ///   any depth, `keepp(&slot); slot = b; *b = x;` (#655);
    /// - a local whose address such code holds, directly or through such a
    ///   local, [`Known::locals_named_by_unknown_code`] (#655).
    ///
    /// **The allocations below the first level, at a store of more than one
    /// dereference, decide no case, measured**: such a store reaches
    /// `through_unnamed` or the locals first in every program the corpus has.
    /// And the first two parts each hold `**ppp = x` alone, so only dropping
    /// both silences it.
    pub(super) fn leaves(&self, place: &Place) -> bool {
        let depth = derefs(place);
        if self.through_unnamed(place.local, depth) {
            return true;
        }
        let reachable = self.sites_named_by_unknown_code();
        let into: Vec<Site> = if depth == 1 {
            self.sites_of(place.local).collect()
        } else {
            self.stored_below(place.local, depth - 1)
                .into_iter()
                .collect()
        };
        let lands = self.locals_a_write_may_land_in(place.local, depth);
        (into.is_empty() && lands.is_empty())
            || into.iter().any(|site| reachable.contains(site))
            || {
                let named = self.locals_named_by_unknown_code();
                lands.into_iter().any(|local| named.contains(&local))
            }
    }

    /// What `site`'s block holds at `element`, where that is known and the
    /// block is kept per element, and the block's own [`Self::written`]
    /// otherwise. See [`Self::written`].
    pub(super) fn site_init_at(&self, site: Site, element: Option<u64>) -> Init {
        self.written[site].at(element)
    }

    /// A write landed at `element` of this one allocation's block, kept per
    /// slot: that slot is written, and the block as one is what its slots
    /// say. `false`, and nothing written, where the block is not
    /// kept per element or the element is past it; the caller writes the
    /// block whole then. See [`Self::written`].
    pub(super) fn certainly_written_at(&mut self, at: &Certain, element: u64) -> bool {
        let Some(slots) = self.written[at.site()].slots_mut() else {
            return false;
        };
        let Ok(element) = usize::try_from(element) else {
            return false;
        };
        if element >= slots.len() {
            return false;
        }
        slots.set(element, Init::Written);
        true
    }

    /// **Whether a write of `width` bytes at `offset` covers the whole of
    /// `at`**, so that it writes the place for certain rather than may have
    /// missed part of it: the place's size a constant, the write at least
    /// that wide, and not known to start past its start. A write that wide
    /// that starts anywhere else lands past the place's end, which the
    /// bounds rule reports. A place of no constant size is covered by no
    /// write (#932).
    ///
    /// **The one answer the store and the library copy both ask** (#926):
    /// spelled at each door, the copy kept believing a block it wrote one
    /// slot of written whole after the store had stopped (#925).
    pub(super) fn covers_whole(&self, at: Loc, offset: At, width: Option<u64>) -> bool {
        match self.extent[at] {
            Some(Extent::Exactly(size)) => {
                !matches!(offset, At::Moved | At::Bytes(1..))
                    && width.is_some_and(|width| width >= size)
            }
            Some(Extent::AtLeast(_)) | None => false,
        }
    }

    /// A write landed in this one allocation, so its contents are not
    /// indeterminate on this path. Only a landing that proves one place can
    /// ask, which is what [`Certain`] is. See [`Self::written`].
    pub(super) fn certainly_written(&mut self, at: Certain) {
        self.written[at.site()].set(Init::Written);
    }

    /// Something may have written any allocation: a write this check cannot
    /// place, or a library call it reads only by name. A may-write, since
    /// no landing says which (ADR-0074 rule 5). See [`Self::written`].
    pub(super) fn may_write_all_sites(&mut self) {
        for site in self.written.sites() {
            self.written[site].may_write();
        }
    }

    /// Which locals are certainly unwritten, [`Init::Unwritten`], as of now:
    /// what a write that may land past its target asks of the moment before
    /// it. See `Allocations::written_past_its_end`.
    pub(super) fn unwritten_locals(&self) -> Table<LocalIdx, bool> {
        Table::from_vec(
            self.written
                .locals()
                .map(|local| self.written[local].whole().is_unwritten())
                .collect(),
        )
    }

    /// `at` wrote its local whole, on every path that reaches here: what only
    /// a [`WrittenLocal`] says. A write that may have missed it is
    /// `Init::may_write` instead. See [`Self::written`] and ADR-0074.
    pub(super) fn written_local(&mut self, at: WrittenLocal) {
        let local = at.local();
        self.written[local].set(Init::Written);
    }

    /// What a call this check cannot read is believed to have done to a local
    /// it may reach: written it, the output-parameter idiom (#782). A belief
    /// at the boundary, as [`Self::believed_written_by_unknown_code`] is for
    /// a site, and asked by [`Self::replaced`] for [`WrittenBy::Call`] alone. See
    /// ADR-0074 rule 3.
    fn believed_written_local_by_unknown_code(&mut self, local: LocalIdx) {
        self.written[local].set(Init::Written);
    }

    /// What a write that may land in `target` leaves there: a union of what
    /// it held and `written`, since the write may have missed it. The one
    /// rule for every such write, a store's one dereference and its deeper
    /// targets and a library copy's alike, which had drifted: the copy kept
    /// neither half below (#777).
    ///
    /// **Where it starts is what either value said**, the old one or the
    /// one written, and not unknown, which [`Held::accumulated`] says for a
    /// value built by arithmetic. A local is asked what it holds where it is
    /// reached through its address (ADR-0060), and the value written was
    /// asked where it was stored: made unknown, `**t3 = r;` with `*t3 =
    /// &slot` doubted every later `***t3`. Found by review.
    ///
    /// **The proof goes, whatever the accumulator would say about it.**
    /// [`Held::accumulated`] keeps it while the set does not grow, which is
    /// right where an expression is built from its operands and wrong here:
    /// this write may have replaced the pointer, and then freeing the target
    /// again is not a second free of anything. The sites stay because
    /// keeping them is the conservative direction for a use after free; the
    /// proof goes because keeping *it* is the confident one. See ADR-0024.
    /// **No program observes this, and it is here anyway.** `Held::locals` is
    /// written only where an address is taken, so a target of such a write
    /// has always escaped, and ADR-0017 answers `Reached::Lost` for an
    /// escaped local wherever a report is made, measured: removing it breaks
    /// nothing today. What it would cost if the escape stopped covering it is
    /// a proof about a pointer this write may have replaced.
    pub(super) fn target_union(&mut self, target: LocalIdx, written: &Held) {
        let offset = self.points_to[target].offset.joined(written.offset);
        self.points_to[target].accumulated(written);
        self.points_to[target].offset = offset;
        self.points_to[target].freed = None;
    }

    /// What `writes` leave of the arrays they name, taken before the write
    /// that would clear them whole and handed to
    /// [`Self::array_writes_after`] once it has. See `built::write_effects`.
    pub(super) fn array_writes_before(
        &self,
        writes: Vec<(LocalIdx, ArrayWrite)>,
    ) -> Vec<PendingWrite> {
        writes
            .into_iter()
            .map(|(local, write)| PendingWrite {
                local,
                write,
                written: self.written[local].clone(),
            })
            .collect()
    }

    /// Put back what `pending` saved and write it as each says: one element,
    /// or an element that may be any of them. See
    /// [`Self::written`].
    pub(super) fn array_writes_after(&mut self, pending: Vec<PendingWrite>) {
        for write in pending {
            let mut written = write.written;
            match (&mut written.kept, write.write) {
                // Written, keeping a doubt it held: the write is of one
                // element, or of one nobody can name, and is no write of the
                // rest.
                (Kept::Whole(init), ArrayWrite::Element(_) | ArrayWrite::Covered(_)) => {
                    *init = match *init {
                        Init::Maybe => Init::Maybe,
                        Init::Unwritten | Init::Written => Init::Written,
                    };
                }
                (Kept::Slots(slots), ArrayWrite::Element(element)) => {
                    if let Ok(element) = usize::try_from(element) {
                        if element < slots.len() {
                            slots.set(element, Init::Written);
                        }
                    }
                }
                // The elements a copy covers whole from the start are
                // written, and the rest keep what they held.
                (Kept::Slots(slots), ArrayWrite::Covered(count)) => {
                    let count = usize::try_from(count).unwrap_or(usize::MAX);
                    for element in 0..count.min(slots.len()) {
                        slots.set(element, Init::Written);
                    }
                }
                // No element is certainly unwritten once one nobody can name
                // may have been written, and each may still be.
                (_, ArrayWrite::Maybe) => written.may_write(),
            }
            self.written[write.local] = written;
        }
    }

    /// Whether `local` is certainly unwritten where a read reaches it at
    /// `element`, or anywhere in it where that is not known. See
    /// [`Self::written`].
    pub(super) fn certainly_unwritten(&self, local: LocalIdx, element: Option<u64>) -> bool {
        let Some(elements) = self.written[local].slots() else {
            return self.written[local].whole().is_unwritten();
        };
        match element.and_then(|element| usize::try_from(element).ok()) {
            Some(element) if element < elements.len() => elements.get(element).is_unwritten(),
            Some(_) => false,
            None => elements.all_unwritten(),
        }
    }

    /// Whether `local` may still be unwritten where a read reaches it, so
    /// that the read is a doubt: the element read may be unwritten, or it is
    /// not known and some element may be. A longer array, kept whole, and a
    /// local that is no array ask this only where the read is not known to
    /// be off their start (`at_start`), since a counted position inside a
    /// whole array may read an element nothing wrote (#918's review). See
    /// [`Self::written`].
    ///
    /// **An element nobody can name may be any of them**, whether or not the
    /// array was written in part: `m[c]` after `m[1] = &x;` may read `m[0]`,
    /// and so may `*f` with `f` either `m` or `m + 1`. Asking only where the
    /// array was written in part left both silent, where `main` proved them.
    /// Found by review of #667.
    pub(super) fn may_be_unwritten(
        &self,
        local: LocalIdx,
        element: Option<u64>,
        at_start: bool,
    ) -> bool {
        let Some(elements) = self.written[local].slots() else {
            return self.written[local].whole() == Init::Maybe && at_start;
        };
        match element.and_then(|element| usize::try_from(element).ok()) {
            Some(element) if element < elements.len() => elements.get(element).may_be_unwritten(),
            Some(_) => false,
            None => elements.any_may_be_unwritten(),
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
        for site in self.exposed.sites() {
            if self.exposed[site] {
                self.state[site].doubted();
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
    /// a cell's `unreplaced_read` instead, and a pointer later read out of it is
    /// asked about where it is handed by address, which
    /// [`Self::handed_below`] does. Every such allocation, not only those the
    /// call did not reach: a load out of one it reached is doubted already.
    /// Only for a site this leaves `Reachable`, since a site `Unknown` or
    /// freed is asked about whoever holds it. See ADR-0047.
    pub(super) fn held_out_of_reach(&mut self, live_after: impl Fn(LocalIdx) -> bool) {
        for site in self.state.indices() {
            if self.state[site] != SiteState::Reachable {
                continue;
            }
            for container in self.contents.indices() {
                if self.contents[container].has_site(site) {
                    self.contents[container].mark_unreplaced_read();
                }
            }
            let out_of_reach = self.points_to.indices().any(|holder| {
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
    /// what a slot was given stays in a cell's `names` after the slot
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
            if let Some(target) = self.points_to[place.local].certain_target() {
                self.handed_away[target] = true;
            }
        }
    }

    /// Forget every `realloc` fact naming an old allocation `touched` says
    /// something else may have freed since, so a branch on the result can no
    /// longer give it back as live. See ADR-0039.
    pub(super) fn forget_reallocs_touching(&mut self, touched: impl Fn(Site) -> bool) {
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
    /// lands, and every allocation whose cell is `lost` is `stale_read`, so
    /// a load out of it is one this check stopped following. Marking too much
    /// is a doubt; marking only the local handed over left the other three
    /// spellings read in silence (#503). See ADR-0045.
    pub(super) fn lost_may_be_gone(&mut self) {
        for held in self.points_to.iter_mut() {
            // A value read out of memory a lost value may be in may be it.
            // See `Held::lost_read`.
            if held.beyond().may_hold_a_lost_value() {
                held.lost_and_may_be_gone();
            }
        }
        for contents in &mut self.contents {
            if contents.lost() {
                contents.mark_stale_read();
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
            if held.beyond().is_from_caller() {
                held.lost_and_may_be_gone();
            }
        }
        for contents in &mut self.contents {
            if contents.holds_from_caller() {
                contents.mark_stale_read();
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
    /// **An escaped local that may hold a pointer read out of memory, one it
    /// lost, or one a call this check cannot read wrote, reaches what is
    /// stored**, since a callee handed its address can read that pointer out
    /// of it and nothing here names what it is: `Beyond::unrecorded_address`.
    /// The third moved no case when it was added, measured. See ADR-0040.
    pub(super) fn reach_of(&self, named: impl Iterator<Item = Site>) -> Vec<Site> {
        let mut reach: Vec<Site> = named.collect();
        let mut unnamed = false;
        for (local, &escaped) in self.escaped.enumerated() {
            if escaped {
                reach.extend(self.points_to[local].sites());
                unnamed |= self.points_to[local].beyond().unrecorded_address();
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
    pub(super) fn held_sites(&self) -> Table<Site, bool> {
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
        if self.points_to.iter().any(|held| held.beyond().foreign()) {
            return Table::filled(true, sites);
        }
        let mut held: Table<Site, bool> = Table::filled(false, sites);
        for local in &self.points_to {
            for site in local.sites() {
                held[site] = true;
            }
        }
        for contents in &self.contents {
            for site in contents.sites() {
                held[site] = true;
            }
        }
        // `exposed_after_call` can be all that holds a site at an edge: in
        // `release_all() + (c ? (e = a, e = 0, 1) : 0)` with `e`'s address
        // taken, the arm stores `a` in `e` and overwrites it before the join.
        // Then nothing that can name the site after the join holds it on that
        // arm, so its state there decides no answer. Tried, with no answer
        // changing: that program, and the same with `&&` in place of `?:`.
        for (site, held) in held.enumerated_mut() {
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
    /// allocation is recorded in a cell's `names` or exposed at once, and the
    /// exposed half is read by [`Self::levels_below`] rather than here, for a
    /// load through memory this check cannot place (#484). One read
    /// out of a local through its address is not here and need not be: that
    /// local escaped, so what it holds is in every call's reach already. See
    /// ADR-0040.
    pub(super) fn stored(&self) -> Vec<Site> {
        self.contents
            .indices()
            .filter(|&held| self.contents.iter().any(|row| row.has_site(held)))
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
        for local in self.escaped.indices() {
            self.unproved(local);
        }
    }

    /// **The weak update, not [`Self::replace`]**, which is the one strong
    /// one: this adds to what each local may hold and replaces none of it.
    ///
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
    /// [`Allocations::touching`](super::allocations::Allocations::touching)'s own rule whatever this says.
    ///
    /// **Left alone here, not for good.** A call this check cannot read
    /// marks what it wrote as handed over, [`Held::from_caller`], and the
    /// next such call makes it lost by that route, so the idiom is quiet
    /// only until a later call that may free what it was handed (#394).
    ///
    /// **Which it may have written is a second answer, `may_write`**, about
    /// [`Self::written`] alone. A store this check cannot pin down answers
    /// it with the locals its place may land in, of the type it stores; a
    /// call answers it with what it may reach,
    /// [`Self::exposed`], which is narrower than every escaped local,
    /// so `int **p = m; tick(); q = *p;` reads `m` as nothing wrote it
    /// (#643). Two parameters rather than a second function, so that a new
    /// caller cannot give one answer and forget the other. See ADR-0057.
    pub(super) fn replaced(
        &mut self,
        may_hold: impl Fn(LocalIdx) -> bool,
        may_write: impl Fn(LocalIdx) -> bool,
        writer: WrittenBy,
    ) {
        let named_now = self.locals_named_by_unknown_code();
        for local in self.escaped.indices() {
            if may_hold(local)
                && self.escaped[local]
                && self.points_to[local].sites().next().is_some()
            {
                self.points_to[local].lost_its_allocation();
            }
            // **Whatever it holds**, since the writer may have put a pointer
            // to memory this check does not model there. A local holding no
            // site is given this and not `lost`, which is the output-parameter
            // idiom above: the report does not read it. See ADR-0044.
            if may_hold(local) && self.escaped[local] {
                self.points_to[local].written_by_unknown_code();
            }
            // **And, where it may have been written, what such code could
            // name as it wrote**, added rather than set since it may not have
            // been: a write through it lands there, not in what is named by
            // the time of the write (#661). See `Held::named_at_read`. Only
            // where `may_write`: a local the writer cannot reach keeps what
            // it had, and recording into every escaped local let a store
            // through a pointer no call could have aimed at `t` count as a
            // write of what `t` points at. At a call this is taken after the
            // call exposed what it was handed; a store this check cannot place
            // adds what it exposes afterwards, `Self::named_after_a_store`.
            if may_hold(local) && self.escaped[local] && may_write(local) {
                for &named in &named_now {
                    self.points_to[local].named_at_read.set(named);
                }
            }
            // **And it may have been written**, which is what a write this
            // check cannot read is: `int *q; get(&q); *pq` is the
            // output-parameter idiom, and believing `q` unwritten after it
            // reported a correct program. See `Self::init`. The escape
            // decides nothing here, measured: a mark is read only of a local
            // whose address is taken, which is escaped, so dropping it leaves
            // the suite green. It is kept so that `may_write` answers for the
            // same locals `may_hold` does.
            //
            // **Only for a call**: a store or a library copy this check could
            // not prove one local for may have missed it, and borrowing the
            // call's belief wrote it for certain (#803). See ADR-0074.
            if may_write(local) && self.escaped[local] {
                match writer {
                    WrittenBy::Call => self.believed_written_local_by_unknown_code(local),
                    WrittenBy::StoreOrCopy => {
                        self.written[local].may_write();
                    }
                }
            }
        }
    }
}

/// What [`Known::closed_from`] found: the sites and the locals, by name,
/// since both are indexes and the one taken for the other compiles.
struct Closed {
    sites: BTreeSet<Site>,
    locals: BTreeSet<LocalIdx>,
}

/// Who [`Known::replaced`] answers for, matched without a default arm so
/// that a writer added answers what it does to a local it may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WrittenBy {
    /// A call this check cannot read, believed to write what it may reach
    /// (#782).
    Call,
    /// A store or a library copy this check could not prove one local for,
    /// which may have missed each it may reach. See ADR-0074.
    StoreOrCopy,
}

/// How many innermost elements a local array may have and still be kept
/// per element in [`Known::written`]. A longer one is kept whole,
/// as every array was before #667, so the bits a block holds are bounded.
pub(super) const ELEMENTS_KEPT: u64 = 64;

/// What a write is to one array it may land in, where it is not a write of
/// the whole of it. See `built::write_effects`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ArrayWrite {
    /// The element it lands in is known.
    Element(u64),
    /// It may land in any element. See [`Known::written`].
    Maybe,
    /// It writes this many elements whole from the start, and none of the
    /// rest: a library copy of a length this check reads into an array kept
    /// per element, the length divided by a pointer's width and rounded
    /// down, since an element covered in part is not written (#826). `0`
    /// writes nothing.
    Covered(u64),
}

/// An [`ArrayWrite`] with what the array held before the write, put back
/// after it. See [`Known::array_writes_before`].
pub(super) struct PendingWrite {
    local: LocalIdx,
    write: ArrayWrite,
    written: PlaceInit,
}
