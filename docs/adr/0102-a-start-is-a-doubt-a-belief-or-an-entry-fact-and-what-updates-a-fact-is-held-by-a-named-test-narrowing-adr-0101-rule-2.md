---
status: "accepted"
date: 2026-11-18
decision-makers: itsakeyfut
---

# A start is a doubt, a belief or an entry fact, and what updates a fact is held by a named test, narrowing ADR-0101 rule 2

## Context and Problem Statement

ADR-0101 rule 2 says a field of `Known` or `Held` starts believed only where an
operation grants its safe value or a route-matrix belief exercises it, and
otherwise starts in doubt. #1029 was to apply it to the eighteen fields
`tests/routes.rs` lists as `Field::Exempt` with `Start::Quiet`.

Read field by field, none of the eighteen starts believed. The eleven of
`Known` start at `NOTHING_YET`, a may-fact about what the function has done,
false at its entry because it has done nothing; the seven of `Held` start at
what a value is before any door gave it anything. Each start is the fact, and
starting it in doubt would make every function report what no execution of it
has done. Of every quiet start in the two tables, two are beliefs about what
the caller passed, `points_to` and `state`, and both have route-matrix rows.

What can go wrong with a fact is not where it starts but a door that should
update it and does not, and for several of the eighteen nothing fails when
their updates are dropped.

## Decision Drivers

* A start is a belief only where it assumes something about the caller; one
  that states what the function has not yet done is exact.
* A door that misses an update is a silence wherever the fact was the safe
  answer's guard, and a guard is a named test that fails when it is missed.

## Considered Options

* Apply ADR-0101 rule 2 as written, starting the eighteen in doubt.
* Tell the three kinds of start apart in the tables, and leave the updates.
* Tell them apart, and hold each exempt field's updates by a named test.

## Decision Outcome

Chosen: the third, by the owner's decision on #1029.

1. **A start is one of three**: in doubt; a belief about what the caller
   passed, which a route-matrix belief must exercise; or an entry fact, exact
   because nothing has happened yet, with the reason said.
2. **Every field exempt from the route matrix names the tests that fail when
   its updates are dropped**, and a test fails where a name is not a test. A
   field nothing fails for gets a case that does, or says why no C reaches it.
3. ADR-0101 rule 2's "otherwise starts in doubt" is narrowed to a start that is
   neither an entry fact nor a belief a row exercises; rules 1 and 3 stand.

### Confirmation

Measured on #1029's branch by putting each exempt field back to where it
starts after every element and terminator, in the solver and in the report's
walk, and running the unit tests, the corpus and the route matrix; the tests
`held_by` names in `tests/routes.rs` are among those that failed. Tests
failed per field: `handed_away` 1, `exposed` 343, `realloced` 23, `pending`
46, `calls` 8, `exposed_after_call` 1, `extent` 1,707, `stored_in_part` 1,
`left_at` 4, `held_levels` 20, `other_written` 13, `part_before` 4,
`writes_elsewhere` 38, `named_at_read` 4, `returned_by` 52, `site_element`
20, `array_edge` 10, `stored_through` 72, `may_be_null` 11, `vouched` 7.

Two failed nothing, and their rows say why:

* `exposed_in_part`: no program isolates it since ADR-0101 rule 1, which
  makes an array an earlier call held an element of a may-write whether or
  not the field is set. Three programs measured alike with it dropped.
* `inner_address`: no C reaches it, since the frontend lowers every address
  inside a local to its start and arithmetic; its reader is held by the unit
  test `an_edge_inside_a_local_is_not_read_at_the_local_s_size`.

`every_field_of_the_memory_checks_value_is_a_belief_or_an_exemption` fails
when an exempt row starts believed, names a test that does not exist, or
names none without one of those two reasons.

### Consequences

* Good, because the tables say which starts rest on an assumption, two of
  them, and which are exact, and an update a door drops fails a named test.
* Bad, because a field's guard is a name the test can only check exists; that
  it fails under the mutation is measured once, here, and not again.

## More Information

#1029 (the issue), #941 (the parent), #1030 (the table in the safety model).
See ADR-0091 rule 3 and ADR-0101.
