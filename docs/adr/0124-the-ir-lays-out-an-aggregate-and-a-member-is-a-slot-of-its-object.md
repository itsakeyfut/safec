---
status: "accepted"
date: 2026-11-23
decision-makers: itsakeyfut
---

# The IR lays out an aggregate, and a member is a slot of its object

## Context and Problem Statement

Phase 11 adds structs, unions and the rest of the aggregates (#614). The IR
has no struct type and no member projection, the backend writes pointer
arithmetic as `getelementptr` over an element's type, and the memory check
measures the same element with `TranslationUnit::size_of`. Four questions
decide the shape of every sub-issue under #614, so they are settled once:
who lays an aggregate out, what a member means to the memory check, how a
struct crosses a call, and what the check concludes about comparing or
subtracting two pointers.

## Decision Drivers

* safec has to replace `cc` in an existing build (roadmap, Phase 11), so a
  struct passed by value has to meet objects other compilers made
* one width answered in two places has already been flagged as the first
  place the backend and the check could disagree (#638), and a disagreement
  there is the check reading bytes the program does not
* Phase 11's rule: the memory check reads each construct or concludes
  `Unknown` about it, never safe without reading it

## Considered Options

For the layout: the IR as the one authority, against LLVM's data layout
with the IR checked against it.

For a member: a slot of its object, read with the machinery an array's
element already has; `Unknown` for every access through a member; or a
storage of its own per member.

For a call: each target's C ABI, as `clang` classifies it; one target now
and the rest later; or a convention of safec's own.

For comparing or subtracting pointers: doubted where the two cannot be shown
to point into one object, against not asked and named as a silence.

## Decision Outcome

* **The IR lays an aggregate out, and nothing else does.** `TranslationUnit`
  answers `size_of`, `align_of` and `offset_of` for a struct or union from
  the target, as C17 6.7.2.1 and the target's ABI say, and the backend
  writes a member's address as a byte offset from its object (`getelementptr
  i8`), never as an LLVM struct type it would lay out again. That the IR
  agrees with `clang` is held by a test that compiles the same declarations
  with `clang` and compares, as the target table already is (ADR-0013).
* **A member is a slot of its object.** `Projection::Field` names a member by
  its index; to the memory check it is the object's storage at the member's
  byte offset and width, read with the same positions and per-slot written
  state an array element has (ADR-0092, ADR-0094). A write to a member is a
  write to that slot of the object, and a union's members are slots at the
  same offset. Every reader of a place answers `Projection::Field` by name
  (ADR-0113).
* **A struct crosses a call as the target's C ABI says.** The backend
  classifies a struct argument or result for each target, as `clang` does
  for Win64 and for x86-64 System V, and passes it in registers or through
  memory (`byval`, `sret`) to match. A test links a safec object against a
  `clang` object for each shape and runs it.
* **Comparing or subtracting two pointers is doubted unless they are shown
  to point into one object.** A relational comparison or a difference whose
  operands cannot both be shown to name the same object is `Unknown` under a
  new memory code (C17 6.5.8 p5, 6.5.6 p9); one whose operands certainly
  name one object is not reported.

### Confirmation

* **The layout** (#1200): `the_layout_of_every_struct_shape_is_clangs` in
  `tests/layout.rs` compiles each shape with `clang` on every target and
  compares `TranslationUnit::push_struct`'s offsets, size and alignment.
  Mutation: place each member at the end of the one before it in
  `record::lay_out`; it fails. `sizeof` in the type check asks the same
  `record::lay_out`.
* **A member is a slot of its object** (#1200): `Projection::Field` is an
  `E0004` at every reader of a place's steps, and the memory check reads
  `&(*p).m` as `p` moved by the member's offset.
  `a_struct_member_is_a_slot_of_its_object` holds it: with the offset taken
  as zero, its write past a block's end reaches 8 bytes rather than 16.
  A pointer from a member's address walks the member alone, the member's
  bytes being the row it walks, since C17 6.5.6 p7 makes a member an array
  of one: `a_member_s_address_walks_the_member_alone` is silent with no row
  entered.
* **A struct crosses a call as the target's C ABI says** (#1201):
  `a_struct_by_value_meets_clangs_abi` in `tests/abi.rs` links an object
  this compiler built against one `clang` built, both ways round, for each
  size class and after five integers, on each runner's own host: Win64 on
  Windows, x86-64 System V on Linux, and AArch64 on macOS, whose runner is
  Apple silicon. The corpus cases `a_struct_crosses_a_call_as_system_v_says`,
  `a_struct_crosses_a_call_as_aarch64_says` and
  `a_struct_crosses_a_call_as_win64_says` hold each convention's text on
  every host. `abi::classify` answers Win64, System V and AArch64 by name,
  for a whole signature at once since System V counts the registers left,
  and refuses every other target. Mutations: classify a Win64 struct of 8
  bytes as memory, and `a_struct_by_value_meets_clangs_abi` fails on
  Windows; drop System V's count of free registers, and
  `a_struct_that_does_not_fit_the_registers_left_goes_in_memory` fails.
* **A union is laid out as `clang` lays it out** (#1203), every member at
  offset 0 and the whole its largest member rounded to the strictest
  alignment: `the_layout_of_every_struct_shape_is_clangs` holds union shapes
  beside the structs on every target. Mutation: size a union as its first
  member in `record::lay_out_union`; it fails.
* **A bit-field is packed as `clang` packs it** (#1206), by the Microsoft,
  Itanium or AAPCS convention its target follows:
  `the_layout_of_every_bit_field_shape_is_clangs` holds structs and unions
  of bit-fields against `clang`'s record layout dump on every target. A
  bit-field is read and written through its unit, a member of the unit's
  unsigned type, so it is a slot of its object as any member is.
  Mutation: let a bit-field straddle its unit in
  `record::lay_out_itanium`; it fails.
* **A flexible array member is an array of none** (#1208): it takes no
  bytes of its struct, and a walk through it is bounded by the row the
  struct is in, the allocation's where it is one, rather than by a row of
  its own. `a_flexible_array_member_is_bounded_by_its_allocation` writes the
  last allocated element in silence and one past it under `SC0412`.
  Mutation: enter the member's row of no bytes; `fits` is reported.
* Still unguarded, because comparing pointers is not lowered yet: a corpus
  case for the comparison's code, which its sub-issue of #614 adds as it
  lands.

### Consequences

* Good, because the check and the backend cannot disagree about where a
  member is, since only the IR says.
* Good, because a struct is read by machinery the check already proves things
  with, so the first struct program is checked rather than refused.
* Bad, because a write to one member is a weak update of a slot, which keeps
  what the other members held: a false report rather than a silence (row 4).
* Bad, because two ABIs are written and kept in step with `clang`.
* Bad, because `p < q` over pointers the check cannot place is doubted.
* What would reverse this: a member needing facts a slot cannot hold, which
  is #1192's question for statics and would be a record of its own.

## More Information

#614 and its sub-issues; #638 for the two widths; ADR-0013, ADR-0092,
ADR-0094 and ADR-0113.
