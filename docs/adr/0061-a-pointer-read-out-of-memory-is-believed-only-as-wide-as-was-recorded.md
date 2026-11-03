---
status: "accepted"
date: 2026-11-03
decision-makers: itsakeyfut
---

# A pointer read out of memory is believed only as wide as was recorded there

## Context and Problem Statement

ADR-0060 believes a pointer read out of memory to point at one whole object of
the type it is read as, no wider than what the memory is recorded to hold. Where
nothing is recorded, nothing narrows the type: `Allocations::believed_read`
answers `bound.map_or(as_read, ...)`, and `Known::memory_levels` gives a local
neither loaded nor written by a call no bound. So every route that reaches
memory without recording a width is believed whole, and seventeen fixes from
#622 to #724 were such a route found by review after it was silent: #622, #623,
#624, #627, #629, #632, #634, #636, #675, #677, #679, #690, #711, #713, #715,
#717, #719.

This record replaces ADR-0060's belief about a load and nothing else. A
parameter, a call's result and what a call writes into memory it was handed are
still believed as the type they arrive as, and a pointer that leaves is still
asked as ADR-0060 says.

## Decision Drivers

* **A forgotten width is a doubt, not a belief.** The failure this project
  exists to avoid is saying safe without having read it, and a route that
  records nothing has read nothing.
* **The common case still proves.** `*box = &q; int *t = *box; *t = 1;` is
  correct and has to stay silent.
* **What arrives is still believed.** Nothing here reads a caller or a callee,
  and ADR-0017 and ADR-0060 believe what arrives from them.

## Considered Options

* **Record the width of what a store names, then believe a load no wider than
  what was recorded, and nothing where nothing was.**
* Believe nothing where nothing was recorded, without recording more.
* Record `0` at each writer that cannot say a width, and keep believing the
  type where nothing was recorded.

## Decision Outcome

Chosen option: **record, then believe no wider than the record.**

**What a store names is recorded at its width.** A value that names sites or
locals, at its start and not inside a local, is recorded at the least of their
widths: each site's `Extent` lower bound, and each local at its size. A value
off its start or inside a local records `0`, and so does a lost value, which
may hold an object no site it names stands for. A value naming nothing, a
null, records nothing. The store into memory records it in
`Contents::stored_at_most`, and a local reached through its address on the way
of a read gives it from its own value in `Known::memory_levels`.

**What arrives is recorded as the type it arrives as.** A local or a site a
call this check cannot read was handed is believed written as it was handed,
and where it was handed nothing that bounds a width, as the type it is read as.

**A load believes no wider than what its memory records, and nothing where
nothing is recorded**: `believed_read` answers `0` where no level is bounded.
**Except a read already reported as indeterminate** (ADR-0057), which is
believed as its type, since the defect is reported once, where it is read.

### Confirmation

Each mutation applied on its own, the workspace run, and the file restored.

* `Allocations::believed_read` believing the type again where nothing is
  recorded: `a_pointer_read_out_of_memory_calloc_made_is_not_reported_as_indeterminate`
  goes silent, among fourteen.
* The store arm recording no width for a value that names objects:
  `a_copy_from_a_source_read_out_of_memory_is_recorded` is reported, among 133.
* `Known::memory_levels` giving a local's own value no width:
  `a_live_pointer_written_into_a_local_through_its_address_is_read_back_in_silence`
  is reported, among 21.
* `Held::recorded_width` ignoring an offset and an inner address:
  `a_pointer_off_its_start_stored_as_void_and_read_through_an_int_alias`
  changes, among twelve.
* The walk's arm for a value a call wrote, handed nothing, at `0` rather than
  `AS_TYPED`: `a_live_pointer_through_a_locals_address_stored_in_an_allocation_is_read_in_silence`
  is reported, among 23. The store's arm without `AS_TYPED`:
  `a_pointer_a_call_handed_nothing_may_have_written_on_one_arm_is_believed_as_its_type_when_stored_and_read_back`
  is reported, and nothing else.
* `Allocations::handed_site` recording no bound:
  `a_pointer_read_out_of_memory_a_call_was_handed_is_not_reported_as_indeterminate`
  gains an `SC0412`.
* A read already reported as indeterminate asked its width as well:
  `a_byte_copied_out_of_a_local_is_doubted_where_read_back_as_a_pointer` gains
  an `SC0412`, among 23.
* `Held::recorded_width` not asking whether a value is lost:
  `a_lost_value_joined_into_a_live_allocation_stored_and_read_back_is_doubted`
  goes silent, among seven.
* `report::reached_by_library` keeping the first report per pointer per call:
  `a_proof_at_a_later_argument_of_a_library_call_is_not_displaced_by_a_doubt_at_an_earlier_one`
  exits 0.
* `Held::recorded_width` taking the greatest width rather than the least moves
  no case, because a load carries what the store named and `report::fits`
  folds each one's extent; its doc comment says so.

### Consequences

* Good, because a route that reaches memory with no record at all is a doubt.
  Measured on a prototype at `0586f663`: reversing six past width fixes one at
  a time left 61 cases silent on `main` and 33 on the prototype.
* Bad, because **a record is a bound and is never replaced**, so a route that
  forgets its width after another store recorded one is still believed by that
  record. The 33 are that shape, and this record does not close them.
* Bad, because memory nothing was stored into is believed nothing too:
  `calloc`'s nulls read back and tested are doubted, a false `Unknown`, by the
  user's decision; and a lost value stored and read back is doubted.
* Bad, because **a doubt became the common answer at a library call**, and the
  first report kept per pointer per call let a doubt about `*pp` stand over the
  proof that `memcpy(*pp, pp, 16)` runs past `pp`. The proof now replaces it.

## Pros and Cons of the Options

### Believe nothing where nothing was recorded, without recording more

* Good, because it is one line.
* Bad, because no store of a local's address or an allocation records a width
  today, so the prototype moved 237 cases and newly reported 22 correct ones,
  `*box = &q; int *t = *box; *t = 1;` among them.

### Record `0` at each writer that cannot say a width

* Good, because a fresh or `calloc` block is believed as before.
* Bad, because a writer nobody has written yet records nothing and is believed
  whole, which is the shape of all seventeen.

## More Information

* `crates/safec-ir/src/memory/parts.rs`: `Held::recorded_width`, `AS_TYPED`.
* `crates/safec-ir/src/memory/transfer.rs`: `Allocations::believed_read`, the
  store arm that records `Contents::stored_at_most`, `Allocations::handed_bounds`
  and `Allocations::handed_site`.
* `crates/safec-ir/src/memory/known.rs`: `Known::memory_levels`.
* Issue #735 and its design comment.
