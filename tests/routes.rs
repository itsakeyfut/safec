//! Each belief the memory check keeps about memory, asked once per route a
//! pointer can take into memory, as a matrix generated at test time.
//!
//! A row is a belief, a route, a depth and a path shape. Its test writes two
//! programs, a hazard the belief has to report and a control it must not, and
//! asks the belief's rule of both. **The rule comes from the belief and not
//! from the checker**, so a route the checker answers wrongly is a named test
//! failing rather than a review finding; and **a control is asked as well**,
//! since a rule asked only of the hazard passes a checker that reports every
//! program. Beside the rows, the whole matrix's answers are blessed into one
//! summary, so that a change silencing a route and its plain store together,
//! which every row would still agree with, is a diff somebody reads.
//!
//! The rows are a literal table with a name each, as the corpus is (ADR-0007),
//! and the table is checked against its axes. See ADR-0062.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What memory is believed to hold, and what reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Belief {
    /// How wide a pointer stored there points (ADR-0060, ADR-0061): a
    /// `char`'s address read back as an `int *` and written through.
    Width,
    /// Whether it may be indeterminate (ADR-0057): memory nothing wrote read
    /// back as a pointer.
    Fresh,
    /// Whether a pointer there is one this check stopped following
    /// (ADR-0045): a loop's earlier allocation, put there, freed by the name
    /// it was put there under, read back and used, against the same with
    /// nothing freed. Held in memory, a lost pointer is doubted only once
    /// something may have freed it, through a store, a `realloc` one level
    /// down and a copy out of a block; through a copy out of a local's
    /// address, a call, or a local itself it is doubted with nothing freed,
    /// which `ACCEPTED` lists.
    Lost,
    /// Whether a pointer there may be stale (ADR-0045), as a free leaves it
    /// before it is stored: the loop's lost allocation freed, then put there
    /// and read back, against the same with nothing freed. The mark a store
    /// leaves in what it lands in, the cell's `stale_read`.
    Stale,
    /// Whether a pointer there names a site made again since it was freed
    /// (ADR-0045): a loop frees its allocation each turn and the next turn
    /// puts the last one in the slot, against the same loop freeing nothing.
    /// The mark a rebirth leaves on what held the freed site, which neither
    /// [`Belief::Stale`] nor [`Belief::Lost`] reaches. Mutation: a rebirth
    /// does not mark a stale read on what held the freed site;
    /// `reborn_memcpy_block_d1_both` fails, among eighteen.
    Reborn,
    /// Whether a pointer there was read out of the caller's memory
    /// (ADR-0040): read through a parameter, put there, and read back after
    /// a call this check cannot read, which may free what the caller handed
    /// over, against the same with no call. The mark a store leaves in what
    /// it lands in, the cell's `from_caller`, which no other belief reaches.
    /// Mutation: `Cell::absorb` takes no `from_caller`;
    /// `caller_store_d1_both` fails, among ten.
    Caller,
    /// Whether a pointer there was read out of a slot a call this check
    /// cannot read reached by another route and could not replace
    /// (ADR-0047): an allocation stored in a block, handed to such a call
    /// through another local's address, read back out of the block, put
    /// there, read back and handed on by address, against the same with no
    /// call. Asked by address alone, as `Known::handed_below` asks it. The
    /// cell's `unreplaced_read`, which no other belief reaches. Mutation:
    /// `Cell::absorb` takes no `unreplaced_read`; `unreplaced_store_d1_both`
    /// fails, among twelve.
    Unreplaced,
    /// **The door questions start here**: what a door asks of a write, and
    /// not what a load believes of what it reads back (ADR-0068). Each puts an
    /// allocation `x` into the slot and frees it, nothing is read back, and
    /// its hazard and control differ in one thing, so that the code a row
    /// asks for can only come from the door (#801). For this one and
    /// [`Belief::StoredFreed`] that thing is the one the door asks, whether
    /// code outside the function can read the slot: the hazard hands the
    /// slot's container to a call this check cannot read before the write,
    /// the control does not. [`Belief::LeftBehind`] says its own.
    ///
    /// This one: put there, then freed, which `report::reported` doubts at the
    /// free as stored out (`FreedAfterStore`). Mutation: the free's door never
    /// pushes `FreedAfterStore`; `stored_out_store_d1_both` fails, among all
    /// thirty eight of this belief's rows.
    /// Mutation: a library copy marks nothing it carries stored out (its
    /// `StoreKind::Copy` arm in the transfer answers `false`);
    /// `stored_out_memcpy_block_d1_both` fails, among twelve, the copies out
    /// of a local's address with them.
    StoredOut,
    /// Freed, then put there, which the store's door doubts or proves
    /// (`StoreAfterFree`) only where the slot leaves (ADR-0068 rules 2 and 3).
    /// Mutation: `report::stored_away` asks every store, leaving or not;
    /// `stored_freed_store_d1_both` fails, among twenty four.
    StoredFreed,
    /// Put there in memory only this function reads, freed, and the slot's
    /// container then handed to a call this check cannot read, which doubts
    /// what it holds behind it (`FreedBehindArgument`, ADR-0068 rule 4). Its
    /// control frees nothing, since the container is handed in both.
    /// Mutation: `report::freed_behind` asks nothing;
    /// `left_behind_store_d1_both` fails, among thirty four.
    LeftBehind,
    /// Put there whole, then half of it overwritten by a copy of an `int`'s
    /// four bytes, and the slot's container handed to a call this check
    /// cannot read: the callee reads a pointer made of part of an `int`.
    /// Asked at the copy, `SC0413`, by ADR-0073 rule 4 where the copy's
    /// destination says pointers and by the partial pointer record
    /// (ADR-0087 to ADR-0090) where it does not. Its control copies nothing
    /// over the pointer. The record's own routes, a copy into a block that
    /// is held as pointers only later, are ADR-0092's to add.
    ///
    /// Mutation: `Allocations::copied_pointers` answers `Whole` for every
    /// length; `part_pointer_store_d1_both` fails, among twenty four, every row
    /// at depths one and two. The rows at depth zero, a copy into a local's
    /// own address, are asked by a second door too and are not held by that
    /// mutation alone.
    PartPointer,
    /// **ADR-0073's three beliefs start here.** Each puts a pointer to
    /// pointers, `x`, into a slot by each route, and asks what `x` points at
    /// where it is read back (see `pointer_to_pointers`).
    ///
    /// This one: `x` is the address of a `char` array written at its first
    /// byte alone, read back and loaded through as an `int *`: a local that
    /// holds no pointer starts unwritten, and a write of part of it leaves it
    /// possibly unwritten (ADR-0073 rule 2, #822, #826), `SC0411`. Its
    /// control writes the whole array by a copy of a pointer first.
    ///
    /// Mutation: `Allocations::unwritten_at_entry` keeps only a local that
    /// holds a pointer; `part_local_store_d0_both` fails, among twenty six.
    /// Mutation: `Allocations::reads_partly` leaves out an array that a
    /// pointer read out of memory names, as it did before #919;
    /// `part_local_store_d1_both` fails, among twenty four, every row whose
    /// address has passed through memory.
    PartLocal,
    /// `x` is a block nothing wrote, read back and returned by a function
    /// whose return type points at a pointer: the caller reads pointers out
    /// of it (ADR-0073 rule 3, #823, #828), `SC0411` in the `return`'s words.
    /// Its control writes a pointer into the block first.
    ///
    /// Mutation: `report::report_returned_unwritten` asks nothing;
    /// `returned_store_d1_both` fails, among twenty.
    Returned,
    /// `x` is a block held as an `int **`, converted to a `void *` before the
    /// route puts it there, read back as one and handed to a copy of an
    /// `int`'s four bytes, then `x` handed to a call that reads pointers out
    /// of it: what the value points at holds pointers, so the copy writes
    /// part of one (ADR-0073 rule 4, #818, #830), `SC0413`. No type the copy
    /// sees says so; the value's record does, carried by the route. Its
    /// control copies a whole pointer.
    ///
    /// Mutation: `Allocations::first_level_holds_a_pointer` answers `false`;
    /// `held_pointers_store_d1_both` fails, among twenty four.
    HeldPointers,
}

/// How a pointer gets where it is read back from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    /// `slot = v;`
    Store,
    /// `memcpy(&slot, &v_, 8);`
    Memcpy,
    /// Stored, then what holds it grown by `realloc`.
    Realloc,
    /// A call this check cannot read writes it through an out-parameter.
    /// Asked of width, where the call writes a pointer of its own, and of
    /// freshness, where it writes nothing and is believed to have written
    /// what it was handed, which `ACCEPTED` lists as that belief (#782).
    /// Mutation: `Known::believed_written_by_unknown_code` writes nothing;
    /// the four `fresh_call_*` rows fail, `fresh_call_d1_both` among them,
    /// beside the copy rows the same belief holds.
    Call,
    /// Handed through `void *pass(void *)`, which this check cannot read.
    Ret,
    /// Copied by `memcpy` out of a block holding it, rather than out of a
    /// local's address: `*blk_ = v; memcpy(&slot, blk_, 8);`. The copy that
    /// carries what memory holds, which `Route::Memcpy` from a local does not
    /// reach. Mutation: `Cell::poured` keeps no `lost`;
    /// `lost_memcpy_block_d1_both` fails, among twenty. Mutation: it keeps no
    /// `stale_read`; `stale_memcpy_block_d1_both` fails, among forty eight.
    MemcpyFromBlock,
    /// Half a pointer copied, `memcpy(&slot, b, 4)`, against the whole one:
    /// `CopiedPointers::NoneWhole`. Asked of freshness only. Mutation: drop
    /// the `NoneWhole` and `LastInPart` arms of `copied_pointers`;
    /// `fresh_memcpy_half_d1_both` fails, and so does
    /// `fresh_memcpy_part_d1_both`. Dropping the `NoneWhole` arm alone turns
    /// the half copy's proof into a doubt of the same code, which
    /// only `a_route_is_as_certain_as_its_plain_store` fails.
    MemcpyHalf,
    /// A pointer and a half copied into a block of two, the second read:
    /// `CopiedPointers::LastInPart`. Asked of freshness only, one level down.
    MemcpyPart,
    /// Copied out of a block written on one arm only, against one written on
    /// both: the copy's may-be-unwritten arm. Asked of freshness only.
    /// Mutation: drop the arm that doubts a block from a source that may be
    /// unwritten; `fresh_memcpy_maybe_d1_both` fails.
    MemcpyMaybe,
    /// What a call this check cannot read wrote into a local, stored into a
    /// block and read back from there, which is where the width it was
    /// handed at is asked (`Known::handed_at_most`). Width only, at a local.
    /// **Its control's silence is a belief**: what such a call writes into a
    /// local is believed as its result is, no wider than it was handed as
    /// (ADR-0060), which the plain call route does not show because reading
    /// the escaped local through itself is doubted for the escape.
    /// Mutation: `handed` records no `handed_at_most`;
    /// `width_call_stored_d0_both` fails.
    CallStored,
    /// A call handed a local holding a pointer to pointers through
    /// `char ***`, read two levels down, which is where what it was handed at
    /// below its own level is asked (`Known::handed_contents_at_most`).
    /// Width only, at a local. Mutation: `handed` bounds no
    /// `handed_contents_at_most`; `width_call_deep_d0_both` fails.
    CallDeep,
    /// A `void ***` parameter grown by `realloc` and read two levels down
    /// through an `int ***`, against an `int ***` one: the bound `realloc`
    /// carries from what it is handed. Width only. Mutation: `realloc` bounds
    /// nothing it carries by what it is handed;
    /// `width_realloc_param_d2_both` fails.
    ReallocParam,
    /// Stored into a block, read back out of it, and the read stored into
    /// the slot: `*blk_ = v; int *t_ = *blk_; slot = t_;`. What a load out
    /// of marked memory carries into the next store, which no single store
    /// or copy reaches. Mutation: a load out of memory marked stale is not
    /// marked a stale read; `stale_restored_d1_both` fails. Mutation: a
    /// store of a pointer read out of memory holding a lost one does not
    /// mark what it lands in; `lost_restored_d1_both` fails.
    Restored,
    /// Copied out of a block nothing wrote into the slot, written first, by
    /// a length this check cannot read, `memcpy(&slot, b, k)`, against the
    /// same out of a written block: a copy whose length is not a constant
    /// does not cover its destination, so the slot is a doubt. Freshness
    /// only, in a block. Mutation: `covers` takes an unreadable length as
    /// covering; `a_route_is_as_certain_as_its_plain_store` fails at
    /// `fresh_memcpy_unread_d1_both`, which then proves.
    MemcpyUnread,
    /// One pointer copied out of a block nothing wrote into a block of two
    /// whose second was written, the second read back: what a copy shorter
    /// than its destination leaves is in doubt. Freshness only, in a block.
    /// Mutation: drop a block copy's `&& covers`;
    /// `a_route_is_as_certain_as_its_plain_store` fails at
    /// `fresh_memcpy_short_d1_both`, which then proves.
    MemcpyShort,
    /// The same into a local array of two: the local copy's own `covers`.
    /// Freshness only, at a local. Mutation: drop a local copy's `&& covers`;
    /// `a_route_is_as_certain_as_its_plain_store` fails at
    /// `fresh_memcpy_into_part_d0_both`, which then proves.
    MemcpyIntoPart,
    /// Copied out of a block nothing wrote into `d`, a parameter, after the
    /// slot's block, written, was handed to a call this check cannot read:
    /// `d` may be that block, so it is a doubt. Freshness only, in a block.
    /// Mutation: drop a block copy's `&& alone`;
    /// `a_route_is_as_certain_as_its_plain_store` fails at
    /// `fresh_memcpy_unplaced_d1_both`, which then proves.
    MemcpyUnplaced,
    /// Copied out of a local nothing wrote, `memcpy(&slot, &src_, 8)`,
    /// against one written. Freshness only. Mutation: drop
    /// `locals_unwritten`; `a_route_is_as_certain_as_its_plain_store` fails
    /// at `fresh_memcpy_from_local_d0_both`, a doubt where the store proves.
    MemcpyFromLocal,
    /// The same with the local written on one arm only. Freshness only.
    /// Mutation: drop `locals_maybe`; `fresh_memcpy_from_local_maybe_d0_both`
    /// goes silent.
    MemcpyFromLocalMaybe,
    /// A local array of two written backwards through a pointer moved off
    /// its end, its first read where only its second was written, against
    /// both written. Freshness only, at a local. Mutation: `reads_partly`
    /// drops `stored_through`; `fresh_stored_through_d0_both` fails.
    StoredThrough,
    /// A local array's second element stored as a pointer into a block,
    /// read back and written through, its first read: `box2[0] = a2 + 1;`,
    /// against `box2[0] = a2;`. Freshness only, at a local. Mutation: a load
    /// takes no `array_edge` from `Known::stored_in_part`;
    /// `fresh_element_slot_d0_both` goes silent.
    ElementSlot,
    /// A local's address handed to a call taking `void *` that keeps it, a
    /// pointer's address stored there after, a later call, the local moved
    /// on, and the pointer passed on after another: `SC0413`, against the
    /// same handed typed. Width only, at a local. Mutation: `handed` records
    /// no read-as for `void`; `width_kept_void_d0_both` goes silent.
    KeptVoid,
    /// The same handed as `char *` through `void *`: memory read as bytes,
    /// which a later pointer below may be stored into. Width only, at a
    /// local. Mutation: `handed` records nothing untyped below;
    /// `width_kept_bytes_d0_both` goes silent.
    KeptBytes,
    /// The same with the pointer's address stored into a block the call
    /// kept through `void *`. Width only, in a block. Mutation: `handed_site`
    /// records nothing untyped; `width_kept_block_d0_both` goes silent.
    KeptBlock,
    /// The slot a level below a local reached through that local's address,
    /// `int ***t_ = &m; **t_ = v;`: the same place `Route::Store` writes as
    /// `*m = v;`, two dereferences down. Asked of the door questions only, at
    /// the one depth whose slot has a local above it to take the address of.
    ByAddress,
    /// The same with the level above loaded first, `int ***t_ = &m;
    /// int **q_ = *t_; *q_ = v;`. The load marks what it reads as possibly
    /// holding an address never recorded, so the door took the write as
    /// leaving where [`Route::ByAddress`] did not, until both were settled on
    /// the side that doubts (#802): the two now answer alike, and both are in
    /// `ACCEPTED` for leaving where nothing outside reads.
    ByAddressLoaded,
}

/// Where the slot is: a local, or one or two levels below one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Depth {
    D0,
    D1,
    D2,
}

/// Whether the route runs on every path, or on one arm with a null stored on
/// the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Both,
    One,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    belief: Belief,
    route: Route,
    depth: Depth,
    shape: Shape,
}

/// What the matrix says of one field of the memory check's value, `Known`.
enum Field {
    /// The beliefs whose rows write it through every route.
    Belief(&'static [Belief]),
    /// Why no belief writes it, said where a reader of the matrix looks.
    Exempt(&'static str),
}

/// Why a field has no belief yet. A field listed so is the matrix's own
/// account of what it does not hold, rather than a silence.
const PENDING: &str = "no belief yet: the corpus holds it, and its rows land with the next change to it (ADR-0091 rule 2)";

/// Every field of `Known`, each a belief the matrix writes or an exemption
/// with its reason. A field added to `Known` and not here fails
/// [`every_field_of_the_memory_checks_value_is_a_belief_or_an_exemption`], so a
/// new fact of the memory check lands with its rows or says why not. See
/// ADR-0091 rule 2.
const KNOWN_FIELDS: &[(&str, Field)] = &[
    (
        "points_to",
        Field::Belief(&[
            Belief::Width,
            Belief::Lost,
            Belief::Stale,
            Belief::Caller,
            Belief::Unreplaced,
            Belief::HeldPointers,
        ]),
    ),
    (
        "state",
        Field::Belief(&[
            Belief::Lost,
            Belief::Reborn,
            Belief::StoredFreed,
            Belief::LeftBehind,
        ]),
    ),
    ("escaped", Field::Exempt(PENDING)),
    ("handed_away", Field::Exempt(PENDING)),
    ("exposed", Field::Exempt(PENDING)),
    (
        "contents",
        Field::Belief(&[Belief::Width, Belief::Lost, Belief::Stale, Belief::Caller]),
    ),
    ("realloced", Field::Exempt(PENDING)),
    (
        "init",
        Field::Belief(&[Belief::Fresh, Belief::PartLocal, Belief::Returned]),
    ),
    ("pending", Field::Exempt(PENDING)),
    ("calls", Field::Exempt(PENDING)),
    ("exposed_after_call", Field::Exempt(PENDING)),
    ("lost_exposed", Field::Exempt(PENDING)),
    ("extent", Field::Exempt(PENDING)),
    ("handed_at_most", Field::Exempt(PENDING)),
    ("element_init", Field::Exempt(PENDING)),
    ("exposed_in_part", Field::Exempt(PENDING)),
    ("stored_in_part", Field::Exempt(PENDING)),
    ("stored_out", Field::Belief(&[Belief::StoredOut])),
    ("left_at", Field::Exempt(PENDING)),
    (
        "held_levels",
        Field::Exempt(
            "no row yet: a block held as pointers and reached later through a `void *` (ADR-0086) is asked at a `return` the matrix does not write; `HeldPointers` is held by the value's own record, and no row fails when this is never raised",
        ),
    ),
    ("stored_out_at", Field::Exempt(PENDING)),
    ("part_written", Field::Belief(&[Belief::PartPointer])),
    ("part_written_since_made", Field::Exempt(PENDING)),
    ("handed_untyped", Field::Exempt(PENDING)),
    ("block_elements", Field::Exempt(PENDING)),
];

/// The fields of `Known`, read off its declaration.
fn known_fields() -> Vec<String> {
    let source = include_str!("../crates/safec-ir/src/memory/known.rs");
    let start = source
        .find("pub(super) struct Known {")
        .expect("`Known` is declared in known.rs");
    let body = &source[start..];
    let end = body
        .find(
            "
}",
        )
        .expect("`Known`'s declaration ends");
    body[..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub(super) "))
        .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim().to_owned()))
        .collect()
}

/// Mutation: drop `held_levels`'s row from [`KNOWN_FIELDS`]; this fails and
/// names it. A field added to `Known` fails to compile first, in the
/// exhaustive destructures of the join, the entry and a rebirth, and then
/// here until it has a row. A row left for a field removed fails too, so the
/// table is never a list of what used to be there.
#[test]
fn every_field_of_the_memory_checks_value_is_a_belief_or_an_exemption() {
    let declared = known_fields();
    let listed: Vec<&str> = KNOWN_FIELDS.iter().map(|(name, _)| *name).collect();
    let missing: Vec<&String> = declared
        .iter()
        .filter(|field| !listed.contains(&field.as_str()))
        .collect();
    let stale: Vec<&&str> = listed
        .iter()
        .filter(|field| !declared.iter().any(|declared| declared == **field))
        .collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "fields of `Known` with no row in KNOWN_FIELDS: {missing:?}; rows for no field: {stale:?}"
    );
    for (name, field) in KNOWN_FIELDS {
        match field {
            Field::Belief(beliefs) => assert!(
                !beliefs.is_empty() && beliefs.iter().all(|belief| BELIEFS.contains(belief)),
                "`{name}` names a belief the matrix does not ask"
            ),
            Field::Exempt(why) => assert!(!why.is_empty(), "`{name}` is exempt with no reason"),
        }
    }
}

/// Every value of each axis, in the order [`axis_place`] gives them.
///
/// **Adding a value** is: its variant, its arm in [`axis_place`] and
/// [`name_of`], which `E0004` asks for; its place here, which
/// `every_axis_holds_every_value_once` asks for; for a belief its [`mark`],
/// its template, and its arm in [`program`] and in [`compared_by_depth`],
/// and for a route its arm in every template, [`put`], [`read_from`],
/// [`parameters`] and [`mark`], each an exhaustive `match`, so `E0004`
/// names what is left; its rows in `routes!`
/// and what cannot be written in [`NOT_WRITTEN`], which
/// `the_matrix_is_every_combination_of_its_axes` asks for; each row that
/// disagrees in [`ACCEPTED`] and each less certain than its store in
/// [`UNCERTAIN`]; and a blessing.
const BELIEFS: [Belief; 14] = [
    Belief::Width,
    Belief::Fresh,
    Belief::Lost,
    Belief::Stale,
    Belief::Reborn,
    Belief::Caller,
    Belief::Unreplaced,
    Belief::StoredOut,
    Belief::StoredFreed,
    Belief::LeftBehind,
    Belief::PartPointer,
    Belief::PartLocal,
    Belief::Returned,
    Belief::HeldPointers,
];
const ROUTES: [Route; 26] = [
    Route::Store,
    Route::Memcpy,
    Route::Realloc,
    Route::Call,
    Route::Ret,
    Route::MemcpyFromBlock,
    Route::MemcpyHalf,
    Route::MemcpyPart,
    Route::MemcpyMaybe,
    Route::CallStored,
    Route::CallDeep,
    Route::ReallocParam,
    Route::Restored,
    Route::MemcpyUnread,
    Route::MemcpyShort,
    Route::MemcpyIntoPart,
    Route::MemcpyUnplaced,
    Route::MemcpyFromLocal,
    Route::MemcpyFromLocalMaybe,
    Route::StoredThrough,
    Route::ElementSlot,
    Route::KeptVoid,
    Route::KeptBytes,
    Route::KeptBlock,
    Route::ByAddress,
    Route::ByAddressLoaded,
];
const DEPTHS: [Depth; 3] = [Depth::D0, Depth::D1, Depth::D2];
const SHAPES: [Shape; 2] = [Shape::Both, Shape::One];

/// Where each value stands in its axis's list. A `match`, so that a variant
/// added to an axis does not compile until it is given a place, and the test
/// below fails until the list holds it there.
fn axis_place(row: Row) -> [usize; 4] {
    let belief = match row.belief {
        Belief::Width => 0,
        Belief::Fresh => 1,
        Belief::Lost => 2,
        Belief::Stale => 3,
        Belief::Reborn => 4,
        Belief::Caller => 5,
        Belief::Unreplaced => 6,
        Belief::StoredOut => 7,
        Belief::StoredFreed => 8,
        Belief::LeftBehind => 9,
        Belief::PartPointer => 10,
        Belief::PartLocal => 11,
        Belief::Returned => 12,
        Belief::HeldPointers => 13,
    };
    let route = match row.route {
        Route::Store => 0,
        Route::Memcpy => 1,
        Route::Realloc => 2,
        Route::Call => 3,
        Route::Ret => 4,
        Route::MemcpyFromBlock => 5,
        Route::MemcpyHalf => 6,
        Route::MemcpyPart => 7,
        Route::MemcpyMaybe => 8,
        Route::CallStored => 9,
        Route::CallDeep => 10,
        Route::ReallocParam => 11,
        Route::Restored => 12,
        Route::MemcpyUnread => 13,
        Route::MemcpyShort => 14,
        Route::MemcpyIntoPart => 15,
        Route::MemcpyUnplaced => 16,
        Route::MemcpyFromLocal => 17,
        Route::MemcpyFromLocalMaybe => 18,
        Route::StoredThrough => 19,
        Route::ElementSlot => 20,
        Route::KeptVoid => 21,
        Route::KeptBytes => 22,
        Route::KeptBlock => 23,
        Route::ByAddress => 24,
        Route::ByAddressLoaded => 25,
    };
    let depth = match row.depth {
        Depth::D0 => 0,
        Depth::D1 => 1,
        Depth::D2 => 2,
    };
    let shape = match row.shape {
        Shape::Both => 0,
        Shape::One => 1,
    };
    [belief, route, depth, shape]
}

/// Each axis's list holds every value at the place [`axis_place`] gives it,
/// so a value left out of a list is a failure rather than rows nobody asks
/// for.
///
/// Mutation: drop `Belief::Stale` from `BELIEFS` and shorten it; this fails.
/// Mutation: add a variant to `Belief`, name it in every `match`, and leave
/// it out of `BELIEFS`; this fails on the count.
#[test]
fn every_axis_holds_every_value_once() {
    let any = Row {
        belief: Belief::Width,
        route: Route::Store,
        depth: Depth::D0,
        shape: Shape::Both,
    };
    for (place, belief) in BELIEFS.iter().enumerate() {
        assert_eq!(
            axis_place(Row {
                belief: *belief,
                ..any
            })[0],
            place,
            "{belief:?}"
        );
    }
    for (place, route) in ROUTES.iter().enumerate() {
        assert_eq!(
            axis_place(Row {
                route: *route,
                ..any
            })[1],
            place,
            "{route:?}"
        );
    }
    for (place, depth) in DEPTHS.iter().enumerate() {
        assert_eq!(
            axis_place(Row {
                depth: *depth,
                ..any
            })[2],
            place,
            "{depth:?}"
        );
    }
    for (place, shape) in SHAPES.iter().enumerate() {
        assert_eq!(
            axis_place(Row {
                shape: *shape,
                ..any
            })[3],
            place,
            "{shape:?}"
        );
    }
    assert_eq!(BELIEFS.len(), values(Belief::Width), "BELIEFS");
    assert_eq!(ROUTES.len(), values(Route::Store), "ROUTES");
    assert_eq!(DEPTHS.len(), values(Depth::D0), "DEPTHS");
    assert_eq!(SHAPES.len(), values(Shape::Both), "SHAPES");
}

/// How many values an axis has, from a `match` over one of them that names
/// every variant, so that a variant added does not compile until it is named
/// here beside the count. **What ties a list's length to its enum**: the
/// places above are only asked of the values a list holds, so a value left
/// out of its list and given the next place passed them. Found by review.
trait Axis {
    fn values(self) -> usize;
}

impl Axis for Belief {
    fn values(self) -> usize {
        match self {
            Belief::Width
            | Belief::Fresh
            | Belief::Lost
            | Belief::Stale
            | Belief::Reborn
            | Belief::Caller
            | Belief::Unreplaced
            | Belief::StoredOut
            | Belief::StoredFreed
            | Belief::LeftBehind
            | Belief::PartPointer
            | Belief::PartLocal
            | Belief::Returned
            | Belief::HeldPointers => 14,
        }
    }
}

impl Axis for Route {
    fn values(self) -> usize {
        match self {
            Route::Store
            | Route::Memcpy
            | Route::Realloc
            | Route::Call
            | Route::Ret
            | Route::MemcpyFromBlock
            | Route::MemcpyHalf
            | Route::MemcpyPart
            | Route::MemcpyMaybe
            | Route::CallStored
            | Route::CallDeep
            | Route::ReallocParam
            | Route::Restored
            | Route::MemcpyUnread
            | Route::MemcpyShort
            | Route::MemcpyIntoPart
            | Route::MemcpyUnplaced
            | Route::MemcpyFromLocal
            | Route::MemcpyFromLocalMaybe
            | Route::StoredThrough
            | Route::ElementSlot
            | Route::KeptVoid
            | Route::KeptBytes
            | Route::KeptBlock
            | Route::ByAddress
            | Route::ByAddressLoaded => 26,
        }
    }
}

impl Axis for Depth {
    fn values(self) -> usize {
        match self {
            Depth::D0 | Depth::D1 | Depth::D2 => 3,
        }
    }
}

impl Axis for Shape {
    fn values(self) -> usize {
        match self {
            Shape::Both | Shape::One => 2,
        }
    }
}

fn values(axis: impl Axis) -> usize {
    axis.values()
}

/// One `#[test]` per row, and the roster the axis guard reads, from one
/// literal table, as `cases!` builds the corpus (ADR-0007).
macro_rules! routes {
    ($($name:ident : $belief:ident $route:ident $depth:ident $shape:ident;)*) => {
        const ROWS: &[(&str, Row)] = &[$((stringify!($name), Row {
            belief: Belief::$belief,
            route: Route::$route,
            depth: Depth::$depth,
            shape: Shape::$shape,
        })),*];

        $(
            #[test]
            fn $name() {
                check(stringify!($name), Row {
                    belief: Belief::$belief,
                    route: Route::$route,
                    depth: Depth::$depth,
                    shape: Shape::$shape,
                });
            }
        )*
    };
}

routes! {
    width_store_d0_both: Width Store D0 Both;
    width_store_d0_one: Width Store D0 One;
    width_store_d1_both: Width Store D1 Both;
    width_store_d1_one: Width Store D1 One;
    width_store_d2_both: Width Store D2 Both;
    width_store_d2_one: Width Store D2 One;
    width_memcpy_d0_both: Width Memcpy D0 Both;
    width_memcpy_d0_one: Width Memcpy D0 One;
    width_memcpy_d1_both: Width Memcpy D1 Both;
    width_memcpy_d1_one: Width Memcpy D1 One;
    width_memcpy_d2_both: Width Memcpy D2 Both;
    width_memcpy_d2_one: Width Memcpy D2 One;
    width_realloc_d1_both: Width Realloc D1 Both;
    width_realloc_d1_one: Width Realloc D1 One;
    width_realloc_d2_both: Width Realloc D2 Both;
    width_realloc_d2_one: Width Realloc D2 One;
    width_call_d0_both: Width Call D0 Both;
    width_call_d0_one: Width Call D0 One;
    width_call_d1_both: Width Call D1 Both;
    width_call_d1_one: Width Call D1 One;
    width_call_d2_both: Width Call D2 Both;
    width_call_d2_one: Width Call D2 One;
    width_ret_d0_both: Width Ret D0 Both;
    width_ret_d0_one: Width Ret D0 One;
    width_ret_d1_both: Width Ret D1 Both;
    width_ret_d1_one: Width Ret D1 One;
    width_ret_d2_both: Width Ret D2 Both;
    width_ret_d2_one: Width Ret D2 One;
    width_memcpy_block_d0_both: Width MemcpyFromBlock D0 Both;
    width_memcpy_block_d0_one: Width MemcpyFromBlock D0 One;
    width_memcpy_block_d1_both: Width MemcpyFromBlock D1 Both;
    width_memcpy_block_d1_one: Width MemcpyFromBlock D1 One;
    width_memcpy_block_d2_both: Width MemcpyFromBlock D2 Both;
    width_memcpy_block_d2_one: Width MemcpyFromBlock D2 One;
    width_call_stored_d0_both: Width CallStored D0 Both;
    width_call_deep_d0_both: Width CallDeep D0 Both;
    width_realloc_param_d2_both: Width ReallocParam D2 Both;
    width_restored_d0_both: Width Restored D0 Both;
    width_restored_d0_one: Width Restored D0 One;
    width_restored_d1_both: Width Restored D1 Both;
    width_restored_d1_one: Width Restored D1 One;
    width_restored_d2_both: Width Restored D2 Both;
    width_restored_d2_one: Width Restored D2 One;
    fresh_store_d0_both: Fresh Store D0 Both;
    fresh_store_d0_one: Fresh Store D0 One;
    fresh_store_d1_both: Fresh Store D1 Both;
    fresh_store_d1_one: Fresh Store D1 One;
    fresh_store_d2_both: Fresh Store D2 Both;
    fresh_store_d2_one: Fresh Store D2 One;
    fresh_memcpy_d0_both: Fresh Memcpy D0 Both;
    fresh_memcpy_d0_one: Fresh Memcpy D0 One;
    fresh_memcpy_d1_both: Fresh Memcpy D1 Both;
    fresh_memcpy_d1_one: Fresh Memcpy D1 One;
    fresh_memcpy_d2_both: Fresh Memcpy D2 Both;
    fresh_memcpy_d2_one: Fresh Memcpy D2 One;
    fresh_realloc_d0_both: Fresh Realloc D0 Both;
    fresh_realloc_d0_one: Fresh Realloc D0 One;
    fresh_realloc_d1_both: Fresh Realloc D1 Both;
    fresh_realloc_d1_one: Fresh Realloc D1 One;
    fresh_realloc_d2_both: Fresh Realloc D2 Both;
    fresh_realloc_d2_one: Fresh Realloc D2 One;
    fresh_memcpy_half_d1_both: Fresh MemcpyHalf D1 Both;
    fresh_memcpy_half_d1_one: Fresh MemcpyHalf D1 One;
    fresh_memcpy_half_d2_both: Fresh MemcpyHalf D2 Both;
    fresh_memcpy_half_d2_one: Fresh MemcpyHalf D2 One;
    fresh_memcpy_part_d1_both: Fresh MemcpyPart D1 Both;
    fresh_memcpy_maybe_d0_both: Fresh MemcpyMaybe D0 Both;
    fresh_memcpy_maybe_d1_both: Fresh MemcpyMaybe D1 Both;
    fresh_memcpy_maybe_d2_both: Fresh MemcpyMaybe D2 Both;
    lost_store_d0_both: Lost Store D0 Both;
    lost_store_d0_one: Lost Store D0 One;
    lost_store_d1_both: Lost Store D1 Both;
    lost_store_d1_one: Lost Store D1 One;
    lost_store_d2_both: Lost Store D2 Both;
    lost_store_d2_one: Lost Store D2 One;
    lost_memcpy_d0_both: Lost Memcpy D0 Both;
    lost_memcpy_d0_one: Lost Memcpy D0 One;
    lost_memcpy_d1_both: Lost Memcpy D1 Both;
    lost_memcpy_d1_one: Lost Memcpy D1 One;
    lost_memcpy_d2_both: Lost Memcpy D2 Both;
    lost_memcpy_d2_one: Lost Memcpy D2 One;
    lost_realloc_d1_both: Lost Realloc D1 Both;
    lost_realloc_d1_one: Lost Realloc D1 One;
    lost_realloc_d2_both: Lost Realloc D2 Both;
    lost_realloc_d2_one: Lost Realloc D2 One;
    lost_ret_d0_both: Lost Ret D0 Both;
    lost_ret_d0_one: Lost Ret D0 One;
    lost_ret_d1_both: Lost Ret D1 Both;
    lost_ret_d1_one: Lost Ret D1 One;
    lost_ret_d2_both: Lost Ret D2 Both;
    lost_ret_d2_one: Lost Ret D2 One;
    lost_memcpy_block_d0_both: Lost MemcpyFromBlock D0 Both;
    lost_memcpy_block_d0_one: Lost MemcpyFromBlock D0 One;
    lost_memcpy_block_d1_both: Lost MemcpyFromBlock D1 Both;
    lost_memcpy_block_d1_one: Lost MemcpyFromBlock D1 One;
    lost_memcpy_block_d2_both: Lost MemcpyFromBlock D2 Both;
    lost_memcpy_block_d2_one: Lost MemcpyFromBlock D2 One;
    lost_restored_d0_both: Lost Restored D0 Both;
    lost_restored_d0_one: Lost Restored D0 One;
    lost_restored_d1_both: Lost Restored D1 Both;
    lost_restored_d1_one: Lost Restored D1 One;
    lost_restored_d2_both: Lost Restored D2 Both;
    lost_restored_d2_one: Lost Restored D2 One;
    stale_store_d0_both: Stale Store D0 Both;
    stale_store_d0_one: Stale Store D0 One;
    stale_store_d1_both: Stale Store D1 Both;
    stale_store_d1_one: Stale Store D1 One;
    stale_store_d2_both: Stale Store D2 Both;
    stale_store_d2_one: Stale Store D2 One;
    stale_memcpy_d0_both: Stale Memcpy D0 Both;
    stale_memcpy_d0_one: Stale Memcpy D0 One;
    stale_memcpy_d1_both: Stale Memcpy D1 Both;
    stale_memcpy_d1_one: Stale Memcpy D1 One;
    stale_memcpy_d2_both: Stale Memcpy D2 Both;
    stale_memcpy_d2_one: Stale Memcpy D2 One;
    stale_realloc_d1_both: Stale Realloc D1 Both;
    stale_realloc_d1_one: Stale Realloc D1 One;
    stale_realloc_d2_both: Stale Realloc D2 Both;
    stale_realloc_d2_one: Stale Realloc D2 One;
    stale_ret_d0_both: Stale Ret D0 Both;
    stale_ret_d0_one: Stale Ret D0 One;
    stale_ret_d1_both: Stale Ret D1 Both;
    stale_ret_d1_one: Stale Ret D1 One;
    stale_ret_d2_both: Stale Ret D2 Both;
    stale_ret_d2_one: Stale Ret D2 One;
    stale_memcpy_block_d0_both: Stale MemcpyFromBlock D0 Both;
    stale_memcpy_block_d0_one: Stale MemcpyFromBlock D0 One;
    stale_memcpy_block_d1_both: Stale MemcpyFromBlock D1 Both;
    stale_memcpy_block_d1_one: Stale MemcpyFromBlock D1 One;
    stale_memcpy_block_d2_both: Stale MemcpyFromBlock D2 Both;
    stale_memcpy_block_d2_one: Stale MemcpyFromBlock D2 One;
    stale_restored_d0_both: Stale Restored D0 Both;
    stale_restored_d0_one: Stale Restored D0 One;
    stale_restored_d1_both: Stale Restored D1 Both;
    stale_restored_d1_one: Stale Restored D1 One;
    stale_restored_d2_both: Stale Restored D2 Both;
    stale_restored_d2_one: Stale Restored D2 One;
    reborn_store_d0_both: Reborn Store D0 Both;
    reborn_store_d0_one: Reborn Store D0 One;
    reborn_store_d1_both: Reborn Store D1 Both;
    reborn_store_d1_one: Reborn Store D1 One;
    reborn_store_d2_both: Reborn Store D2 Both;
    reborn_store_d2_one: Reborn Store D2 One;
    reborn_memcpy_d0_both: Reborn Memcpy D0 Both;
    reborn_memcpy_d0_one: Reborn Memcpy D0 One;
    reborn_memcpy_d1_both: Reborn Memcpy D1 Both;
    reborn_memcpy_d1_one: Reborn Memcpy D1 One;
    reborn_memcpy_d2_both: Reborn Memcpy D2 Both;
    reborn_memcpy_d2_one: Reborn Memcpy D2 One;
    reborn_realloc_d1_both: Reborn Realloc D1 Both;
    reborn_realloc_d1_one: Reborn Realloc D1 One;
    reborn_realloc_d2_both: Reborn Realloc D2 Both;
    reborn_realloc_d2_one: Reborn Realloc D2 One;
    reborn_ret_d0_both: Reborn Ret D0 Both;
    reborn_ret_d0_one: Reborn Ret D0 One;
    reborn_ret_d1_both: Reborn Ret D1 Both;
    reborn_ret_d1_one: Reborn Ret D1 One;
    reborn_ret_d2_both: Reborn Ret D2 Both;
    reborn_ret_d2_one: Reborn Ret D2 One;
    reborn_memcpy_block_d0_both: Reborn MemcpyFromBlock D0 Both;
    reborn_memcpy_block_d0_one: Reborn MemcpyFromBlock D0 One;
    reborn_memcpy_block_d1_both: Reborn MemcpyFromBlock D1 Both;
    reborn_memcpy_block_d1_one: Reborn MemcpyFromBlock D1 One;
    reborn_memcpy_block_d2_both: Reborn MemcpyFromBlock D2 Both;
    reborn_memcpy_block_d2_one: Reborn MemcpyFromBlock D2 One;
    reborn_restored_d0_both: Reborn Restored D0 Both;
    reborn_restored_d0_one: Reborn Restored D0 One;
    reborn_restored_d1_both: Reborn Restored D1 Both;
    reborn_restored_d1_one: Reborn Restored D1 One;
    reborn_restored_d2_both: Reborn Restored D2 Both;
    reborn_restored_d2_one: Reborn Restored D2 One;
    fresh_call_d1_both: Fresh Call D1 Both;
    fresh_call_d1_one: Fresh Call D1 One;
    fresh_call_d2_both: Fresh Call D2 Both;
    fresh_call_d2_one: Fresh Call D2 One;
    caller_store_d0_both: Caller Store D0 Both;
    caller_store_d0_one: Caller Store D0 One;
    caller_store_d1_both: Caller Store D1 Both;
    caller_store_d1_one: Caller Store D1 One;
    caller_store_d2_both: Caller Store D2 Both;
    caller_store_d2_one: Caller Store D2 One;
    caller_memcpy_d0_both: Caller Memcpy D0 Both;
    caller_memcpy_d0_one: Caller Memcpy D0 One;
    caller_memcpy_d1_both: Caller Memcpy D1 Both;
    caller_memcpy_d1_one: Caller Memcpy D1 One;
    caller_memcpy_d2_both: Caller Memcpy D2 Both;
    caller_memcpy_d2_one: Caller Memcpy D2 One;
    caller_realloc_d1_both: Caller Realloc D1 Both;
    caller_realloc_d1_one: Caller Realloc D1 One;
    caller_realloc_d2_both: Caller Realloc D2 Both;
    caller_realloc_d2_one: Caller Realloc D2 One;
    caller_ret_d0_both: Caller Ret D0 Both;
    caller_ret_d0_one: Caller Ret D0 One;
    caller_ret_d1_both: Caller Ret D1 Both;
    caller_ret_d1_one: Caller Ret D1 One;
    caller_ret_d2_both: Caller Ret D2 Both;
    caller_ret_d2_one: Caller Ret D2 One;
    caller_memcpy_block_d0_both: Caller MemcpyFromBlock D0 Both;
    caller_memcpy_block_d0_one: Caller MemcpyFromBlock D0 One;
    caller_memcpy_block_d1_both: Caller MemcpyFromBlock D1 Both;
    caller_memcpy_block_d1_one: Caller MemcpyFromBlock D1 One;
    caller_memcpy_block_d2_both: Caller MemcpyFromBlock D2 Both;
    caller_memcpy_block_d2_one: Caller MemcpyFromBlock D2 One;
    caller_restored_d0_both: Caller Restored D0 Both;
    caller_restored_d0_one: Caller Restored D0 One;
    caller_restored_d1_both: Caller Restored D1 Both;
    caller_restored_d1_one: Caller Restored D1 One;
    caller_restored_d2_both: Caller Restored D2 Both;
    caller_restored_d2_one: Caller Restored D2 One;
    unreplaced_store_d0_both: Unreplaced Store D0 Both;
    unreplaced_store_d0_one: Unreplaced Store D0 One;
    unreplaced_store_d1_both: Unreplaced Store D1 Both;
    unreplaced_store_d1_one: Unreplaced Store D1 One;
    unreplaced_store_d2_both: Unreplaced Store D2 Both;
    unreplaced_store_d2_one: Unreplaced Store D2 One;
    unreplaced_memcpy_d0_both: Unreplaced Memcpy D0 Both;
    unreplaced_memcpy_d0_one: Unreplaced Memcpy D0 One;
    unreplaced_memcpy_d1_both: Unreplaced Memcpy D1 Both;
    unreplaced_memcpy_d1_one: Unreplaced Memcpy D1 One;
    unreplaced_memcpy_d2_both: Unreplaced Memcpy D2 Both;
    unreplaced_memcpy_d2_one: Unreplaced Memcpy D2 One;
    unreplaced_realloc_d1_both: Unreplaced Realloc D1 Both;
    unreplaced_realloc_d1_one: Unreplaced Realloc D1 One;
    unreplaced_realloc_d2_both: Unreplaced Realloc D2 Both;
    unreplaced_realloc_d2_one: Unreplaced Realloc D2 One;
    unreplaced_ret_d0_both: Unreplaced Ret D0 Both;
    unreplaced_ret_d0_one: Unreplaced Ret D0 One;
    unreplaced_ret_d1_both: Unreplaced Ret D1 Both;
    unreplaced_ret_d1_one: Unreplaced Ret D1 One;
    unreplaced_ret_d2_both: Unreplaced Ret D2 Both;
    unreplaced_ret_d2_one: Unreplaced Ret D2 One;
    unreplaced_memcpy_block_d0_both: Unreplaced MemcpyFromBlock D0 Both;
    unreplaced_memcpy_block_d0_one: Unreplaced MemcpyFromBlock D0 One;
    unreplaced_memcpy_block_d1_both: Unreplaced MemcpyFromBlock D1 Both;
    unreplaced_memcpy_block_d1_one: Unreplaced MemcpyFromBlock D1 One;
    unreplaced_memcpy_block_d2_both: Unreplaced MemcpyFromBlock D2 Both;
    unreplaced_memcpy_block_d2_one: Unreplaced MemcpyFromBlock D2 One;
    unreplaced_restored_d0_both: Unreplaced Restored D0 Both;
    unreplaced_restored_d0_one: Unreplaced Restored D0 One;
    unreplaced_restored_d1_both: Unreplaced Restored D1 Both;
    unreplaced_restored_d1_one: Unreplaced Restored D1 One;
    unreplaced_restored_d2_both: Unreplaced Restored D2 Both;
    unreplaced_restored_d2_one: Unreplaced Restored D2 One;
    fresh_memcpy_unread_d1_both: Fresh MemcpyUnread D1 Both;
    fresh_memcpy_unread_d1_one: Fresh MemcpyUnread D1 One;
    fresh_memcpy_short_d1_both: Fresh MemcpyShort D1 Both;
    fresh_memcpy_into_part_d0_both: Fresh MemcpyIntoPart D0 Both;
    fresh_memcpy_unplaced_d1_both: Fresh MemcpyUnplaced D1 Both;
    fresh_memcpy_unplaced_d1_one: Fresh MemcpyUnplaced D1 One;
    fresh_memcpy_from_local_d0_both: Fresh MemcpyFromLocal D0 Both;
    fresh_memcpy_from_local_d0_one: Fresh MemcpyFromLocal D0 One;
    fresh_memcpy_from_local_d1_both: Fresh MemcpyFromLocal D1 Both;
    fresh_memcpy_from_local_d1_one: Fresh MemcpyFromLocal D1 One;
    fresh_memcpy_from_local_maybe_d0_both: Fresh MemcpyFromLocalMaybe D0 Both;
    fresh_memcpy_from_local_maybe_d1_both: Fresh MemcpyFromLocalMaybe D1 Both;
    fresh_stored_through_d0_both: Fresh StoredThrough D0 Both;
    fresh_element_slot_d0_both: Fresh ElementSlot D0 Both;
    width_kept_void_d0_both: Width KeptVoid D0 Both;
    width_kept_bytes_d0_both: Width KeptBytes D0 Both;
    width_kept_block_d0_both: Width KeptBlock D0 Both;
    stored_out_store_d0_both: StoredOut Store D0 Both;
    stored_out_store_d0_one: StoredOut Store D0 One;
    stored_out_store_d1_both: StoredOut Store D1 Both;
    stored_out_store_d1_one: StoredOut Store D1 One;
    stored_out_store_d2_both: StoredOut Store D2 Both;
    stored_out_store_d2_one: StoredOut Store D2 One;
    stored_out_memcpy_d0_both: StoredOut Memcpy D0 Both;
    stored_out_memcpy_d0_one: StoredOut Memcpy D0 One;
    stored_out_memcpy_d1_both: StoredOut Memcpy D1 Both;
    stored_out_memcpy_d1_one: StoredOut Memcpy D1 One;
    stored_out_memcpy_d2_both: StoredOut Memcpy D2 Both;
    stored_out_memcpy_d2_one: StoredOut Memcpy D2 One;
    stored_out_realloc_d1_both: StoredOut Realloc D1 Both;
    stored_out_realloc_d1_one: StoredOut Realloc D1 One;
    stored_out_realloc_d2_both: StoredOut Realloc D2 Both;
    stored_out_realloc_d2_one: StoredOut Realloc D2 One;
    stored_out_ret_d0_both: StoredOut Ret D0 Both;
    stored_out_ret_d0_one: StoredOut Ret D0 One;
    stored_out_ret_d1_both: StoredOut Ret D1 Both;
    stored_out_ret_d1_one: StoredOut Ret D1 One;
    stored_out_ret_d2_both: StoredOut Ret D2 Both;
    stored_out_ret_d2_one: StoredOut Ret D2 One;
    stored_out_memcpy_block_d0_both: StoredOut MemcpyFromBlock D0 Both;
    stored_out_memcpy_block_d0_one: StoredOut MemcpyFromBlock D0 One;
    stored_out_memcpy_block_d1_both: StoredOut MemcpyFromBlock D1 Both;
    stored_out_memcpy_block_d1_one: StoredOut MemcpyFromBlock D1 One;
    stored_out_memcpy_block_d2_both: StoredOut MemcpyFromBlock D2 Both;
    stored_out_memcpy_block_d2_one: StoredOut MemcpyFromBlock D2 One;
    stored_out_restored_d0_both: StoredOut Restored D0 Both;
    stored_out_restored_d0_one: StoredOut Restored D0 One;
    stored_out_restored_d1_both: StoredOut Restored D1 Both;
    stored_out_restored_d1_one: StoredOut Restored D1 One;
    stored_out_restored_d2_both: StoredOut Restored D2 Both;
    stored_out_restored_d2_one: StoredOut Restored D2 One;
    stored_freed_store_d0_both: StoredFreed Store D0 Both;
    stored_freed_store_d0_one: StoredFreed Store D0 One;
    stored_freed_store_d1_both: StoredFreed Store D1 Both;
    stored_freed_store_d1_one: StoredFreed Store D1 One;
    stored_freed_store_d2_both: StoredFreed Store D2 Both;
    stored_freed_store_d2_one: StoredFreed Store D2 One;
    stored_freed_memcpy_d0_both: StoredFreed Memcpy D0 Both;
    stored_freed_memcpy_d0_one: StoredFreed Memcpy D0 One;
    stored_freed_memcpy_d1_both: StoredFreed Memcpy D1 Both;
    stored_freed_memcpy_d1_one: StoredFreed Memcpy D1 One;
    stored_freed_memcpy_d2_both: StoredFreed Memcpy D2 Both;
    stored_freed_memcpy_d2_one: StoredFreed Memcpy D2 One;
    stored_freed_realloc_d1_both: StoredFreed Realloc D1 Both;
    stored_freed_realloc_d1_one: StoredFreed Realloc D1 One;
    stored_freed_realloc_d2_both: StoredFreed Realloc D2 Both;
    stored_freed_realloc_d2_one: StoredFreed Realloc D2 One;
    stored_freed_ret_d0_both: StoredFreed Ret D0 Both;
    stored_freed_ret_d0_one: StoredFreed Ret D0 One;
    stored_freed_ret_d1_both: StoredFreed Ret D1 Both;
    stored_freed_ret_d1_one: StoredFreed Ret D1 One;
    stored_freed_ret_d2_both: StoredFreed Ret D2 Both;
    stored_freed_ret_d2_one: StoredFreed Ret D2 One;
    stored_freed_memcpy_block_d0_both: StoredFreed MemcpyFromBlock D0 Both;
    stored_freed_memcpy_block_d0_one: StoredFreed MemcpyFromBlock D0 One;
    stored_freed_memcpy_block_d1_both: StoredFreed MemcpyFromBlock D1 Both;
    stored_freed_memcpy_block_d1_one: StoredFreed MemcpyFromBlock D1 One;
    stored_freed_memcpy_block_d2_both: StoredFreed MemcpyFromBlock D2 Both;
    stored_freed_memcpy_block_d2_one: StoredFreed MemcpyFromBlock D2 One;
    stored_freed_restored_d0_both: StoredFreed Restored D0 Both;
    stored_freed_restored_d0_one: StoredFreed Restored D0 One;
    stored_freed_restored_d1_both: StoredFreed Restored D1 Both;
    stored_freed_restored_d1_one: StoredFreed Restored D1 One;
    stored_freed_restored_d2_both: StoredFreed Restored D2 Both;
    stored_freed_restored_d2_one: StoredFreed Restored D2 One;
    left_behind_store_d0_both: LeftBehind Store D0 Both;
    left_behind_store_d0_one: LeftBehind Store D0 One;
    left_behind_store_d1_both: LeftBehind Store D1 Both;
    left_behind_store_d1_one: LeftBehind Store D1 One;
    left_behind_store_d2_both: LeftBehind Store D2 Both;
    left_behind_store_d2_one: LeftBehind Store D2 One;
    left_behind_memcpy_d0_both: LeftBehind Memcpy D0 Both;
    left_behind_memcpy_d0_one: LeftBehind Memcpy D0 One;
    left_behind_memcpy_d1_both: LeftBehind Memcpy D1 Both;
    left_behind_memcpy_d1_one: LeftBehind Memcpy D1 One;
    left_behind_memcpy_d2_both: LeftBehind Memcpy D2 Both;
    left_behind_memcpy_d2_one: LeftBehind Memcpy D2 One;
    left_behind_realloc_d1_both: LeftBehind Realloc D1 Both;
    left_behind_realloc_d1_one: LeftBehind Realloc D1 One;
    left_behind_realloc_d2_both: LeftBehind Realloc D2 Both;
    left_behind_realloc_d2_one: LeftBehind Realloc D2 One;
    left_behind_ret_d0_both: LeftBehind Ret D0 Both;
    left_behind_ret_d0_one: LeftBehind Ret D0 One;
    left_behind_ret_d1_both: LeftBehind Ret D1 Both;
    left_behind_ret_d1_one: LeftBehind Ret D1 One;
    left_behind_ret_d2_both: LeftBehind Ret D2 Both;
    left_behind_ret_d2_one: LeftBehind Ret D2 One;
    left_behind_memcpy_block_d0_both: LeftBehind MemcpyFromBlock D0 Both;
    left_behind_memcpy_block_d0_one: LeftBehind MemcpyFromBlock D0 One;
    left_behind_memcpy_block_d1_both: LeftBehind MemcpyFromBlock D1 Both;
    left_behind_memcpy_block_d1_one: LeftBehind MemcpyFromBlock D1 One;
    left_behind_memcpy_block_d2_both: LeftBehind MemcpyFromBlock D2 Both;
    left_behind_memcpy_block_d2_one: LeftBehind MemcpyFromBlock D2 One;
    left_behind_restored_d0_both: LeftBehind Restored D0 Both;
    left_behind_restored_d0_one: LeftBehind Restored D0 One;
    left_behind_restored_d1_both: LeftBehind Restored D1 Both;
    left_behind_restored_d1_one: LeftBehind Restored D1 One;
    left_behind_restored_d2_both: LeftBehind Restored D2 Both;
    left_behind_restored_d2_one: LeftBehind Restored D2 One;
    stored_out_by_address_d1_both: StoredOut ByAddress D1 Both;
    stored_out_by_address_d1_one: StoredOut ByAddress D1 One;
    stored_out_by_address_loaded_d1_both: StoredOut ByAddressLoaded D1 Both;
    stored_out_by_address_loaded_d1_one: StoredOut ByAddressLoaded D1 One;
    stored_freed_by_address_d1_both: StoredFreed ByAddress D1 Both;
    stored_freed_by_address_d1_one: StoredFreed ByAddress D1 One;
    stored_freed_by_address_loaded_d1_both: StoredFreed ByAddressLoaded D1 Both;
    stored_freed_by_address_loaded_d1_one: StoredFreed ByAddressLoaded D1 One;
    left_behind_by_address_d1_both: LeftBehind ByAddress D1 Both;
    left_behind_by_address_d1_one: LeftBehind ByAddress D1 One;
    left_behind_by_address_loaded_d1_both: LeftBehind ByAddressLoaded D1 Both;
    left_behind_by_address_loaded_d1_one: LeftBehind ByAddressLoaded D1 One;
    part_pointer_store_d0_both: PartPointer Store D0 Both;
    part_pointer_store_d0_one: PartPointer Store D0 One;
    part_pointer_store_d1_both: PartPointer Store D1 Both;
    part_pointer_store_d1_one: PartPointer Store D1 One;
    part_pointer_store_d2_both: PartPointer Store D2 Both;
    part_pointer_store_d2_one: PartPointer Store D2 One;
    part_pointer_memcpy_d0_both: PartPointer Memcpy D0 Both;
    part_pointer_memcpy_d0_one: PartPointer Memcpy D0 One;
    part_pointer_memcpy_d1_both: PartPointer Memcpy D1 Both;
    part_pointer_memcpy_d1_one: PartPointer Memcpy D1 One;
    part_pointer_memcpy_d2_both: PartPointer Memcpy D2 Both;
    part_pointer_memcpy_d2_one: PartPointer Memcpy D2 One;
    part_pointer_realloc_d1_both: PartPointer Realloc D1 Both;
    part_pointer_realloc_d1_one: PartPointer Realloc D1 One;
    part_pointer_realloc_d2_both: PartPointer Realloc D2 Both;
    part_pointer_realloc_d2_one: PartPointer Realloc D2 One;
    part_pointer_ret_d0_both: PartPointer Ret D0 Both;
    part_pointer_ret_d0_one: PartPointer Ret D0 One;
    part_pointer_ret_d1_both: PartPointer Ret D1 Both;
    part_pointer_ret_d1_one: PartPointer Ret D1 One;
    part_pointer_ret_d2_both: PartPointer Ret D2 Both;
    part_pointer_ret_d2_one: PartPointer Ret D2 One;
    part_pointer_memcpy_block_d0_both: PartPointer MemcpyFromBlock D0 Both;
    part_pointer_memcpy_block_d0_one: PartPointer MemcpyFromBlock D0 One;
    part_pointer_memcpy_block_d1_both: PartPointer MemcpyFromBlock D1 Both;
    part_pointer_memcpy_block_d1_one: PartPointer MemcpyFromBlock D1 One;
    part_pointer_memcpy_block_d2_both: PartPointer MemcpyFromBlock D2 Both;
    part_pointer_memcpy_block_d2_one: PartPointer MemcpyFromBlock D2 One;
    part_pointer_restored_d0_both: PartPointer Restored D0 Both;
    part_pointer_restored_d0_one: PartPointer Restored D0 One;
    part_pointer_restored_d1_both: PartPointer Restored D1 Both;
    part_pointer_restored_d1_one: PartPointer Restored D1 One;
    part_pointer_restored_d2_both: PartPointer Restored D2 Both;
    part_pointer_restored_d2_one: PartPointer Restored D2 One;
    part_pointer_by_address_d1_both: PartPointer ByAddress D1 Both;
    part_pointer_by_address_d1_one: PartPointer ByAddress D1 One;
    part_pointer_by_address_loaded_d1_both: PartPointer ByAddressLoaded D1 Both;
    part_pointer_by_address_loaded_d1_one: PartPointer ByAddressLoaded D1 One;
    part_local_store_d0_both: PartLocal Store D0 Both;
    part_local_store_d0_one: PartLocal Store D0 One;
    part_local_store_d1_both: PartLocal Store D1 Both;
    part_local_store_d1_one: PartLocal Store D1 One;
    part_local_store_d2_both: PartLocal Store D2 Both;
    part_local_store_d2_one: PartLocal Store D2 One;
    part_local_memcpy_d0_both: PartLocal Memcpy D0 Both;
    part_local_memcpy_d0_one: PartLocal Memcpy D0 One;
    part_local_memcpy_d1_both: PartLocal Memcpy D1 Both;
    part_local_memcpy_d1_one: PartLocal Memcpy D1 One;
    part_local_memcpy_d2_both: PartLocal Memcpy D2 Both;
    part_local_memcpy_d2_one: PartLocal Memcpy D2 One;
    part_local_realloc_d1_both: PartLocal Realloc D1 Both;
    part_local_realloc_d1_one: PartLocal Realloc D1 One;
    part_local_realloc_d2_both: PartLocal Realloc D2 Both;
    part_local_realloc_d2_one: PartLocal Realloc D2 One;
    part_local_ret_d0_both: PartLocal Ret D0 Both;
    part_local_ret_d0_one: PartLocal Ret D0 One;
    part_local_ret_d1_both: PartLocal Ret D1 Both;
    part_local_ret_d1_one: PartLocal Ret D1 One;
    part_local_ret_d2_both: PartLocal Ret D2 Both;
    part_local_ret_d2_one: PartLocal Ret D2 One;
    part_local_memcpy_block_d0_both: PartLocal MemcpyFromBlock D0 Both;
    part_local_memcpy_block_d0_one: PartLocal MemcpyFromBlock D0 One;
    part_local_memcpy_block_d1_both: PartLocal MemcpyFromBlock D1 Both;
    part_local_memcpy_block_d1_one: PartLocal MemcpyFromBlock D1 One;
    part_local_memcpy_block_d2_both: PartLocal MemcpyFromBlock D2 Both;
    part_local_memcpy_block_d2_one: PartLocal MemcpyFromBlock D2 One;
    part_local_restored_d0_both: PartLocal Restored D0 Both;
    part_local_restored_d0_one: PartLocal Restored D0 One;
    part_local_restored_d1_both: PartLocal Restored D1 Both;
    part_local_restored_d1_one: PartLocal Restored D1 One;
    part_local_restored_d2_both: PartLocal Restored D2 Both;
    part_local_restored_d2_one: PartLocal Restored D2 One;
    part_local_by_address_d1_both: PartLocal ByAddress D1 Both;
    part_local_by_address_d1_one: PartLocal ByAddress D1 One;
    part_local_by_address_loaded_d1_both: PartLocal ByAddressLoaded D1 Both;
    part_local_by_address_loaded_d1_one: PartLocal ByAddressLoaded D1 One;
    returned_store_d0_both: Returned Store D0 Both;
    returned_store_d0_one: Returned Store D0 One;
    returned_store_d1_both: Returned Store D1 Both;
    returned_store_d1_one: Returned Store D1 One;
    returned_store_d2_both: Returned Store D2 Both;
    returned_store_d2_one: Returned Store D2 One;
    returned_memcpy_d0_both: Returned Memcpy D0 Both;
    returned_memcpy_d0_one: Returned Memcpy D0 One;
    returned_memcpy_d1_both: Returned Memcpy D1 Both;
    returned_memcpy_d1_one: Returned Memcpy D1 One;
    returned_memcpy_d2_both: Returned Memcpy D2 Both;
    returned_memcpy_d2_one: Returned Memcpy D2 One;
    returned_realloc_d1_both: Returned Realloc D1 Both;
    returned_realloc_d1_one: Returned Realloc D1 One;
    returned_realloc_d2_both: Returned Realloc D2 Both;
    returned_realloc_d2_one: Returned Realloc D2 One;
    returned_ret_d0_both: Returned Ret D0 Both;
    returned_ret_d0_one: Returned Ret D0 One;
    returned_ret_d1_both: Returned Ret D1 Both;
    returned_ret_d1_one: Returned Ret D1 One;
    returned_ret_d2_both: Returned Ret D2 Both;
    returned_ret_d2_one: Returned Ret D2 One;
    returned_memcpy_block_d0_both: Returned MemcpyFromBlock D0 Both;
    returned_memcpy_block_d0_one: Returned MemcpyFromBlock D0 One;
    returned_memcpy_block_d1_both: Returned MemcpyFromBlock D1 Both;
    returned_memcpy_block_d1_one: Returned MemcpyFromBlock D1 One;
    returned_memcpy_block_d2_both: Returned MemcpyFromBlock D2 Both;
    returned_memcpy_block_d2_one: Returned MemcpyFromBlock D2 One;
    returned_restored_d0_both: Returned Restored D0 Both;
    returned_restored_d0_one: Returned Restored D0 One;
    returned_restored_d1_both: Returned Restored D1 Both;
    returned_restored_d1_one: Returned Restored D1 One;
    returned_restored_d2_both: Returned Restored D2 Both;
    returned_restored_d2_one: Returned Restored D2 One;
    returned_by_address_d1_both: Returned ByAddress D1 Both;
    returned_by_address_d1_one: Returned ByAddress D1 One;
    returned_by_address_loaded_d1_both: Returned ByAddressLoaded D1 Both;
    returned_by_address_loaded_d1_one: Returned ByAddressLoaded D1 One;
    held_pointers_store_d0_both: HeldPointers Store D0 Both;
    held_pointers_store_d0_one: HeldPointers Store D0 One;
    held_pointers_store_d1_both: HeldPointers Store D1 Both;
    held_pointers_store_d1_one: HeldPointers Store D1 One;
    held_pointers_store_d2_both: HeldPointers Store D2 Both;
    held_pointers_store_d2_one: HeldPointers Store D2 One;
    held_pointers_memcpy_d0_both: HeldPointers Memcpy D0 Both;
    held_pointers_memcpy_d0_one: HeldPointers Memcpy D0 One;
    held_pointers_memcpy_d1_both: HeldPointers Memcpy D1 Both;
    held_pointers_memcpy_d1_one: HeldPointers Memcpy D1 One;
    held_pointers_memcpy_d2_both: HeldPointers Memcpy D2 Both;
    held_pointers_memcpy_d2_one: HeldPointers Memcpy D2 One;
    held_pointers_realloc_d1_both: HeldPointers Realloc D1 Both;
    held_pointers_realloc_d1_one: HeldPointers Realloc D1 One;
    held_pointers_realloc_d2_both: HeldPointers Realloc D2 Both;
    held_pointers_realloc_d2_one: HeldPointers Realloc D2 One;
    held_pointers_ret_d0_both: HeldPointers Ret D0 Both;
    held_pointers_ret_d0_one: HeldPointers Ret D0 One;
    held_pointers_ret_d1_both: HeldPointers Ret D1 Both;
    held_pointers_ret_d1_one: HeldPointers Ret D1 One;
    held_pointers_ret_d2_both: HeldPointers Ret D2 Both;
    held_pointers_ret_d2_one: HeldPointers Ret D2 One;
    held_pointers_memcpy_block_d0_both: HeldPointers MemcpyFromBlock D0 Both;
    held_pointers_memcpy_block_d0_one: HeldPointers MemcpyFromBlock D0 One;
    held_pointers_memcpy_block_d1_both: HeldPointers MemcpyFromBlock D1 Both;
    held_pointers_memcpy_block_d1_one: HeldPointers MemcpyFromBlock D1 One;
    held_pointers_memcpy_block_d2_both: HeldPointers MemcpyFromBlock D2 Both;
    held_pointers_memcpy_block_d2_one: HeldPointers MemcpyFromBlock D2 One;
    held_pointers_restored_d0_both: HeldPointers Restored D0 Both;
    held_pointers_restored_d0_one: HeldPointers Restored D0 One;
    held_pointers_restored_d1_both: HeldPointers Restored D1 Both;
    held_pointers_restored_d1_one: HeldPointers Restored D1 One;
    held_pointers_restored_d2_both: HeldPointers Restored D2 Both;
    held_pointers_restored_d2_one: HeldPointers Restored D2 One;
    held_pointers_by_address_d1_both: HeldPointers ByAddress D1 Both;
    held_pointers_by_address_d1_one: HeldPointers ByAddress D1 One;
    held_pointers_by_address_loaded_d1_both: HeldPointers ByAddressLoaded D1 Both;
    held_pointers_by_address_loaded_d1_one: HeldPointers ByAddressLoaded D1 One;
}

/// A combination of the axes no program is written for, and why.
struct Unwritten {
    route: Route,
    depth: Option<Depth>,
    belief: Option<Belief>,
    shape: Option<Shape>,
    why: &'static str,
}

/// The combinations no program is written for, each with why: a row that is
/// neither here nor in the table fails
/// `the_matrix_is_every_combination_of_its_axes`, and so does an entry with
/// no reason, which excuses nothing.
///
/// `Route::Realloc` at `Depth::D0` is unwritten for the beliefs that grow
/// what holds the slot, since a local is not grown; `fresh` grows the block
/// it reads from instead, so its rows at a local are written.
const NOT_WRITTEN: &[Unwritten] = &[
    Unwritten {
        route: Route::Call,
        depth: Some(Depth::D0),
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a local slot is made with a value, so a call handed its address is handed nothing fresh",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Caller),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "a call writes a value of its own, not the one read out of the caller's memory",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "a call writes a value of its own, not the one read out of the slot",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Width),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Lost),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Stale),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "a call writes a value of its own, not the loop's lost one",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "a call writes a value of its own, not the loop's freed one",
    },
    Unwritten {
        route: Route::Ret,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a value handed through a call is a pointer written into the slot, not memory nothing wrote",
    },
    Unwritten {
        route: Route::MemcpyFromBlock,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "freshness's copy is already out of a block",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::Width),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::Width),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::Width),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: Some(Depth::D0),
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a local starts as a null here, so half a pointer copied over it is not memory nothing wrote",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: Some(Depth::D0),
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a block of two pointers is memory, one level below a local",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: Some(Depth::D2),
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a block of two pointers is asked one level below a local only",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: Some(Shape::One),
        why: "the part the copy misses is the shape this row asks",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: Some(Shape::One),
        why: "the arm the source is written on is the shape this row asks",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::Width),
        shape: Some(Shape::One),
        why: "the row asks one route on every path; the one-arm shape is the plain route's",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::Width),
        shape: Some(Shape::One),
        why: "the row asks one route on every path; the one-arm shape is the plain route's",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::Width),
        shape: Some(Shape::One),
        why: "the row asks one route on every path; the one-arm shape is the plain route's",
    },
    Unwritten {
        route: Route::CallStored,
        depth: Some(Depth::D1),
        belief: Some(Belief::Width),
        shape: None,
        why: "at a level below a local the slot is memory already, which the plain call route asks",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: Some(Depth::D1),
        belief: Some(Belief::Width),
        shape: None,
        why: "the two levels the row reads are below the local it is handed",
    },
    Unwritten {
        route: Route::CallStored,
        depth: Some(Depth::D2),
        belief: Some(Belief::Width),
        shape: None,
        why: "at a level below a local the slot is memory already, which the plain call route asks",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: Some(Depth::D2),
        belief: Some(Belief::Width),
        shape: None,
        why: "the two levels the row reads are below the local it is handed",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: Some(Depth::D0),
        belief: Some(Belief::Width),
        shape: None,
        why: "the parameter is read two levels down, which is the depth it is written at",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: Some(Depth::D1),
        belief: Some(Belief::Width),
        shape: None,
        why: "the parameter is read two levels down, which is the depth it is written at",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a call writes a value of its own, not the loop's freed one",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a call writes a value of its own, not the loop's freed one",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a call writes a value of its own, not the loop's freed one",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a call writes a value of its own, not the loop's freed one",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::Restored,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a restore loads what it stores, which the store row already asks of freshness",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::StoredOut),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::StoredOut),
        shape: None,
        why: "a call writes a value of its own, not the allocation the door question frees",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::StoredOut),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::StoredOut),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::StoredOut),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::StoredOut),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::StoredOut),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::StoredOut),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::StoredFreed),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::StoredFreed),
        shape: None,
        why: "a call writes a value of its own, not the allocation the door question frees",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::StoredFreed),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::StoredFreed),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::StoredFreed),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::StoredFreed),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::StoredFreed),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::StoredFreed),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::LeftBehind),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::LeftBehind),
        shape: None,
        why: "a call writes a value of its own, not the allocation the door question frees",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::LeftBehind),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::LeftBehind),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::LeftBehind),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::LeftBehind),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::LeftBehind),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::LeftBehind),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: Some(Depth::D0),
        belief: None,
        shape: None,
        why: "a local slot has no local above it to take the address of",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: Some(Depth::D2),
        belief: None,
        shape: None,
        why: "the slot two levels down has a block above it, not the local whose address is taken",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: None,
        belief: Some(Belief::Width),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddress,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: Some(Depth::D0),
        belief: None,
        shape: None,
        why: "a local slot has no local above it to take the address of",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: Some(Depth::D2),
        belief: None,
        shape: None,
        why: "the slot two levels down has a block above it, not the local whose address is taken",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: None,
        belief: Some(Belief::Width),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: None,
        belief: Some(Belief::Reborn),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: None,
        belief: Some(Belief::Caller),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::ByAddressLoaded,
        depth: None,
        belief: Some(Belief::Unreplaced),
        shape: None,
        why: "a write through a local's address is asked of the door questions, where #802 found the loaded spelling apart",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::PartPointer),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::PartPointer),
        shape: None,
        why: "a call writes a value of its own, not the allocation the door question frees",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::PartPointer),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::PartPointer),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::PartPointer),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::PartPointer),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::PartPointer),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::PartPointer),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::PartLocal),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::PartLocal),
        shape: None,
        why: "a call writes a pointer of its own, not the `x` this belief asks what it points at",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::PartLocal),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::PartLocal),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::PartLocal),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::PartLocal),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::PartLocal),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::PartLocal),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Returned),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Returned),
        shape: None,
        why: "a call writes a pointer of its own, not the `x` this belief asks what it points at",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::Returned),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::Returned),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::Returned),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::Returned),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::Returned),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::Returned),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::HeldPointers),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::HeldPointers),
        shape: None,
        why: "a call writes a pointer of its own, not the `x` this belief asks what it points at",
    },
    Unwritten {
        route: Route::MemcpyHalf,
        depth: None,
        belief: Some(Belief::HeldPointers),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyPart,
        depth: None,
        belief: Some(Belief::HeldPointers),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::MemcpyMaybe,
        depth: None,
        belief: Some(Belief::HeldPointers),
        shape: None,
        why: "a copy's length and its source's writing are asked of freshness, the belief they decide",
    },
    Unwritten {
        route: Route::CallStored,
        depth: None,
        belief: Some(Belief::HeldPointers),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::CallDeep,
        depth: None,
        belief: Some(Belief::HeldPointers),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
    Unwritten {
        route: Route::ReallocParam,
        depth: None,
        belief: Some(Belief::HeldPointers),
        shape: None,
        why: "what a call or realloc is handed at is a width, and asked of width",
    },
];

/// Which side of a row disagrees with its belief's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// The hazard is not reported: a silence.
    Hazard,
    /// The control is reported too: a doubt about a program that has none.
    Control,
    /// Both.
    Both,
}

/// Why a row is let disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// A false report on a control that is defined C, or a silence, that an
    /// open issue or a known cost answers for: the row agreeing is a fix,
    /// and removes its entry.
    Cost(&'static str),
    /// A report a record takes on purpose, because the control is not
    /// provable from the unit by this check: it hands the pointer to a call
    /// this check cannot read, or reads one the check stopped following.
    /// **The row agreeing is a regression**, the check saying safe about
    /// what it did not prove, unless that record is superseded.
    Belief(&'static str),
}

// The reasons `ACCEPTED` gives, once each.
const VOID_RESULT: &str =
    "a `void *` a call this check cannot read returns believes no width (ADR-0060)";
const ESCAPED_OUTPUT: &str = "a call this check cannot read writes the local through its address, and ADR-0060 doubts what it wrote where the local is read through itself, which it says the output-parameter idiom pays";
const MAY_FREE: &str =
    "a pointer handed to a call this check cannot read may be freed by it (ADR-0039)";
const LOST_READ: &str =
    "a lost pointer read out of a local proves nothing where it is used (ADR-0045)";
const LOST_COPIED: &str =
    "a lost pointer copied by memcpy proves nothing where it is read back (ADR-0045)";
const LOST_HANDED: &str = "a lost pointer handed through a call this check cannot read proves nothing where it is used (ADR-0045)";
const OLD_BLOCK: &str = "the old block a store replaced in memory is still taken to be there after realloc moves it (#656)";
const REPORTED_ELSEWHERE: &str = "the free and the call a freed pointer is handed to are each reported where they happen (SC0401, SC0407), and not again at its use; agreeing would report the use too";
const WRITES_WHAT_IT_IS_HANDED: &str = "a call this check cannot read is believed to write what it is handed, which `docs/safety-model.md` states beside the other boundary beliefs; not believing it refused correct programs that rely on the callee writing (#782)";
const DEEP_COPY_LOSES_A_LOCAL: &str = "a copy two levels down loses the address of a local it carries, where the store spelling keeps it, so what is read through it is asked nothing (#686)";
const POSITION_LOST: &str = "a local kept whole that may be unwritten is asked only where the pointer is known to point at its start, and one read out of memory and stored two levels down is not known to (#703)";
const RETURNED_WRITTEN: &str = "a block written before it is returned is reported as one nothing wrote once its pointer went through another block, a write two levels down or a local's address (#920)";
const VOID_READ_BACK: &str = "a `void *` read out of memory is not believed to start an object where it is stored again, though the record says what it holds (#631)";
// The door questions' reasons (#801).
const DEEP_WRITE_LEAVES: &str = "a write more than one dereference below the pointer it goes through is taken as landing where this check cannot place it, and so as leaving, into memory only this function reads too, which both spellings of it answer alike (ADR-0071 rule 2, #802)";
const ESCAPED_SOURCE: &str = "the address a copy takes of its source local escapes it (ADR-0017), so what the local holds is doubted behind a later call with nothing freed (#345)";
const ESCAPED_HOLDER: &str = "the address of `m` taken makes `m` itself doubted where it is handed (ADR-0017), and what lies behind a pointer is asked only where the pointer itself drew nothing (ADR-0068)";
// End of the reasons `ACCEPTED` gives.

/// The rows whose answer is not their belief's rule, each with the issue or
/// the record that says why. **Exact in both directions**: a row listed here
/// that now agrees fails, and what that means is the entry's [`Why`].
///
/// **A row listed on [`Side::Control`] no longer asks its hazard**: the
/// control already carries the belief's code, so the hazard's report cannot
/// be told from it. Such a row holds only that the route is not silent
/// altogether, and the blessed summary is what notices more. See ADR-0062.
const ACCEPTED: &[(&str, Side, Why)] = &[
    (
        "fresh_call_d1_both",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "fresh_call_d1_one",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "fresh_call_d2_both",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "fresh_call_d2_one",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    ("caller_ret_d0_both", Side::Control, Why::Belief(MAY_FREE)),
    ("caller_ret_d0_one", Side::Control, Why::Belief(MAY_FREE)),
    ("caller_ret_d1_both", Side::Control, Why::Belief(MAY_FREE)),
    ("caller_ret_d1_one", Side::Control, Why::Belief(MAY_FREE)),
    ("caller_ret_d2_both", Side::Control, Why::Belief(MAY_FREE)),
    ("caller_ret_d2_one", Side::Control, Why::Belief(MAY_FREE)),
    (
        "caller_realloc_d2_both",
        Side::Control,
        Why::Cost(OLD_BLOCK),
    ),
    ("caller_realloc_d2_one", Side::Control, Why::Cost(OLD_BLOCK)),
    (
        "unreplaced_ret_d0_both",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "unreplaced_ret_d0_one",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "unreplaced_ret_d1_both",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "unreplaced_ret_d1_one",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "unreplaced_ret_d2_both",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "unreplaced_ret_d2_one",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "unreplaced_realloc_d2_both",
        Side::Control,
        Why::Cost(OLD_BLOCK),
    ),
    (
        "unreplaced_realloc_d2_one",
        Side::Control,
        Why::Cost(OLD_BLOCK),
    ),
    (
        "part_pointer_ret_d0_both",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "part_pointer_ret_d0_one",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "part_pointer_ret_d1_both",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "part_pointer_ret_d1_one",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "part_pointer_ret_d2_both",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "part_pointer_ret_d2_one",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "held_pointers_restored_d1_both",
        Side::Control,
        Why::Cost(VOID_READ_BACK),
    ),
    (
        "held_pointers_restored_d1_one",
        Side::Control,
        Why::Cost(VOID_READ_BACK),
    ),
    (
        "held_pointers_ret_d0_both",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "held_pointers_ret_d0_one",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "held_pointers_ret_d1_both",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "held_pointers_ret_d1_one",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "held_pointers_ret_d2_both",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "held_pointers_ret_d2_one",
        Side::Control,
        Why::Belief(VOID_RESULT),
    ),
    (
        "part_local_memcpy_block_d2_both",
        Side::Hazard,
        Why::Cost(DEEP_COPY_LOSES_A_LOCAL),
    ),
    (
        "part_local_memcpy_block_d2_one",
        Side::Hazard,
        Why::Cost(DEEP_COPY_LOSES_A_LOCAL),
    ),
    (
        "part_local_memcpy_d2_both",
        Side::Hazard,
        Why::Cost(DEEP_COPY_LOSES_A_LOCAL),
    ),
    (
        "part_local_memcpy_d2_one",
        Side::Hazard,
        Why::Cost(DEEP_COPY_LOSES_A_LOCAL),
    ),
    (
        "part_local_restored_d2_both",
        Side::Hazard,
        Why::Cost(POSITION_LOST),
    ),
    (
        "part_local_restored_d2_one",
        Side::Hazard,
        Why::Cost(POSITION_LOST),
    ),
    (
        "part_local_ret_d0_both",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "part_local_ret_d1_both",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "part_local_ret_d1_one",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "part_local_ret_d2_both",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "part_local_ret_d2_one",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "returned_by_address_d1_both",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_by_address_d1_one",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_by_address_loaded_d1_both",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_by_address_loaded_d1_one",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_memcpy_block_d2_both",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_memcpy_block_d2_one",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_memcpy_d2_both",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_memcpy_d2_one",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_realloc_d2_both",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_realloc_d2_one",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_restored_d1_both",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_restored_d1_one",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_restored_d2_both",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_restored_d2_one",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_ret_d0_both",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "returned_ret_d0_one",
        Side::Both,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_ret_d1_both",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "returned_ret_d1_one",
        Side::Both,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_ret_d2_both",
        Side::Hazard,
        Why::Belief(WRITES_WHAT_IT_IS_HANDED),
    ),
    (
        "returned_ret_d2_one",
        Side::Both,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_store_d2_both",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    (
        "returned_store_d2_one",
        Side::Control,
        Why::Cost(RETURNED_WRITTEN),
    ),
    ("width_ret_d0_both", Side::Control, Why::Belief(VOID_RESULT)),
    ("width_ret_d0_one", Side::Control, Why::Belief(VOID_RESULT)),
    ("width_ret_d1_both", Side::Control, Why::Belief(VOID_RESULT)),
    ("width_ret_d1_one", Side::Control, Why::Belief(VOID_RESULT)),
    ("width_ret_d2_both", Side::Control, Why::Belief(VOID_RESULT)),
    ("width_ret_d2_one", Side::Control, Why::Belief(VOID_RESULT)),
    (
        "width_call_d0_both",
        Side::Control,
        Why::Belief(ESCAPED_OUTPUT),
    ),
    (
        "width_call_d0_one",
        Side::Control,
        Why::Belief(ESCAPED_OUTPUT),
    ),
    ("lost_ret_d2_both", Side::Control, Why::Belief(MAY_FREE)),
    ("lost_ret_d2_one", Side::Control, Why::Belief(MAY_FREE)),
    ("lost_ret_d0_both", Side::Control, Why::Belief(LOST_HANDED)),
    ("lost_ret_d0_one", Side::Control, Why::Belief(LOST_HANDED)),
    ("stale_ret_d2_both", Side::Control, Why::Belief(MAY_FREE)),
    ("stale_ret_d2_one", Side::Control, Why::Belief(MAY_FREE)),
    ("stale_ret_d0_both", Side::Control, Why::Belief(LOST_HANDED)),
    ("stale_ret_d0_one", Side::Control, Why::Belief(LOST_HANDED)),
    (
        "stale_ret_d1_both",
        Side::Hazard,
        Why::Cost(REPORTED_ELSEWHERE),
    ),
    (
        "stale_ret_d1_one",
        Side::Hazard,
        Why::Cost(REPORTED_ELSEWHERE),
    ),
    (
        "stale_memcpy_d0_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "stale_memcpy_d0_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "stale_memcpy_d1_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "stale_memcpy_d1_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "stale_memcpy_d2_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "stale_memcpy_d2_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    ("stale_realloc_d2_both", Side::Control, Why::Cost(OLD_BLOCK)),
    ("stale_realloc_d2_one", Side::Control, Why::Cost(OLD_BLOCK)),
    ("stale_store_d0_both", Side::Control, Why::Belief(LOST_READ)),
    ("stale_store_d0_one", Side::Control, Why::Belief(LOST_READ)),
    ("lost_realloc_d2_both", Side::Control, Why::Cost(OLD_BLOCK)),
    ("lost_realloc_d2_one", Side::Control, Why::Cost(OLD_BLOCK)),
    ("lost_store_d0_both", Side::Control, Why::Belief(LOST_READ)),
    ("lost_store_d0_one", Side::Control, Why::Belief(LOST_READ)),
    (
        "lost_memcpy_d0_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "lost_memcpy_d0_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "lost_memcpy_d1_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "lost_memcpy_d1_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "lost_memcpy_d2_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "lost_memcpy_d2_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "reborn_memcpy_d0_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "reborn_memcpy_d0_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "reborn_memcpy_d1_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "reborn_memcpy_d1_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "reborn_memcpy_d2_both",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "reborn_memcpy_d2_one",
        Side::Control,
        Why::Belief(LOST_COPIED),
    ),
    (
        "reborn_store_d0_both",
        Side::Control,
        Why::Belief(LOST_READ),
    ),
    ("reborn_store_d0_one", Side::Control, Why::Belief(LOST_READ)),
    ("reborn_ret_d0_both", Side::Control, Why::Belief(MAY_FREE)),
    ("reborn_ret_d0_one", Side::Control, Why::Belief(MAY_FREE)),
    ("reborn_ret_d1_both", Side::Control, Why::Belief(MAY_FREE)),
    ("reborn_ret_d1_one", Side::Control, Why::Belief(MAY_FREE)),
    ("reborn_ret_d2_both", Side::Control, Why::Belief(MAY_FREE)),
    ("reborn_ret_d2_one", Side::Control, Why::Belief(MAY_FREE)),
    (
        "reborn_realloc_d2_both",
        Side::Control,
        Why::Cost(OLD_BLOCK),
    ),
    ("reborn_realloc_d2_one", Side::Control, Why::Cost(OLD_BLOCK)),
    (
        "left_behind_by_address_d1_both",
        Side::Hazard,
        Why::Cost(ESCAPED_HOLDER),
    ),
    (
        "left_behind_by_address_d1_one",
        Side::Hazard,
        Why::Cost(ESCAPED_HOLDER),
    ),
    (
        "left_behind_by_address_loaded_d1_both",
        Side::Hazard,
        Why::Cost(ESCAPED_HOLDER),
    ),
    (
        "left_behind_by_address_loaded_d1_one",
        Side::Hazard,
        Why::Cost(ESCAPED_HOLDER),
    ),
    (
        "left_behind_memcpy_d1_both",
        Side::Control,
        Why::Cost(ESCAPED_SOURCE),
    ),
    (
        "left_behind_memcpy_d1_one",
        Side::Control,
        Why::Cost(ESCAPED_SOURCE),
    ),
    (
        "left_behind_memcpy_d2_both",
        Side::Control,
        Why::Cost(ESCAPED_SOURCE),
    ),
    (
        "left_behind_memcpy_d2_one",
        Side::Control,
        Why::Cost(ESCAPED_SOURCE),
    ),
    (
        "left_behind_realloc_d2_both",
        Side::Control,
        Why::Cost(OLD_BLOCK),
    ),
    (
        "left_behind_realloc_d2_one",
        Side::Control,
        Why::Cost(OLD_BLOCK),
    ),
    (
        "left_behind_ret_d0_both",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "left_behind_ret_d0_one",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "left_behind_ret_d1_both",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "left_behind_ret_d1_one",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "left_behind_ret_d2_both",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "left_behind_ret_d2_one",
        Side::Control,
        Why::Belief(MAY_FREE),
    ),
    (
        "stored_freed_by_address_loaded_d1_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_by_address_loaded_d1_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_memcpy_block_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_memcpy_block_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_memcpy_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_memcpy_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_realloc_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_realloc_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_restored_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_restored_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_ret_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_ret_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_store_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_store_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_by_address_loaded_d1_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_by_address_loaded_d1_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_memcpy_block_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_memcpy_block_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_realloc_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_realloc_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_restored_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_restored_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_store_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_store_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_by_address_d1_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_freed_by_address_d1_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_by_address_d1_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_by_address_d1_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_memcpy_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_memcpy_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_ret_d2_both",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
    (
        "stored_out_ret_d2_one",
        Side::Control,
        Why::Cost(DEEP_WRITE_LEAVES),
    ),
];

/// What a row's hazard is reported as: a code, and where the control reports
/// the same code for a reason of its own, a phrase that only the hazard's
/// report carries.
///
/// **Codes alone for the beliefs**: `SC0402` stands for a lost, a stale and
/// a reborn pointer alike, in whichever words the route leaves it, "this
/// check stopped following" or "may have been freed", and each belief's
/// control differs from its hazard in the free alone, which is what keeps
/// the code from passing on a doubt the free did not cause. **A phrase for
/// the routes that read through a pointer first**, whose control doubts the
/// pointer it reads through: only the hazard doubts `r`.
fn mark(row: Row) -> (&'static str, Option<&'static str>) {
    // **About the pointer read back** where the route reads through a pointer
    // first: the control doubts the pointer it reads through for a reason of
    // its own, and only the hazard doubts `r`.
    match row.route {
        Route::CallDeep | Route::ReallocParam => return ("SC0412", Some("object `r` points at")),
        // **About the pointer passed on**, where a kept call may have stored
        // any pointer: what a departure asks (`Known::handed_untyped`).
        Route::KeptVoid | Route::KeptBytes | Route::KeptBlock => return ("SC0413", None),
        Route::Store
        | Route::Memcpy
        | Route::Realloc
        | Route::Call
        | Route::Ret
        | Route::MemcpyFromBlock
        | Route::MemcpyHalf
        | Route::MemcpyPart
        | Route::MemcpyMaybe
        | Route::CallStored
        | Route::Restored
        | Route::MemcpyUnread
        | Route::MemcpyShort
        | Route::MemcpyIntoPart
        | Route::MemcpyUnplaced
        | Route::MemcpyFromLocal
        | Route::MemcpyFromLocalMaybe
        | Route::StoredThrough
        | Route::ElementSlot
        | Route::ByAddress
        | Route::ByAddressLoaded => {}
    }
    match row.belief {
        Belief::Width => ("SC0412", None),
        Belief::Fresh => ("SC0411", None),
        Belief::Lost | Belief::Stale | Belief::Reborn | Belief::Caller => ("SC0402", None),
        Belief::Unreplaced => ("SC0407", None),
        // **A phrase for each door question**, since `SC0414` is both the
        // free after a store and the store after a free, and each words it
        // its own way.
        Belief::StoredOut => ("SC0414", Some("was stored where code outside")),
        Belief::StoredFreed => ("SC0414", Some("this stores")),
        Belief::LeftBehind => (
            "SC0407",
            Some("where a pointer that may have been freed is stored"),
        ),
        // **Either wording of `SC0413`**: a copy whose destination says
        // pointers is asked how wide (ADR-0073 rule 4), and one the record
        // asks is asked whether it starts an object (ADR-0087).
        Belief::PartPointer => ("SC0413", None),
        Belief::PartLocal => ("SC0411", None),
        // **The `return`'s own words**, since a read of the block is
        // `SC0411` too and would answer for a route the `return` misses.
        Belief::Returned => ("SC0411", Some("the caller may read pointers out of it")),
        Belief::HeldPointers => ("SC0413", None),
    }
}

const DECLARATIONS: &str = "void *malloc(int n);
void free(void *p);
void *realloc(void *p, int n);
void *memcpy(void *d, void *s, int n);
void *pass(void *p);
void get_narrow(char **p);
void get_wide(int **p);
void get_narrow2(char ***p);
void get_wide2(int ***p);
void release_all(void);
void release_ref(int **pp);
int use2(int **pp);
void touch(int **p);
void hand(int **p);
void keep_v(void *p);
void keep_c(char *p);
void keep_t(int ***p);
void keep_t4(int ****p);
void use1(int *p);
";

/// The statements that make the slot, its spelling, and what it lives in.
fn container(depth: Depth) -> (Vec<String>, &'static str) {
    container_of(depth, "int *")
}

/// [`container`] for a slot holding a `ty`, a pointer type spelled with its
/// last `*`, so that the levels above it are spelled by adding more.
fn container_of(depth: Depth, ty: &str) -> (Vec<String>, &'static str) {
    let made = match depth {
        Depth::D0 => vec![format!("{ty}s = 0;")],
        Depth::D1 => vec![
            format!("{ty}*m = malloc(8);"),
            "if (!m) return 0;".to_owned(),
        ],
        Depth::D2 => vec![
            format!("{ty}**m = malloc(8);"),
            "if (!m) return 0;".to_owned(),
            format!("{ty}*n = malloc(8);"),
            "if (!n) return 0;".to_owned(),
            "*m = n;".to_owned(),
        ],
    };
    let slot = match depth {
        Depth::D0 => "s",
        Depth::D1 => "*m",
        Depth::D2 => "**m",
    };
    (made, slot)
}

/// What holds the slot, which `Route::Realloc` grows, and the statement that
/// puts the grown block back where it was.
fn holder(depth: Depth) -> (&'static str, &'static str) {
    match depth {
        Depth::D0 => unreachable!("a local is not grown; see `NOT_WRITTEN`"),
        Depth::D1 => ("m", "m = g_;"),
        Depth::D2 => ("n", "*m = g_;"),
    }
}

/// The statements that put `value`, an `int *`, into the slot by `route`.
fn put(route: Route, depth: Depth, slot: &str, value: &str) -> Vec<String> {
    put_of(route, depth, slot, value, "int *")
}

/// [`put`] for a `value` of type `ty`, spelled as [`container_of`] takes it.
fn put_of(route: Route, depth: Depth, slot: &str, value: &str, ty: &str) -> Vec<String> {
    match route {
        Route::Store => vec![format!("{slot} = {value};")],
        Route::Memcpy => vec![
            format!("{ty}v_ = {value};"),
            format!("memcpy(&{slot}, &v_, 8);"),
        ],
        Route::Ret => vec![format!("{slot} = pass({value});")],
        Route::Realloc => {
            let (grown, back) = holder(depth);
            vec![
                format!("{slot} = {value};"),
                format!("{ty}*g_ = realloc({grown}, 16);"),
                "if (!g_) return 0;".to_owned(),
                back.to_owned(),
            ]
        }
        Route::MemcpyFromBlock => vec![
            format!("{ty}*blk_ = malloc(8);"),
            "if (!blk_) return 0;".to_owned(),
            format!("*blk_ = {value};"),
            format!("memcpy(&{slot}, blk_, 8);"),
        ],
        Route::Restored => vec![
            format!("{ty}*blk_ = malloc(8);"),
            "if (!blk_) return 0;".to_owned(),
            format!("*blk_ = {value};"),
            format!("{ty}t_ = *blk_;"),
            format!("{slot} = t_;"),
        ],
        Route::Call | Route::CallStored | Route::CallDeep | Route::ReallocParam => {
            unreachable!("only `Belief::Width` writes through a call or a parameter; see `width`")
        }
        Route::MemcpyHalf
        | Route::MemcpyPart
        | Route::MemcpyMaybe
        | Route::MemcpyUnread
        | Route::MemcpyShort
        | Route::MemcpyIntoPart
        | Route::MemcpyUnplaced
        | Route::MemcpyFromLocal
        | Route::MemcpyFromLocalMaybe
        | Route::StoredThrough
        | Route::ElementSlot => {
            unreachable!("only `Belief::Fresh` asks a copy's length or source; see `fresh`")
        }
        Route::KeptVoid | Route::KeptBytes | Route::KeptBlock => {
            unreachable!("only `Belief::Width` asks a kept call; see `width`")
        }
        // **Through the address of the local above the slot**, at the one
        // depth whose slot, `*m`, has one; `NOT_WRITTEN` excuses the others.
        Route::ByAddress => {
            assert_eq!(
                depth,
                Depth::D1,
                "only the slot at depth one is a level below a local"
            );
            vec![format!("{ty}**t_ = &m;"), format!("**t_ = {value};")]
        }
        Route::ByAddressLoaded => {
            assert_eq!(
                depth,
                Depth::D1,
                "only the slot at depth one is a level below a local"
            );
            vec![
                format!("{ty}**t_ = &m;"),
                format!("{ty}*q_ = *t_;"),
                format!("*q_ = {value};"),
            ]
        }
    }
}

/// The route on every path, or on one arm with a null on the other.
fn shaped(shape: Shape, slot: &str, body: Vec<String>) -> Vec<String> {
    match shape {
        Shape::Both => body,
        Shape::One => {
            let mut lines = vec!["if (k) {".to_owned()];
            lines.extend(body.into_iter().map(|line| format!("    {line}")));
            lines.push("} else {".to_owned());
            lines.push(format!("    {slot} = 0;"));
            lines.push("}".to_owned());
            lines
        }
    }
}

/// A `char`'s address, converted through `void *`, for the hazard; an
/// `int`'s for the control. Through a call, one taking `char **` or `int **`
/// writes it, read back from the local, or from a block it was stored in
/// (`Route::CallStored`); one taking `char ***` or `int ***` writes a
/// pointer to pointers, read two levels down (`Route::CallDeep`); and a
/// `void ***` or `int ***` parameter is grown by `realloc` and read two
/// levels down (`Route::ReallocParam`). Those three write their own
/// programs, and the container the depth makes is left unread, which is
/// why their rows are written at one depth only.
fn width(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines = vec!["char c = 0;".to_owned(), "int i = 0;".to_owned()];
    lines.extend(made);
    let body = match row.route {
        Route::CallStored => {
            let mut call = if hazard {
                vec!["void *p_ = &s;".to_owned(), "get_narrow(p_);".to_owned()]
            } else {
                vec!["get_wide(&s);".to_owned()]
            };
            call.extend([
                "int **m_ = malloc(8);".to_owned(),
                "if (!m_) return 0;".to_owned(),
                "*m_ = s;".to_owned(),
            ]);
            call
        }
        Route::CallDeep => {
            let mut call = vec!["int **s2 = 0;".to_owned()];
            if hazard {
                call.extend([
                    "void *p2_ = &s2;".to_owned(),
                    "get_narrow2(p2_);".to_owned(),
                ]);
            } else {
                call.push("get_wide2(&s2);".to_owned());
            }
            call.extend(["int **pp = s2;".to_owned(), "if (!pp) return 0;".to_owned()]);
            call
        }
        Route::ReallocParam => {
            vec![
                "void *pm_ = pm;".to_owned(),
                "void *grown = realloc(pm_, 16);".to_owned(),
                "if (!grown) return 0;".to_owned(),
                "int ***pi = grown;".to_owned(),
                "int **a = *pi;".to_owned(),
                "if (!a) return 0;".to_owned(),
            ]
        }
        Route::Call => {
            let address = match row.depth {
                Depth::D0 => "&s",
                Depth::D1 => "m",
                Depth::D2 => "n",
            };
            if hazard {
                vec![
                    format!("void *p_ = {address};"),
                    "get_narrow(p_);".to_owned(),
                ]
            } else {
                vec![format!("get_wide({address});")]
            }
        }
        Route::Store
        | Route::Memcpy
        | Route::Realloc
        | Route::Ret
        | Route::MemcpyFromBlock
        | Route::Restored => {
            let mut value = vec![];
            if hazard {
                value.push("void *vc = &c;".to_owned());
                value.push("int *v = vc;".to_owned());
            } else {
                value.push("int *v = &i;".to_owned());
            }
            value.extend(put(row.route, row.depth, slot, "v"));
            value
        }
        // **A local's address kept by a call that reads it untyped**, a
        // pointer's address stored there after, a later call, and the
        // pointer passed on, which is where what the kept call may have
        // stored is asked; the control keeps it typed.
        Route::KeptVoid | Route::KeptBytes | Route::KeptBlock => {
            let keep = |untyped: &str| {
                if hazard {
                    untyped.to_owned()
                } else {
                    "keep_t".to_owned()
                }
            };
            let mut kept = vec!["int x3 = 0;".to_owned(), "int *w = &x3;".to_owned()];
            match row.route {
                // **After the holder moves on**, which is where what was kept
                // untyped is walked again from the mark rather than from the
                // holder's value now.
                Route::KeptVoid => kept.extend([
                    "int **u = 0;".to_owned(),
                    "int ***v = 0;".to_owned(),
                    format!("{}(&v);", if hazard { "keep_v" } else { "keep_t4" }),
                    "v = &u;".to_owned(),
                    "release_all();".to_owned(),
                    "v = 0;".to_owned(),
                    "u = &w;".to_owned(),
                ]),
                Route::KeptBytes => {
                    kept.push("int **v = 0;".to_owned());
                    if hazard {
                        kept.extend(["void *pv = &v;".to_owned(), "keep_c(pv);".to_owned()]);
                    } else {
                        kept.push("keep_t(&v);".to_owned());
                    }
                    kept.push("v = &w;".to_owned());
                }
                _ => kept.extend([
                    "int ***m3 = malloc(8);".to_owned(),
                    "if (!m3) return 0;".to_owned(),
                    "*m3 = 0;".to_owned(),
                    format!("{}(m3);", keep("keep_v")),
                    "*m3 = &w;".to_owned(),
                ]),
            }
            kept.push("release_all();".to_owned());
            kept
        }
        Route::MemcpyHalf
        | Route::MemcpyPart
        | Route::MemcpyMaybe
        | Route::MemcpyUnread
        | Route::MemcpyShort
        | Route::MemcpyIntoPart
        | Route::MemcpyUnplaced
        | Route::MemcpyFromLocal
        | Route::MemcpyFromLocalMaybe
        | Route::StoredThrough
        | Route::ElementSlot => {
            unreachable!("asked of freshness only; see `NOT_WRITTEN`")
        }
        Route::ByAddress | Route::ByAddressLoaded => {
            unreachable!("asked of the door questions only; see `NOT_WRITTEN`")
        }
    };
    lines.extend(shaped(row.shape, slot, body));
    lines
}

/// Memory nothing wrote copied into the slot for the hazard; the same memory
/// written first for the control.
///
/// **Through a store or a `realloc`, what is held is the read**: the hazard
/// loads `*b` or `*g_` to store it, and ADR-0057 reports that load where it
/// happens, so those rows hold that a store and a `realloc` read fresh
/// memory as fresh, and not what reaches the slot. Only `memcpy` carries the
/// mark into the slot unread, and its rows are the ones that hold that.
fn fresh(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines = made;
    lines.push("int **b = malloc(8);".to_owned());
    lines.push("if (!b) return 0;".to_owned());
    let covers = matches!(
        row.route,
        Route::MemcpyHalf | Route::MemcpyPart | Route::MemcpyMaybe
    );
    if !hazard && !covers {
        lines.push("int x = 0;".to_owned());
        lines.push("*b = &x;".to_owned());
    }
    let body = match row.route {
        Route::Store => vec![format!("{slot} = *b;")],
        Route::Memcpy => vec![format!("memcpy(&{slot}, b, 8);")],
        // These three ask what the copy covers, so the source is written in
        // both programs, and the two differ in the length or in the arm the
        // source is written on.
        Route::MemcpyHalf => {
            lines.push("int x = 0;".to_owned());
            lines.push("*b = &x;".to_owned());
            vec![format!(
                "memcpy(&{slot}, b, {});",
                if hazard { 4 } else { 8 }
            )]
        }
        Route::MemcpyPart => vec![
            "int x2 = 0;".to_owned(),
            "int *v2[2];".to_owned(),
            "v2[0] = &x2;".to_owned(),
            "v2[1] = &x2;".to_owned(),
            "int **m2 = malloc(16);".to_owned(),
            "if (!m2) return 0;".to_owned(),
            format!("memcpy(m2, v2, {});", if hazard { 12 } else { 16 }),
        ],
        Route::MemcpyMaybe => {
            lines.push("int x = 0;".to_owned());
            if hazard {
                lines.push("if (k) {".to_owned());
                lines.push("    *b = &x;".to_owned());
                lines.push("}".to_owned());
            } else {
                lines.push("*b = &x;".to_owned());
            }
            vec![format!("memcpy(&{slot}, b, 8);")]
        }
        Route::Realloc => vec![
            "int **g_ = realloc(b, 16);".to_owned(),
            "if (!g_) return 0;".to_owned(),
            format!("{slot} = *g_;"),
        ],
        // The slot handed by its address to a call this check cannot read,
        // which writes nothing in either program: the control wrote the slot
        // first. Believed written by the call, so the hazard is silent, which
        // `ACCEPTED` lists as the boundary belief it is.
        Route::Call => {
            if hazard {
                vec![format!("touch(&{slot});")]
            } else {
                vec![
                    "int y = 0;".to_owned(),
                    format!("{slot} = &y;"),
                    format!("touch(&{slot});"),
                ]
            }
        }
        // The slot written first, then copied over out of `b` by a length
        // this check cannot read.
        Route::MemcpyUnread => vec![
            "int x0 = 0;".to_owned(),
            format!("{slot} = &x0;"),
            format!("memcpy(&{slot}, b, k);"),
        ],
        // One pointer copied out of `b` into a block of two whose second
        // was written, the second read.
        Route::MemcpyShort => vec![
            "int x0 = 0;".to_owned(),
            "int **n2 = malloc(16);".to_owned(),
            "if (!n2) return 0;".to_owned(),
            "n2[1] = &x0;".to_owned(),
            "memcpy(n2, b, 8);".to_owned(),
        ],
        // The same into a local array of two.
        Route::MemcpyIntoPart => vec![
            "int x0 = 0;".to_owned(),
            "int *a2[2];".to_owned(),
            "a2[1] = &x0;".to_owned(),
            "memcpy(a2, b, 8);".to_owned(),
        ],
        // The slot written and its block handed on, then a copy out of `b`
        // into a parameter that may be that block.
        Route::MemcpyUnplaced => vec![
            "int x0 = 0;".to_owned(),
            format!("{slot} = &x0;"),
            "hand(m);".to_owned(),
            "memcpy(d, b, 8);".to_owned(),
        ],
        // Out of a local nothing wrote, or one written on one arm only, and
        // the control's written; `b` is not the source.
        Route::MemcpyFromLocal | Route::MemcpyFromLocalMaybe => {
            let mut source = vec!["int x0 = 0;".to_owned()];
            match (hazard, row.route) {
                (true, Route::MemcpyFromLocal) => source.push("int *src_;".to_owned()),
                (true, _) => {
                    source.push("int *src_;".to_owned());
                    source.push("if (k) src_ = &x0;".to_owned());
                }
                (false, _) => source.push("int *src_ = &x0;".to_owned()),
            }
            source.push(format!("memcpy(&{slot}, &src_, 8);"));
            source
        }
        // Written backwards through a pointer moved off the array's end: the
        // hazard writes the second element alone, the control both.
        //
        // **What is read is not the array**, which a walk off its end leaves
        // written in part and so doubted on both sides: the hazard reads the
        // element the loop left unwritten, and the control reads nothing of
        // it, so what it would gain is a doubt at the loop's own store.
        Route::StoredThrough => vec![
            "int x0 = 0;".to_owned(),
            "int *a2[2];".to_owned(),
            "int **e = a2 + 2;".to_owned(),
            format!(
                "for (int i = 0; i < {}; i++) {{",
                if hazard { 1 } else { 2 }
            ),
            "    e = e - 1;".to_owned(),
            "    *e = &x0;".to_owned(),
            "}".to_owned(),
            if hazard {
                "int *t_ = a2[0];"
            } else {
                "int *t_ = &x0;"
            }
            .to_owned(),
        ],
        // An element's address stored into memory, read back and written
        // through: the second element for the hazard, the first for the
        // control.
        Route::ElementSlot => vec![
            "int x0 = 0;".to_owned(),
            "int *a2[2];".to_owned(),
            "int **box2[1];".to_owned(),
            format!("box2[0] = {};", if hazard { "a2 + 1" } else { "a2" }),
            "int **f2 = box2[0];".to_owned(),
            "*f2 = &x0;".to_owned(),
        ],
        Route::Ret
        | Route::Restored
        | Route::MemcpyFromBlock
        | Route::CallStored
        | Route::CallDeep
        | Route::ReallocParam
        | Route::KeptVoid
        | Route::KeptBytes
        | Route::KeptBlock
        | Route::ByAddress
        | Route::ByAddressLoaded => unreachable!("see `NOT_WRITTEN`"),
    };
    lines.extend(shaped(row.shape, slot, body));
    lines
}

/// A loop whose earlier allocation, `v`, the next turn's `malloc` makes this
/// check lose.
const LOST_LOOP: [&str; 11] = [
    "int *u = 0;",
    "int *q = 0;",
    "int t = 0;",
    "while (t < 2) {",
    "    u = q;",
    "    q = malloc(4);",
    "    if (!q) return 0;",
    "    t = t + 1;",
    "}",
    "int *v = u;",
    "if (!v) return 0;",
];

/// A loop's earlier allocation, which the loop's next `malloc` makes this
/// check lose, put in the slot and freed by the name it was put there under,
/// for the hazard; **the same program without the free** for the control, so
/// that the two differ in the free alone and the control asks that a lost
/// pointer held in memory is not doubted before anything may have freed it.
fn lost(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines: Vec<String> = LOST_LOOP.iter().map(|line| (*line).to_owned()).collect();
    lines.extend(made);
    let body = put(row.route, row.depth, slot, "v");
    lines.extend(shaped(row.shape, slot, body));
    if hazard {
        lines.push("free(v);".to_owned());
    }
    lines
}

/// The loop's lost allocation, as [`lost`] makes it, freed **before** the route
/// puts it in the slot, for the hazard; the same with nothing freed for the
/// control. A pointer freed and then stored carries the mark a store leaves
/// in the memory it lands in, the cell's `stale_read`, which is what this belief
/// asks; the free after the route is [`lost`]'s, and a site freed and made
/// again is [`reborn`]'s.
fn stale(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines: Vec<String> = LOST_LOOP.iter().map(|line| (*line).to_owned()).collect();
    lines.push("int *w = v;".to_owned());
    if hazard {
        lines.push("free(v);".to_owned());
    }
    lines.extend(made);
    let body = put(row.route, row.depth, slot, "w");
    lines.extend(shaped(row.shape, slot, body));
    lines
}

/// A loop that frees its allocation each turn, `o` the last turn's, and on
/// the second turn puts `o` in the slot, for the hazard; the same loop
/// freeing nothing for the control. Read back after the loop, the slot holds
/// a pointer whose site the second turn made again after its free.
fn reborn(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines = made;
    lines.push(format!("{slot} = 0;"));
    lines.push("int *o = 0;".to_owned());
    lines.push("int t = 0;".to_owned());
    lines.push("while (t < 2) {".to_owned());
    lines.push("    int *p = malloc(4);".to_owned());
    lines.push("    if (!p) return 0;".to_owned());
    lines.push("    *p = 1;".to_owned());
    lines.push("    if (t == 1) {".to_owned());
    // Grown after the loop, as `stale` once found: a holder `realloc` moves
    // inside the loop is doubted on the next turn for a reason of its own.
    let inside = if row.route == Route::Realloc {
        Route::Store
    } else {
        row.route
    };
    for line in shaped(row.shape, slot, put(inside, row.depth, slot, "o")) {
        lines.push(format!("        {line}"));
    }
    lines.push("    }".to_owned());
    lines.push("    o = p;".to_owned());
    if hazard {
        lines.push("    free(p);".to_owned());
    }
    lines.push("    t = t + 1;".to_owned());
    lines.push("}".to_owned());
    if row.route == Route::Realloc {
        let (grown, back) = holder(row.depth);
        lines.push(format!("int **g_ = realloc({grown}, 16);"));
        lines.push("if (!g_) return 0;".to_owned());
        lines.push(back.to_owned());
    }
    lines
}

/// A pointer read out of the caller's memory, through the parameter `cp`,
/// put in the slot, and a call this check cannot read after it for the
/// hazard; the same with no call for the control.
fn caller(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines: Vec<String> = ["if (!cp) return 0;", "int *v = *cp;", "if (!v) return 0;"]
        .iter()
        .map(|line| (*line).to_owned())
        .collect();
    lines.extend(made);
    lines.extend(shaped(
        row.shape,
        slot,
        put(row.route, row.depth, slot, "v"),
    ));
    if hazard {
        lines.push("release_all();".to_owned());
    }
    lines
}

/// An allocation stored in a block, `*h = a`, and handed to a call this
/// check cannot read through another local's address, `release_ref(&b)`,
/// which may free it and cannot replace the slot in `h`, for the hazard; the
/// same with no call for the control. What is read back out of `h` is put
/// in the slot.
fn unreplaced(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines: Vec<String> = [
        "int *a = malloc(4);",
        "if (!a) return 0;",
        "int **h = malloc(8);",
        "if (!h) return 0;",
        "*h = a;",
        "int *b = a;",
    ]
    .iter()
    .map(|line| (*line).to_owned())
    .collect();
    if hazard {
        lines.push("release_ref(&b);".to_owned());
    }
    lines.push("int *v = *h;".to_owned());
    lines.extend(made);
    lines.extend(shaped(
        row.shape,
        slot,
        put(row.route, row.depth, slot, "v"),
    ));
    lines
}

/// The call that hands the slot's container to code this check cannot read:
/// the local's address at depth zero, the block below it deeper.
fn handed_over(depth: Depth) -> &'static str {
    match depth {
        Depth::D0 => "touch(&s);",
        Depth::D1 => "touch(m);",
        Depth::D2 => "keep_t(m);",
    }
}

/// The door questions' program: an allocation `x`, the slot's container, the
/// route putting `x` there, and the free.
///
/// **The hazard and the control differ in one thing each**, so that the code
/// a row asks for can only come from what the door asks. For
/// [`Belief::StoredOut`] and [`Belief::StoredFreed`] it is whether the
/// container was handed to code this check cannot read before the write, which
/// is whether the write leaves; both free `x`, after the write and before it.
/// For [`Belief::LeftBehind`] the container is handed after the write in
/// both, and only the hazard frees `x` first.
fn door(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines = vec![
        "int *x = malloc(4);".to_owned(),
        "if (!x) return 0;".to_owned(),
    ];
    lines.extend(made);
    let put_x = shaped(row.shape, slot, put(row.route, row.depth, slot, "x"));
    match row.belief {
        Belief::StoredOut => {
            if hazard {
                lines.push(handed_over(row.depth).to_owned());
            }
            lines.extend(put_x);
            lines.push("free(x);".to_owned());
        }
        Belief::StoredFreed => {
            if hazard {
                lines.push(handed_over(row.depth).to_owned());
            }
            lines.push("free(x);".to_owned());
            lines.extend(put_x);
        }
        Belief::LeftBehind => {
            lines.extend(put_x);
            if hazard {
                lines.push("free(x);".to_owned());
            }
            lines.push(handed_over(row.depth).to_owned());
        }
        Belief::PartPointer => {
            lines.extend(put_x);
            if hazard {
                lines.push("int i_ = 1;".to_owned());
                lines.push(format!("memcpy(&{slot}, &i_, 4);"));
            }
            lines.push(handed_over(row.depth).to_owned());
        }
        Belief::Width
        | Belief::Fresh
        | Belief::Lost
        | Belief::Stale
        | Belief::Reborn
        | Belief::Caller
        | Belief::Unreplaced
        | Belief::PartLocal
        | Belief::Returned
        | Belief::HeldPointers => unreachable!("not a door question; see `program`"),
    }
    lines
}

/// ADR-0073's three beliefs' program: `x`, an `int **`, made as the belief
/// asks, then put into a slot holding an `int **`, or a `void *` for
/// [`Belief::HeldPointers`], by the route. What reads it back is the
/// belief's own, in [`program`].
fn pointer_to_pointers(row: Row, hazard: bool) -> Vec<String> {
    let mut lines: Vec<String> = match (row.belief, hazard) {
        (Belief::PartLocal, true) => vec!["char cb_[8];", "cb_[0] = 0;"],
        (Belief::PartLocal, false) => vec![
            "char cb_[8];",
            "int x0 = 0;",
            "int *w_ = &x0;",
            "memcpy(cb_, &w_, 8);",
        ],
        (Belief::Returned | Belief::HeldPointers, _) => {
            vec!["int **x = malloc(8);", "if (!x) return 0;"]
        }
        (
            Belief::Width
            | Belief::Fresh
            | Belief::Lost
            | Belief::Stale
            | Belief::Reborn
            | Belief::Caller
            | Belief::Unreplaced
            | Belief::StoredOut
            | Belief::StoredFreed
            | Belief::LeftBehind
            | Belief::PartPointer,
            _,
        ) => unreachable!("not one of ADR-0073's beliefs; see `program`"),
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    match (row.belief, hazard) {
        (Belief::PartLocal, _) => {
            lines.push("void *cv_ = cb_;".to_owned());
            lines.push("int **x = cv_;".to_owned());
        }
        (Belief::Returned, false) => {
            lines.push("int *y_ = malloc(4);".to_owned());
            lines.push("if (!y_) return 0;".to_owned());
            lines.push("*x = y_;".to_owned());
        }
        // **Through a `void *`**, so that no type the route or the read back
        // goes through says pointers, and only what was recorded of `x`'s
        // block where it was held as an `int **` does (ADR-0086).
        (Belief::HeldPointers, _) => lines.push("void *xv_ = x;".to_owned()),
        _ => {}
    }
    let (ty, value) = match row.belief {
        Belief::HeldPointers => ("void *", "xv_"),
        _ => ("int **", "x"),
    };
    let (made, slot) = container_of(row.depth, ty);
    lines.extend(made);
    lines.extend(shaped(
        row.shape,
        slot,
        put_of(row.route, row.depth, slot, value, ty),
    ));
    lines
}

/// The whole program for one side of a row: the declarations, the belief's
/// statements, and the read back every belief but a door question shares,
/// through an `int *` that is tested and written through.
fn program(row: Row, hazard: bool) -> String {
    let body = match row.belief {
        Belief::Width => width(row, hazard),
        Belief::Fresh => fresh(row, hazard),
        Belief::Lost => lost(row, hazard),
        Belief::Stale => stale(row, hazard),
        Belief::Reborn => reborn(row, hazard),
        Belief::Caller => caller(row, hazard),
        Belief::Unreplaced => unreplaced(row, hazard),
        Belief::StoredOut | Belief::StoredFreed | Belief::LeftBehind | Belief::PartPointer => {
            door(row, hazard)
        }
        Belief::PartLocal | Belief::Returned | Belief::HeldPointers => {
            pointer_to_pointers(row, hazard)
        }
    };
    let read = read_from(row);
    // **The read every belief shares**, but the one asked by address alone,
    // which hands the pointer read back on by its address.
    let kept = matches!(
        row.route,
        Route::KeptVoid | Route::KeptBytes | Route::KeptBlock
    );
    let tail: Vec<String> = match row.belief {
        _ if kept => vec![format!("use1({read});"), "return 0;".to_owned()],
        // **Nothing read back**: a door question is asked at the write, the
        // free or the call, and `door` ends at it.
        Belief::StoredOut | Belief::StoredFreed | Belief::LeftBehind | Belief::PartPointer => {
            vec!["return 0;".to_owned()]
        }
        Belief::Unreplaced => vec![format!("int *r = {read};"), "return use2(&r);".to_owned()],
        // **Loaded through as an `int *`**, which reads the array's bytes.
        Belief::PartLocal => vec![
            format!("int **r = {read};"),
            "if (!r) return 0;".to_owned(),
            "int *p_ = *r;".to_owned(),
            "return p_ == 0;".to_owned(),
        ],
        Belief::Returned => vec![format!("return {read};")],
        // **Read back as the `void *` it was put there as**: see
        // `pointer_to_pointers`.
        Belief::HeldPointers => {
            let mut lines = vec![
                format!("void *d_ = {read};"),
                "if (!d_) return 0;".to_owned(),
                "int i_ = 1;".to_owned(),
            ];
            if hazard {
                lines.push("memcpy(d_, &i_, 4);".to_owned());
            } else {
                lines.push("int *w_ = &i_;".to_owned());
                lines.push("memcpy(d_, &w_, 8);".to_owned());
            }
            // **Handed on as memory of pointers**, where the callee reads a
            // pointer made of part of an `int`: a copy into a block nothing
            // reads after is no hazard, and the partial pointer record asks
            // it where it leaves rather than at the copy (ADR-0087).
            lines.push("return use2(x);".to_owned());
            lines
        }
        Belief::Width
        | Belief::Fresh
        | Belief::Lost
        | Belief::Stale
        | Belief::Reborn
        | Belief::Caller => vec![
            format!("int *r = {read};"),
            "if (!r) return 0;".to_owned(),
            "*r = 1;".to_owned(),
            "return 0;".to_owned(),
        ],
    };
    let mut text = String::from(DECLARATIONS);
    // **A table of pointers is returned** where the `return` is what asks.
    let returns = match row.belief {
        Belief::Returned => "int **",
        _ => "int ",
    };
    text.push_str(&format!("{returns}f({}) {{\n", parameters(row, hazard)));
    for line in body.iter().chain(&tail).map(String::as_str) {
        text.push_str("    ");
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("}\n");
    text
}

/// Where a row's pointer is read back from: the slot, or for a route that
/// moves it on, where the route left it.
fn read_from(row: Row) -> String {
    match row.route {
        Route::CallStored => "*m_".to_owned(),
        Route::CallDeep => "*pp".to_owned(),
        Route::ReallocParam => "*a".to_owned(),
        Route::MemcpyPart => "m2[1]".to_owned(),
        Route::MemcpyShort => "n2[1]".to_owned(),
        Route::MemcpyIntoPart => "a2[1]".to_owned(),
        Route::StoredThrough => "t_".to_owned(),
        Route::ElementSlot => "a2[0]".to_owned(),
        Route::KeptVoid | Route::KeptBytes | Route::KeptBlock => "w".to_owned(),
        Route::Store
        | Route::Memcpy
        | Route::Realloc
        | Route::Call
        | Route::Ret
        | Route::MemcpyFromBlock
        | Route::MemcpyHalf
        | Route::MemcpyMaybe
        | Route::Restored
        | Route::MemcpyUnread
        | Route::MemcpyUnplaced
        | Route::MemcpyFromLocal
        | Route::MemcpyFromLocalMaybe
        | Route::ByAddress
        | Route::ByAddressLoaded => container(row.depth).1.to_owned(),
    }
}

/// The parameters of a row's function: the parameter a route grows, typed
/// for the side, beside the `k` every shape tests.
fn parameters(row: Row, hazard: bool) -> &'static str {
    match row.belief {
        // The caller's memory a pointer is read out of.
        Belief::Caller => return "int k, int **cp",
        Belief::Width
        | Belief::Fresh
        | Belief::Lost
        | Belief::Stale
        | Belief::Reborn
        | Belief::Unreplaced
        | Belief::StoredOut
        | Belief::StoredFreed
        | Belief::LeftBehind
        | Belief::PartPointer
        | Belief::PartLocal
        | Belief::Returned
        | Belief::HeldPointers => {}
    }
    match (row.route, hazard) {
        (Route::ReallocParam, true) => "int k, void ***pm",
        (Route::ReallocParam, false) => "int k, int ***pm",
        (Route::MemcpyUnplaced, _) => "int k, int **d",
        (
            Route::Store
            | Route::Memcpy
            | Route::Realloc
            | Route::Call
            | Route::Ret
            | Route::MemcpyFromBlock
            | Route::MemcpyHalf
            | Route::MemcpyPart
            | Route::MemcpyMaybe
            | Route::CallStored
            | Route::CallDeep
            | Route::Restored
            | Route::MemcpyUnread
            | Route::MemcpyShort
            | Route::MemcpyIntoPart
            | Route::MemcpyFromLocal
            | Route::MemcpyFromLocalMaybe
            | Route::StoredThrough
            | Route::ElementSlot
            | Route::KeptVoid
            | Route::KeptBytes
            | Route::KeptBlock
            | Route::ByAddress
            | Route::ByAddressLoaded,
            _,
        ) => "int k",
    }
}

/// What the compiler said about one program: its exit code and the code of
/// every report, with the message beside it.
#[derive(Debug)]
struct Answer {
    exit: i32,
    /// Each report: whether it is an error, its code, and its message.
    reports: Vec<(bool, String, String)>,
}

impl Answer {
    /// Whether a report carries what `belief` is reported as.
    fn reports(&self, row: Row) -> bool {
        let (code, phrase) = mark(row);
        self.reports.iter().any(|(_, said, message)| {
            said == code && phrase.is_none_or(|phrase| message.contains(phrase))
        })
    }

    /// Whether a report carries what `row`'s belief is reported as **and is
    /// an error**, which under `--allow-unknown` only a proof is.
    fn proves(&self, row: Row) -> bool {
        let (code, phrase) = mark(row);
        self.reports.iter().any(|(error, said, message)| {
            *error && said == code && phrase.is_none_or(|phrase| message.contains(phrase))
        })
    }

    /// `exit:CODE,CODE`, the codes sorted and once each, for the summary.
    fn summary(&self) -> String {
        let mut codes: Vec<&str> = self
            .reports
            .iter()
            .map(|(_, code, _)| code.as_str())
            .collect();
        codes.sort_unstable();
        codes.dedup();
        format!(
            "{}:{}",
            self.exit,
            if codes.is_empty() {
                "-".to_owned()
            } else {
                codes.join(",")
            }
        )
    }
}

/// Run the compiler on `source` at `safety`, in a directory of its own,
/// removed afterwards.
///
/// **Its own per call, and not per row and side**: a row's test and the
/// summary write the same program at the same time, and a directory named by
/// the row had one run read the other's half-written file, a failure that came
/// and went between runs. Numbered by this process, as the corpus's are,
/// rather than named, since a Windows working directory has to stay short.
fn ask(name: &str, source: &str, safety: &str) -> Answer {
    ask_with(name, source, safety, &[])
}

/// [`ask`], with `--allow-unknown`, under which a doubt is a warning and only
/// a proof is an error.
fn ask_allowing(name: &str, source: &str) -> Answer {
    ask_with(name, source, "memory", &["--allow-unknown"])
}

fn ask_with(name: &str, source: &str, safety: &str, extra: &[&str]) -> Answer {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("safec-route-{}-{call}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a row's working directory can be made");
    std::fs::write(dir.join("f.c"), source).expect("a generated program can be written");
    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .current_dir(&dir)
        .args([
            "--color",
            "never",
            "--safety",
            safety,
            "--emit",
            "safety-ir",
        ])
        .args(["--target", "x86_64-pc-windows-msvc", "-o", "out", "f.c"])
        .args(extra)
        .output()
        .expect("the compiler binary was built for this test");
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reports = stderr
        .lines()
        .filter_map(|line| {
            let (error, rest) = match line.strip_prefix("error[") {
                Some(rest) => (true, rest),
                None => (false, line.strip_prefix("warning[")?),
            };
            let (code, message) = rest.split_once(']')?;
            Some((error, code.to_owned(), message.to_owned()))
        })
        .collect();
    let exit = output
        .status
        .code()
        .unwrap_or_else(|| panic!("row `{name}`: the compiler was killed by a signal"));
    Answer { exit, reports }
}

/// Which side of `row` disagrees with its belief's rule, if either.
fn disagreement(name: &str, row: Row) -> Option<Side> {
    let hazard = ask(name, &program(row, true), "memory");
    let control = ask(name, &program(row, false), "memory");
    match (hazard.reports(row), control.reports(row)) {
        (true, false) => None,
        (false, false) => Some(Side::Hazard),
        (true, true) => Some(Side::Control),
        (false, true) => Some(Side::Both),
    }
}

/// Ask a row its belief's rule, against what `ACCEPTED` lists for it.
///
/// Mutation: the checker's `memcpy` pours no width into what it copies into;
/// `width_memcpy_d1_both` fails, among the four rows at one and two levels.
/// Mutation: the copy carries no unwritten mark into a block;
/// `fresh_memcpy_d1_both` fails, with its one-arm row. Mutation: ask the
/// control's program for the hazard's; every row not listed on its control
/// fails. Mutation: list in
/// `ACCEPTED` a row that agrees; that row fails.
fn check(name: &str, row: Row) {
    let entry = ACCEPTED.iter().find(|(listed, _, _)| *listed == name);
    let listed = entry.map(|&(_, side, _)| side);
    let found = disagreement(name, row);
    if found != listed {
        let mark = mark(row);
        let meaning = match meaning(entry.map(|&(_, _, why)| why), found) {
            Meaning::Regression(why) => format!(
                "THIS IS A REGRESSION: the control is not provable from the unit ({why}), \
                 and the row agreeing is the check saying safe about it. Restore the doubt, \
                 or supersede the record first."
            ),
            Meaning::Fixed(why) => {
                format!("The row now agrees ({why}): remove its entry from `ACCEPTED`.")
            }
            Meaning::Moved(why) => format!(
                "The row is listed ({why}) but disagrees on another side now: read both \
                 programs and say which side, or which issue, holds."
            ),
            Meaning::New => "A row that newly disagrees is a defect, or a belief a record took, \
                             and is listed with the issue or record that says so."
                .to_owned(),
        };
        panic!(
            "row `{name}` disagrees on {found:?} where `ACCEPTED` lists {listed:?}.\n\
             The hazard must report {mark:?} and the control must not. {meaning}\n\n\
             hazard:\n{}\ncontrol:\n{}",
            program(row, true),
            program(row, false),
        );
    }
}

/// What a row disagreeing with what `ACCEPTED` lists for it means.
#[derive(Debug, PartialEq, Eq)]
enum Meaning {
    /// A listed belief now agrees: the check says safe about what it cannot
    /// prove.
    Regression(&'static str),
    /// A listed cost now agrees: the entry goes.
    Fixed(&'static str),
    /// A listed row disagrees on another side than listed.
    Moved(&'static str),
    /// A row nothing lists disagrees.
    New,
}

fn meaning(listed: Option<Why>, found: Option<Side>) -> Meaning {
    match (listed, found) {
        (Some(Why::Belief(why)), None) => Meaning::Regression(why),
        (Some(Why::Cost(why)), None) => Meaning::Fixed(why),
        (Some(Why::Belief(why) | Why::Cost(why)), Some(_)) => Meaning::Moved(why),
        (None, _) => Meaning::New,
    }
}

/// **A listed belief that agrees is a regression, and a listed cost that
/// agrees is a fix**, which is the one thing the two kinds of entry exist to
/// tell apart, so it is asked here rather than left to a failure's wording.
///
/// Mutation: swap the first two arms of [`meaning`]; this fails.
#[test]
fn a_belief_agreeing_is_a_regression_and_a_cost_agreeing_is_a_fix() {
    assert_eq!(
        meaning(Some(Why::Belief("b")), None),
        Meaning::Regression("b")
    );
    assert_eq!(meaning(Some(Why::Cost("c")), None), Meaning::Fixed("c"));
    assert_eq!(
        meaning(Some(Why::Cost("c")), Some(Side::Hazard)),
        Meaning::Moved("c")
    );
    assert_eq!(meaning(None, Some(Side::Control)), Meaning::New);
}

/// The rows whose hazard the plain store proves and the row only doubts, each
/// with why. Exact in both directions, as [`ACCEPTED`] is.
const UNCERTAIN: &[(&str, &str)] = &[
    (
        "fresh_call_d1_both",
        "the call is believed to write what it is handed, so the read back proves nothing indeterminate; see `WRITES_WHAT_IT_IS_HANDED`",
    ),
    (
        "fresh_call_d1_one",
        "the call is believed to write what it is handed, so the read back proves nothing indeterminate; see `WRITES_WHAT_IT_IS_HANDED`",
    ),
    (
        "fresh_call_d2_both",
        "the call is believed to write what it is handed, so the read back proves nothing indeterminate; see `WRITES_WHAT_IT_IS_HANDED`",
    ),
    (
        "fresh_call_d2_one",
        "the call is believed to write what it is handed, so the read back proves nothing indeterminate; see `WRITES_WHAT_IT_IS_HANDED`",
    ),
    (
        "width_memcpy_d0_both",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_memcpy_d0_one",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_call_d0_both",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_call_d0_one",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_ret_d0_both",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_ret_d0_one",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_memcpy_block_d0_both",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_memcpy_block_d0_one",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_call_stored_d0_both",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_call_deep_d0_both",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "fresh_memcpy_d0_one",
        "on one arm the slot holds what nothing wrote and on the other a null, and their join is a doubt (ADR-0057, #668)",
    ),
    (
        "fresh_memcpy_d1_one",
        "on one arm the slot holds what nothing wrote and on the other a null, and their join is a doubt (ADR-0057, #668)",
    ),
    (
        "fresh_memcpy_half_d1_one",
        "on one arm the slot holds what nothing wrote and on the other a null, and their join is a doubt (ADR-0057, #668)",
    ),
    (
        "fresh_memcpy_part_d1_both",
        "a copy that writes part of a pointer leaves the block in doubt rather than proved unwritten; a missed proof, since the part it misses is unwritten on every path",
    ),
    (
        "fresh_memcpy_maybe_d0_both",
        "the copy's source is written on one arm only, and the copy carries that doubt into the slot by its may-be-unwritten arm",
    ),
    (
        "fresh_memcpy_maybe_d1_both",
        "the copy's source is written on one arm only, and the copy carries that doubt into the slot by its may-be-unwritten arm",
    ),
    (
        "fresh_memcpy_d2_both",
        "#768: a read two levels down is never asked whether it is indeterminate",
    ),
    (
        "fresh_memcpy_d2_one",
        "#768: a read two levels down is never asked whether it is indeterminate",
    ),
    (
        "fresh_memcpy_half_d2_both",
        "#768: a read two levels down is never asked whether it is indeterminate",
    ),
    (
        "fresh_memcpy_half_d2_one",
        "#768: a read two levels down is never asked whether it is indeterminate",
    ),
    (
        "fresh_memcpy_maybe_d2_both",
        "#768: a read two levels down is never asked whether it is indeterminate",
    ),
    (
        "width_restored_d0_both",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "width_restored_d0_one",
        "a pointer read out of memory, returned by a call or written by one is believed, never known exactly, so a narrow one is a doubt (ADR-0060)",
    ),
    (
        "fresh_memcpy_unread_d1_both",
        "a copy of a length this check cannot read may not cover the slot, so what it leaves there is a doubt",
    ),
    (
        "fresh_memcpy_unread_d1_one",
        "a copy of a length this check cannot read may not cover the slot, so what it leaves there is a doubt",
    ),
    (
        "fresh_memcpy_short_d1_both",
        "a copy shorter than its destination leaves the rest as it was, one mark per block, so the rest is a doubt",
    ),
    (
        "fresh_memcpy_into_part_d0_both",
        "a copy shorter than a local array leaves the rest as it was, so the rest is a doubt",
    ),
    (
        "fresh_memcpy_unplaced_d1_both",
        "a copy into a destination this check cannot place may or may not land in the slot's block",
    ),
    (
        "fresh_memcpy_unplaced_d1_one",
        "a copy into a destination this check cannot place may or may not land in the slot's block",
    ),
    (
        "fresh_memcpy_from_local_d0_one",
        "the copy carries the mark into the slot on one arm and the other arm writes it, so the read after them is a doubt, where the store's proof is of its own load",
    ),
    (
        "fresh_memcpy_from_local_d1_one",
        "the copy carries the mark into the slot on one arm and the other arm writes it, so the read after them is a doubt, where the store's proof is of its own load",
    ),
    (
        "fresh_memcpy_from_local_maybe_d0_both",
        "the source is written on one arm, so what the copy carries may be written",
    ),
    (
        "fresh_memcpy_from_local_maybe_d1_both",
        "the source is written on one arm, so what the copy carries may be written",
    ),
    (
        "fresh_element_slot_d0_both",
        "a write through a pointer read back out of memory names no element, so the array is written in part and its start is a doubt (#667)",
    ),
    (
        "width_kept_void_d0_both",
        "what a call that kept an address may have stored is asked where the pointer leaves, as a doubt (ADR-0060)",
    ),
    (
        "width_kept_bytes_d0_both",
        "what a call that kept an address may have stored is asked where the pointer leaves, as a doubt (ADR-0060)",
    ),
    (
        "width_kept_block_d0_both",
        "what a call that kept an address may have stored is asked where the pointer leaves, as a doubt (ADR-0060)",
    ),
    (
        "fresh_stored_through_d0_both",
        "a walk off a local array's end writes it in part, so an element it may have missed is a doubt (#639)",
    ),
    (
        "stored_freed_memcpy_d0_both",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_d0_one",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_d1_both",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_d1_one",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_d2_both",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_d2_one",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_block_d0_both",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_block_d0_one",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_block_d1_both",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_block_d1_one",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_block_d2_both",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_memcpy_block_d2_one",
        "a library copy that leaves is asked whether what it carries was freed only as a doubt, since C defines it to copy bytes (ADR-0068)",
    ),
    (
        "stored_freed_restored_d0_both",
        "a pointer read back out of memory may hold what was never recorded, so it proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_restored_d0_one",
        "a pointer read back out of memory may hold what was never recorded, so it proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_restored_d1_both",
        "a pointer read back out of memory may hold what was never recorded, so it proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_restored_d1_one",
        "a pointer read back out of memory may hold what was never recorded, so it proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_restored_d2_both",
        "a pointer read back out of memory may hold what was never recorded, so it proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_restored_d2_one",
        "a pointer read back out of memory may hold what was never recorded, so it proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_ret_d0_both",
        "a pointer a call this check cannot read returns is lost, and a lost pointer proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_ret_d0_one",
        "a pointer a call this check cannot read returns is lost, and a lost pointer proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_ret_d1_both",
        "a pointer a call this check cannot read returns is lost, and a lost pointer proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_ret_d1_one",
        "a pointer a call this check cannot read returns is lost, and a lost pointer proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_ret_d2_both",
        "a pointer a call this check cannot read returns is lost, and a lost pointer proves nothing freed where it is stored (ADR-0045)",
    ),
    (
        "stored_freed_ret_d2_one",
        "a pointer a call this check cannot read returns is lost, and a lost pointer proves nothing freed where it is stored (ADR-0045)",
    ),
];

/// **A route is as certain as its plain store**: where the store spelling of
/// a row (the same belief, depth and shape) proves its hazard, an error under
/// `--allow-unknown`, the row's own hazard proves it too, unless
/// [`UNCERTAIN`] says why not. A proof that degrades to a doubt of the same
/// code is what the rows' own rule, which asks the code, cannot see.
///
/// Mutation: list in `UNCERTAIN` a row that is as certain as its store; this
/// fails. Mutation: the copy's certain arm leaves a block in doubt rather
/// than unwritten; this fails, on the copy rows the store proves.
#[test]
fn a_route_is_as_certain_as_its_plain_store() {
    let mut wrong = vec![];
    for &(name, row) in ROWS {
        let Some(&(store_name, store)) = ROWS.iter().find(|(_, store)| {
            store.route == Route::Store
                && store.belief == row.belief
                && store.depth == row.depth
                && store.shape == row.shape
        }) else {
            continue;
        };
        if row.route == Route::Store {
            continue;
        }
        let store_proves = ask_allowing(store_name, &program(store, true)).proves(store);
        let proves = ask_allowing(name, &program(row, true)).proves(row);
        let listed = UNCERTAIN.iter().any(|(listed, _)| *listed == name);
        if (store_proves && !proves) != listed {
            wrong.push(format!(
                "{name}: the store {} and this row {}; UNCERTAIN {}",
                if store_proves {
                    "proves"
                } else {
                    "does not prove"
                },
                if proves { "proves" } else { "does not" },
                if listed {
                    "lists it"
                } else {
                    "does not list it"
                }
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "rows whose certainty is not what UNCERTAIN says:\n{}",
        wrong.join("\n")
    );
}

/// A route, the one belief it is written for, the depths and shapes it is
/// written at, and why the rest are not.
type WrittenOnly = (
    Route,
    Belief,
    &'static [Depth],
    &'static [Shape],
    &'static str,
);

/// The routes written for one belief alone, at the depths and shapes named,
/// with why the rest are not: each asks one rule of the copy or of a kept
/// call, built from the corpus case that rule's mutation fails (#785). A
/// route here is excused everywhere else, and `NOT_WRITTEN` is not asked of
/// it.
const WRITTEN_ONLY: &[WrittenOnly] = &[
    (
        Route::MemcpyUnread,
        Belief::Fresh,
        &[Depth::D1],
        &[Shape::Both, Shape::One],
        "a copy's length is asked of freshness in a block, whose size is what it covers",
    ),
    (
        Route::MemcpyShort,
        Belief::Fresh,
        &[Depth::D1],
        &[Shape::Both],
        "the destination is a block of its own, so the slot's depth and an arm do not move it",
    ),
    (
        Route::MemcpyIntoPart,
        Belief::Fresh,
        &[Depth::D0],
        &[Shape::Both],
        "the destination is a local array of its own, so the slot's depth and an arm do not move it",
    ),
    (
        Route::MemcpyUnplaced,
        Belief::Fresh,
        &[Depth::D1],
        &[Shape::Both, Shape::One],
        "a destination this check cannot place may be the slot's block, which a local is not",
    ),
    (
        Route::MemcpyFromLocal,
        Belief::Fresh,
        &[Depth::D0, Depth::D1],
        &[Shape::Both, Shape::One],
        "a local source is asked of freshness, the belief it decides",
    ),
    (
        Route::MemcpyFromLocalMaybe,
        Belief::Fresh,
        &[Depth::D0, Depth::D1],
        &[Shape::Both],
        "the source is written on an arm already",
    ),
    (
        Route::StoredThrough,
        Belief::Fresh,
        &[Depth::D0],
        &[Shape::Both],
        "the array written is a local of its own, so the slot's depth and an arm do not move it",
    ),
    (
        Route::ElementSlot,
        Belief::Fresh,
        &[Depth::D0],
        &[Shape::Both],
        "the array written is a local of its own, so the slot's depth and an arm do not move it",
    ),
    (
        Route::KeptVoid,
        Belief::Width,
        &[Depth::D0],
        &[Shape::Both],
        "what a kept call may store is asked of width, at a pointer of its own",
    ),
    (
        Route::KeptBytes,
        Belief::Width,
        &[Depth::D0],
        &[Shape::Both],
        "what a kept call may store is asked of width, at a pointer of its own",
    ),
    (
        Route::KeptBlock,
        Belief::Width,
        &[Depth::D0],
        &[Shape::Both],
        "what a kept call may store is asked of width, at a pointer of its own",
    ),
];

/// Whether `NOT_WRITTEN` says, with a reason, that no program is written for
/// `row`.
fn excused(row: Row) -> bool {
    if let Some(&(_, belief, depths, shapes, why)) =
        WRITTEN_ONLY.iter().find(|(route, ..)| *route == row.route)
    {
        return !why.is_empty()
            && !(belief == row.belief
                && depths.contains(&row.depth)
                && shapes.contains(&row.shape));
    }
    NOT_WRITTEN.iter().any(|unwritten| {
        !unwritten.why.is_empty()
            && unwritten.route == row.route
            && unwritten.depth.is_none_or(|depth| depth == row.depth)
            && unwritten.belief.is_none_or(|belief| belief == row.belief)
            && unwritten.shape.is_none_or(|shape| shape == row.shape)
    })
}

/// The table is every combination of its axes, less what `NOT_WRITTEN` says
/// cannot be written, once each, and each row's name is spelled from its axes,
/// so that a row left out is a failure rather than a gap and a name cannot
/// claim one row while testing another.
///
/// Mutation: delete a row from `routes!`. This fails, naming it.
#[test]
fn the_matrix_is_every_combination_of_its_axes() {
    let mut missing = vec![];
    for belief in BELIEFS {
        for route in ROUTES {
            for depth in DEPTHS {
                for shape in SHAPES {
                    let row = Row {
                        belief,
                        route,
                        depth,
                        shape,
                    };
                    let listed = ROWS.iter().filter(|(_, listed)| *listed == row).count();
                    match (excused(row), listed) {
                        (false, 1) | (true, 0) => {}
                        _ => missing.push(format!("{} (listed {listed} times)", name_of(row))),
                    }
                }
            }
        }
    }
    let misnamed: Vec<&str> = ROWS
        .iter()
        .filter(|(name, row)| *name != name_of(*row))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        missing.is_empty(),
        "rows the axes call for that the table does not hold once: {missing:?}"
    );
    assert!(
        misnamed.is_empty(),
        "rows whose name is not spelled from their axes: {misnamed:?}"
    );
}

/// A row's name as its axes spell it.
fn name_of(row: Row) -> String {
    let belief = match row.belief {
        Belief::Width => "width",
        Belief::Fresh => "fresh",
        Belief::Lost => "lost",
        Belief::Stale => "stale",
        Belief::Reborn => "reborn",
        Belief::Caller => "caller",
        Belief::Unreplaced => "unreplaced",
        Belief::StoredOut => "stored_out",
        Belief::StoredFreed => "stored_freed",
        Belief::LeftBehind => "left_behind",
        Belief::PartPointer => "part_pointer",
        Belief::PartLocal => "part_local",
        Belief::Returned => "returned",
        Belief::HeldPointers => "held_pointers",
    };
    let route = match row.route {
        Route::Store => "store",
        Route::Memcpy => "memcpy",
        Route::Realloc => "realloc",
        Route::Call => "call",
        Route::Ret => "ret",
        Route::MemcpyFromBlock => "memcpy_block",
        Route::MemcpyHalf => "memcpy_half",
        Route::MemcpyPart => "memcpy_part",
        Route::MemcpyMaybe => "memcpy_maybe",
        Route::CallStored => "call_stored",
        Route::CallDeep => "call_deep",
        Route::ReallocParam => "realloc_param",
        Route::Restored => "restored",
        Route::MemcpyUnread => "memcpy_unread",
        Route::MemcpyShort => "memcpy_short",
        Route::MemcpyIntoPart => "memcpy_into_part",
        Route::MemcpyUnplaced => "memcpy_unplaced",
        Route::MemcpyFromLocal => "memcpy_from_local",
        Route::MemcpyFromLocalMaybe => "memcpy_from_local_maybe",
        Route::StoredThrough => "stored_through",
        Route::ElementSlot => "element_slot",
        Route::KeptVoid => "kept_void",
        Route::KeptBytes => "kept_bytes",
        Route::KeptBlock => "kept_block",
        Route::ByAddress => "by_address",
        Route::ByAddressLoaded => "by_address_loaded",
    };
    let depth = match row.depth {
        Depth::D0 => "d0",
        Depth::D1 => "d1",
        Depth::D2 => "d2",
    };
    let shape = match row.shape {
        Shape::Both => "both",
        Shape::One => "one",
    };
    format!("{belief}_{route}_{depth}_{shape}")
}

/// `ACCEPTED` names only rows of the table, once each, and gives a reason.
#[test]
fn every_accepted_row_is_in_the_table_once_with_a_reason() {
    for (place, (name, _, why)) in ACCEPTED.iter().enumerate() {
        assert!(
            ROWS.iter().any(|(row, _)| row == name),
            "`ACCEPTED` names `{name}`, which is no row"
        );
        let (Why::Cost(why) | Why::Belief(why)) = why;
        assert!(!why.is_empty(), "`ACCEPTED` gives `{name}` no reason");
        assert!(
            ACCEPTED[place + 1..]
                .iter()
                .all(|(other, _, _)| other != name),
            "`ACCEPTED` names `{name}` twice"
        );
    }
}

/// `UNCERTAIN` names only rows of the table, once each, and gives a reason:
/// a name that is no row would excuse nothing and pass, since the certainty
/// guard asks only the rows.
///
/// Mutation: add `("no_such_row", "x")` to `UNCERTAIN`; this fails.
#[test]
fn every_uncertain_row_is_in_the_table_once_with_a_reason() {
    for (place, (name, why)) in UNCERTAIN.iter().enumerate() {
        assert!(
            ROWS.iter().any(|(row, _)| row == name),
            "`UNCERTAIN` names `{name}`, which is no row"
        );
        assert!(!why.is_empty(), "`UNCERTAIN` gives `{name}` no reason");
        assert!(
            UNCERTAIN[place + 1..]
                .iter()
                .all(|(other, _)| other != name),
            "`UNCERTAIN` names `{name}` twice"
        );
    }
}

/// Every program the matrix writes is C this frontend reads, built at
/// `--safety off` with nothing said, so that a template that is wrong about
/// C fails as itself rather than as a row that disagrees.
///
/// Mutation: have `width`'s control assign `&c`, a `char *`, to its `int *`
/// rather than `&i`; this fails, beside the rows that template writes.
#[test]
fn every_generated_program_is_c_this_frontend_reads() {
    let mut refused = vec![];
    for &(name, row) in ROWS {
        for (side, hazard) in [("hazard", true), ("control", false)] {
            let answer = ask(name, &program(row, hazard), "off");
            if answer.exit != 0 || !answer.reports.is_empty() {
                refused.push(format!("{name} {side}: {answer:?}"));
            }
        }
    }
    assert!(
        refused.is_empty(),
        "generated programs this frontend refuses:\n{}",
        refused.join("\n")
    );
}

/// The whole matrix's answers are what was blessed: one line per row, the
/// hazard's and the control's exit and codes.
///
/// **What finds a regression between two builds**, where the rows find a
/// defect in one: a change that silences a route and its plain store together
/// keeps every row agreeing with itself and moves this. See ADR-0062.
///
/// Mutation: change one line of `tests/routes.expected`. This fails.
#[test]
fn the_matrix_answers_what_was_blessed() {
    let mut lines: Vec<String> = ROWS
        .iter()
        .map(|&(name, row)| {
            let hazard = ask(name, &program(row, true), "memory");
            let control = ask(name, &program(row, false), "memory");
            let allowed = ask_allowing(name, &program(row, true));
            format!(
                "{name} hazard={} control={} allowed={}",
                hazard.summary(),
                control.summary(),
                allowed.summary()
            )
        })
        .collect();
    lines.sort();
    let actual = lines.join("\n") + "\n";
    let path = expected_path();
    if blessing() {
        std::fs::write(&path, &actual)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| {
            panic!(
                "{}: {error}; run with SAFEC_BLESS=1 to write it",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    if expected != actual {
        let moved: Vec<String> = expected
            .lines()
            .filter(|old| !actual.lines().any(|new| new == *old))
            .map(|old| format!("- {old}"))
            .chain(
                actual
                    .lines()
                    .filter(|new| !expected.lines().any(|old| old == *new))
                    .map(|new| format!("+ {new}")),
            )
            .collect();
        panic!(
            "the matrix answers differently from {}:\n{}\n\nRead the difference, then run \
             with SAFEC_BLESS=1 to write what it answers now.",
            path.display(),
            moved.join("\n")
        );
    }
}

/// A pointer this check cannot say starts an object, a call's result or
/// a pointer it stopped following, is asked where it is stored into memory,
/// and a store into a local is no door. See ADR-0065.
const ASKED_WHERE_STORED: &str = "a pointer this check cannot say starts an object is asked where it is stored into memory, and a store into a local is no door (ADR-0065)";
/// See ADR-0045.
const LOST_READ_FROM_A_LOCAL: &str = "a lost pointer read out of a local proves nothing where it is used, and one read out of memory is believed as it was stored (ADR-0045)";

/// The pairs of a row at depth zero and its row at depth one that answer
/// differently, with why: each a rule that tells a local from a block on
/// purpose. Every other pair answers alike in its codes and its exits,
/// which [`a_route_answers_alike_at_depth_zero_and_one`] holds, and a
/// pair listed here that no longer differs fails
/// [`every_pair_kept_apart_still_differs`]. Named as the rows are, without
/// the depth.
const APART: &[(&str, &str)] = &[
    ("caller_ret_both", ASKED_WHERE_STORED),
    ("caller_ret_one", ASKED_WHERE_STORED),
    ("width_ret_both", ASKED_WHERE_STORED),
    ("width_ret_one", ASKED_WHERE_STORED),
    ("lost_restored_both", ASKED_WHERE_STORED),
    ("lost_restored_one", ASKED_WHERE_STORED),
    ("stale_restored_both", ASKED_WHERE_STORED),
    ("stale_restored_one", ASKED_WHERE_STORED),
    ("reborn_restored_both", ASKED_WHERE_STORED),
    ("reborn_restored_one", ASKED_WHERE_STORED),
    ("reborn_ret_both", ASKED_WHERE_STORED),
    ("reborn_ret_one", ASKED_WHERE_STORED),
    (
        "lost_ret_both",
        "a lost pointer read out of a local proves nothing (ADR-0045), and one stored into memory is asked there (ADR-0065)",
    ),
    (
        "lost_ret_one",
        "a lost pointer read out of a local proves nothing (ADR-0045), and one stored into memory is asked there (ADR-0065)",
    ),
    (
        "stale_ret_both",
        "the freed pointer passed is reported where it is freed and passed (SC0401, SC0407) and not again where it is read back out of memory, and stored into memory it is asked there (ADR-0065)",
    ),
    (
        "stale_ret_one",
        "the freed pointer passed is reported where it is freed and passed (SC0401, SC0407) and not again where it is read back out of memory, and stored into memory it is asked there (ADR-0065)",
    ),
    ("lost_store_both", LOST_READ_FROM_A_LOCAL),
    ("lost_store_one", LOST_READ_FROM_A_LOCAL),
    ("stale_store_both", LOST_READ_FROM_A_LOCAL),
    ("stale_store_one", LOST_READ_FROM_A_LOCAL),
    ("reborn_store_both", LOST_READ_FROM_A_LOCAL),
    ("reborn_store_one", LOST_READ_FROM_A_LOCAL),
    ("width_call_both", MAY_FREE),
    ("width_call_one", MAY_FREE),
    (
        "width_store_both",
        "a width read out of memory is a lower bound, so a load proves no access too wide where a local's object does (ADR-0061)",
    ),
    (
        "width_store_one",
        "a width read out of memory is a lower bound, so a load proves no access too wide where a local's object does (ADR-0061)",
    ),
    (
        "fresh_memcpy_one",
        "a copy into a local is believed as its type where its source is fresh, and a load out of memory that may be fresh on one path believes what was recorded (ADR-0061), so the block's form doubts a width the local's does not",
    ),
    (
        "fresh_memcpy_from_local_one",
        "a copy into a local is believed as its type where its source is fresh, and a load out of memory that may be fresh on one path believes what was recorded (ADR-0061), so the block's form doubts a width the local's does not",
    ),
];

/// A route answers a slot in a local as it answers one a level below a
/// local: every row at [`Depth::D0`] of a belief [`compared_by_depth`] and
/// its row at [`Depth::D1`] give the same codes and the same exit on the
/// hazard, on the control and under `--allow-unknown`, unless [`APART`] says
/// why the two differ (#791), and a pair it lists still has the same hazard
/// and control exits.
///
/// **Read off the blessed summary**, which
/// `the_matrix_answers_what_was_blessed` holds to what the matrix answers:
/// a change that parts the two depths fails that test first, and this one
/// once the summary is blessed, so that blessing it does not accept it.
///
/// Mutation: in `tests/routes.expected`, take `SC0413` out of
/// `width_memcpy_d1_both`'s `hazard=`; this fails, where comparing the exits
/// alone did not. Mutation: change `width_store_d1_both`'s `hazard=1` to
/// `hazard=0`, a pair `APART` lists; this fails. Mutation: pair each row
/// with the row at `Depth::D0` rather than `Depth::D1`; this fails, naming a
/// row paired with itself.
#[test]
fn a_route_answers_alike_at_depth_zero_and_one() {
    let summary = std::fs::read_to_string(expected_path())
        .unwrap_or_else(|error| panic!("{}: {error}", expected_path().display()))
        .replace("\r\n", "\n");
    let mut pairs = 0;
    let mut parted = vec![];
    for (near, far) in depth_pairs() {
        pairs += 1;
        let (here, there) = (answers(&summary, near), answers(&summary, far));
        // **A pair kept apart still has the same hazard exit and the same
        // control exit**: what `APART` excuses is a code or what is proved,
        // and a hazard one depth accepts is a difference no record names.
        let kept_apart = APART.iter().any(|(pair, _)| *pair == pair_name(near));
        let differs = if kept_apart {
            exits(here) != exits(there)
        } else {
            here != there
        };
        if differs {
            parted.push(format!("{near} {here}\n{far} {there}"));
        }
    }
    assert!(pairs > 0, "no row at depth 0 has its row at depth 1");
    assert!(
        parted.is_empty(),
        "a route answers a local and the level below one differently, and `APART` does not say why:\n{}",
        parted.join("\n")
    );
}

/// A pair `APART` lists still answers differently: a table that keeps a pair
/// which agrees again hides the next difference that pair grows. Mutation:
/// list a pair that agrees, `width_memcpy_both`; this fails.
#[test]
fn every_pair_kept_apart_still_differs() {
    let summary = std::fs::read_to_string(expected_path())
        .unwrap_or_else(|error| panic!("{}: {error}", expected_path().display()))
        .replace("\r\n", "\n");
    for &(pair, why) in APART {
        assert!(!why.is_empty(), "`{pair}` is kept apart with no reason");
        let (near, far) = depth_pairs()
            .into_iter()
            .find(|(near, _)| pair_name(near) == pair)
            .unwrap_or_else(|| panic!("`APART` lists `{pair}`, which is no pair of the matrix"));
        assert_ne!(
            answers(&summary, near),
            answers(&summary, far),
            "`{pair}` answers alike at depth 0 and 1 now; take it out of `APART`"
        );
    }
}

/// Whether a belief's rows at depth zero and one are held to answer alike.
///
/// **Not the door questions** (#801): their hazard hands the slot's container
/// to a call this check cannot read, and that is a local's address at depth
/// zero and a block at depth one. The address makes the local escape
/// (ADR-0017), so a free of what it holds is a double-free doubt; the block
/// handed may be freed by the call (ADR-0039), so a write into it is a use
/// doubt. The two depths differ by what the hazard does to its container, not
/// by the route, so comparing them would ask the escape rather than the door.
/// The door's own answer is asked by each row's rule and against the plain
/// store by `a_route_is_as_certain_as_its_plain_store`. A `match`, so that a
/// belief added answers here.
///
/// **Not ADR-0073's three either, until #919 and #920 close**: at depth one
/// `x` is read back out of a block, and what that load carries is what those
/// two issues and #631 are about, each row of it listed in `ACCEPTED`. The
/// pairs differ in their exits, which `APART` cannot excuse, so comparing
/// them would name those issues a second time rather than a route.
///
/// Mutation: answer `true` for every belief; `a_route_answers_alike_at_depth_zero_and_one`
/// fails, `stored_out_store_d0_both` among the pairs it names.
fn compared_by_depth(belief: Belief) -> bool {
    match belief {
        Belief::Width
        | Belief::Fresh
        | Belief::Lost
        | Belief::Stale
        | Belief::Reborn
        | Belief::Caller
        | Belief::Unreplaced => true,
        Belief::StoredOut
        | Belief::StoredFreed
        | Belief::LeftBehind
        | Belief::PartPointer
        | Belief::PartLocal
        | Belief::Returned
        | Belief::HeldPointers => false,
    }
}

/// Every row at depth zero beside its row at depth one.
fn depth_pairs() -> Vec<(&'static str, &'static str)> {
    let mut pairs = vec![];
    for &(near, row) in ROWS
        .iter()
        .filter(|(_, row)| row.depth == Depth::D0 && compared_by_depth(row.belief))
    {
        let twin = Row {
            depth: Depth::D1,
            ..row
        };
        // **A row with no twin is one `NOT_WRITTEN` excuses**, not one this
        // skips: `width_call_deep_d0_both` is written at one depth only, and
        // says so there.
        let Some(&(far, found)) = ROWS.iter().find(|(_, other)| *other == twin) else {
            assert!(
                excused(twin),
                "`{near}` has no row at depth 1 and nothing excuses it"
            );
            continue;
        };
        // And it is a row one level below, not `near` itself.
        assert_eq!(found.depth, Depth::D1, "`{near}` is paired with `{far}`");
        pairs.push((near, far));
    }
    pairs
}

/// The hazard's and the control's exit out of what a row answers.
fn exits(answer: &str) -> (&str, &str) {
    let exit = |side: &str| {
        answer
            .split(' ')
            .find_map(|field| field.strip_prefix(side))
            .and_then(|it| it.split(':').next())
            .unwrap_or_else(|| panic!("`{answer}` has no `{side}`"))
    };
    (exit("hazard="), exit("control="))
}

/// A row's name without its depth, the name `APART` lists a pair by.
fn pair_name(near: &str) -> String {
    near.replacen("_d0_", "_", 1)
}

/// What a row answers, `hazard=E:codes control=E:codes allowed=E:codes`,
/// from the summary: every field after its name.
fn answers<'s>(summary: &'s str, name: &str) -> &'s str {
    summary
        .lines()
        .find(|line| line.split(' ').next() == Some(name))
        .and_then(|line| line.split_once(' '))
        .map(|(_, answer)| answer)
        .unwrap_or_else(|| panic!("the summary has no line for `{name}`"))
}

fn expected_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("routes.expected")
}

/// Whether to rewrite the summary, refused under CI, as the corpus's own
/// blessing is in `tests/cases.rs`: a run that rewrites its own expectations
/// proves nothing.
fn blessing() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    decide_blessing(set("SAFEC_BLESS"), set("CI"))
}

/// Split from the environment so that the refusal can be tested, as
/// `tests/cases.rs` splits its own.
fn decide_blessing(asked: bool, under_ci: bool) -> bool {
    assert!(
        !(asked && under_ci),
        "SAFEC_BLESS is set under CI. Expected output is written by a person who then reads the diff."
    );
    asked
}

/// Mutation: return `asked` without the assertion; this fails.
#[test]
#[should_panic(expected = "SAFEC_BLESS is set under CI")]
fn blessing_the_matrix_is_refused_under_ci() {
    decide_blessing(true, true);
}

/// The summary is rewritten when blessing is asked for outside CI, and only
/// then. Mutation: have `decide_blessing` answer `false` or `true` whatever
/// it is asked; this fails.
#[test]
fn the_summary_is_rewritten_only_when_blessing_is_asked_for() {
    assert!(decide_blessing(true, false));
    assert!(!decide_blessing(false, false));
    assert!(!decide_blessing(false, true));
}
