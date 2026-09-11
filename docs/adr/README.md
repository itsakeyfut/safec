# Architecture Decision Records

A design decision's rationale lives here and nowhere else. The documents in
[`docs/`](../README.md) say what the project wants to build; the doc comments in
the code say what each type is and how to use it. Neither repeats the reasoning:
they link here.

These records are written in **English**, like the code they constrain, because
they are read by contributors and tooling alike.

Format: [MADR 4.0](https://adr.github.io/madr/), the de facto Markdown ADR
standard. Copy [`adr-template.md`](./adr-template.md) to start one.

## Index

| # | Decision | Status | Confirmed by |
|---|---|---|---|
| [0001](./0001-promote-unproven-results-in-the-sink.md) | Tag a result the analysis could not prove at the check, and promote it in the sink | accepted | unit tests in `crates/safec/src/diagnostics.rs` (promotion on, promotion off, a proven warning left alone, `error_count` equals a recount) the `--safety strict` resolution test in `crates/safec/src/cli.rs`, and `the_sink_is_built_from_the_resolved_options` in `crates/safec/src/driver.rs`; each verified by the mutation that makes it fail |
| [0002](./0002-severity-runs-from-least-to-most-severe.md) | Declare `Severity` from least to most severe, the same direction as `SafetyLevel` | accepted | `severities_are_ordered_from_least_to_most_serious` and `the_worst_severity_in_a_set_is_its_maximum` in `crates/safec/src/diagnostics.rs`; both fail if the variants are put back in descending order |
| [0003](./0003-pass-the-source-map-to-each-render-call.md) | Pass the source map to each render call rather than holding it | accepted | `a_renderer_does_not_hold_the_source_map` in `crates/safec/src/diagnostics/render.rs`; the borrow half does not compile against a renderer that holds the map, and sizing the cache once makes the assertion fail |
| [0004](./0004-resolve-the-strictest-level-where-the-policy-is-built.md) | Resolve the strictest safety level where the policy is built, not only where the options are | accepted | `Policy`'s private field makes the bypass a compile error (E0451); `the_strictest_safety_level_denies_unknown_however_the_policy_is_built` and `the_strictest_safety_level_denies_unknown_even_unresolved` in `crates/safec/src/diagnostics.rs` and `the_strictest_safety_level_denies_unknown_in_the_sink` in `crates/safec/src/driver.rs`, all three of which fail if `Policy::new` keeps `deny_unknown` alone |
| [0005](./0005-hold-each-source-file-behind-an-arc.md) | Hold each source file behind an `Arc` so one can be read while another is added | accepted | `a_file_can_be_read_while_another_is_added` in `crates/safec-ir/src/source.rs`; writing `file` where it writes `file_owned` is the reversal and fails to compile with E0502 |
| [0006](./0006-carry-a-position-rather-than-the-text.md) | Carry a position rather than the text it covers | accepted | `const _: () = assert!(mem::size_of::<Token>() == 16);` in `crates/safec/src/token.rs`; a `String` field takes it to 40 and a `Symbol` field to 20, and the crate stops building. The trigger for an interner is a convention this record states and nothing enforces |

| [0007](./0007-list-the-test-cases-rather-than-discovering-them.md) | List the test cases in a literal table rather than discovering them on disk | accepted | `every_file_in_the_corpus_belongs_to_a_case_in_the_table` in `crates/safec/tests/cases.rs`; putting anything in `cases/` that the table does not name, at the top level or in a subdirectory, makes it fail and nothing else |

| [0008](./0008-hold-the-tree-in-flat-vectors-and-address-a-node-by-index.md) | Hold the tree in flat vectors and address a node by index | accepted | `an_id_still_names_its_node_after_more_are_pushed` in `crates/safec/src/ast.rs`; taking the id from `len()` after the push rather than before makes it fail, and `Ast`'s private vectors make holding a node across a push `error[E0502]` |

| [0009](./0009-name-a-diagnostic-code-after-the-compiler-and-the-topic.md) | Name a diagnostic code after this compiler and the topic, not after the severity | accepted | `Code::new` in `crates/safec/src/diagnostics.rs` asserts the spelling, so `Code::new("E0301")` is `error[E0080]` at the declaration and `a_code_is_sc_and_four_digits_or_it_is_not_a_code` fails if either half of the assertion goes. Which code goes where is held by `each_lexical_diagnostic_keeps_the_code_it_was_assigned`, `each_syntax_diagnostic_keeps_the_code_it_was_assigned` and thirteen `.stderr` files. Which range a topic takes is enforced by nobody but a reviewer, and the record says so |

| [0010](./0010-give-the-graph-an-edge-no-statement-produced.md) | Give the control-flow graph an edge no statement produced | accepted | `E0004`: `Terminator::successors` in `crates/safec-ir/src/ir.rs` matches every kind written out, so a kind added later stops every walk from compiling, and `an_edge_no_statement_produced_can_be_built` stops compiling if the variant goes |

| [0011](./0011-the-ir-crate-depends-on-nothing-in-the-workspace.md) | The IR crate depends on nothing in the workspace, and the frontend depends on it | accepted | cargo, before anything compiles: adding `safec = { path = "../safec" }` to `crates/safec-ir/Cargo.toml` is `error: cyclic package dependency`, and a `use safec::` without it does not resolve either, as `E0432` or `E0433` depending on how far the path gets. The hand-built interpreter tests in `crates/safec-ir/src/interp.rs` are the other half: `a_function_built_by_hand_runs` and five others run the IR with no frontend in the graph |

| [0012](./0012-say-a-local-s-storage-began-and-ended-in-the-block.md) | Say a local's storage began and ended as elements of a block, not as a tree beside it | accepted | `E0004`: a fourth `Element` kind stops `ir.rs`, `interp.rs` and `print.rs` compiling at once, and a field added to one is `E0027` in each walk and `E0063` where the lowering builds it. `the_same_program_in_a_nested_scope_is_not_the_same_ir` and `a_read_through_a_pointer_to_dead_storage_stops_the_run` are the same claim by assertion and by running |

| [0013](./0013-the-translation-unit-carries-the-target-and-answers-what-a-type-is-worth.md) | The translation unit carries the target, and the IR answers what a type is worth | accepted | `TranslationUnit::new` takes a target and has no `Default`, and `Target`'s private fields make the only targets the measured ones. `every_target_is_what_clang_says_it_is` writes the measurements out rather than walking the table, and `what_a_char_holds_follows_the_target` is the difference between two machines: `c = 200` answers -56 and 200 |

**By status**: accepted: 0001, 0002, 0003, 0004, 0005, 0006, 0007, 0008, 0009, 0010, 0011, 0012, 0013 · proposed: none · superseded: none

Records are numbered consecutively from `0001`.

## Where each kind of writing belongs

| Location | Holds | Does not hold |
|---|---|---|
| [`docs/*.md`](../README.md) | what the project wants to build, roughly: the concept, the safety model, the architecture, the phases | how a decision was reached. These are intent documents, not specifications, and a design may knowingly diverge from one. Say so in the record and update the document |
| doc comments in the code | what a type is, how to use it, and the local reason it is shaped that way | cross-cutting rationale; link here instead |
| `docs/adr/**` | why a cross-cutting decision was chosen, when, and what would reverse it | type or signature detail; link to the code |

## When to write one

A record is a cache of reasoning that the code cannot show. The question that
decides whether to open one is whether someone will later undo this in good
faith: reading the code, seeing something that looks better, and having no way
to find out what it costs.

* Two or more implementations are possible and one is chosen, especially when the
  choice shapes an interface that later phases will be written against.
* A choice affects whether the compiler can be *trusted*: anything that decides
  when a result is reported, suppressed, or promoted. Getting one of these wrong
  makes `safec` claim code is safe that was never proven, which is the worst
  failure this project has.
* The code was knowingly left in a shape that looks wrong: a duplication kept on
  purpose, a slower path chosen for a reason, a guard that appears redundant.
  Without a record these are cleaned up by the next person to read them.
* An existing decision is reversed: write a new record, mark the old one
  `superseded by ADR-NNNN`, and note what changed.
* A design knowingly diverges from a document in `docs/`: record why, and update
  that document in the same change.
* You are about to write "undecided" into a doc comment: open one as `proposed`.

## When not to write one

A reason has cheaper places to live, and each of these is a complete answer on
its own:

1. Nothing. The decision is local and the code already shows it.
2. A comment where it happens.
3. A doc comment on the type or function it constrains.
4. A test whose name states the rule.
5. A record here.

Take the lowest one that holds. This repository leans hard on 4: a test named
for the rule it protects is how most decisions here are written down, and the
mutation that makes it fail is the proof it is a real rule. Open a record only
when the reasoning does not fit into a name and an assertion, which usually
means the rejected alternatives are the part worth keeping.

Specifically, do not open one when:

* It affects one call site, or it is formatting.
* Reversing it would be cheap and local. The record earns its keep by making a
  reversal informed, so a reversal that costs an afternoon does not need one.
* A test name already says it, and someone who broke the rule would read that
  name and understand what they broke.
* The choice was made for us, by the C standard, by a dependency's interface, or
  by the platform. Writing that down records a fact rather than a decision.
* It is a performance choice a benchmark can settle. The benchmark is the
  record, and it stays true when the numbers change.

Naming is usually below the line too, but it rises above it when the name shapes
an API that the rest of the crate compares against or matches on, because those
are the names that go silently wrong.

**Expected rate.** Phase 0 is dense because it is almost entirely interface, and
that is the phase where records are worth the most. It does not continue. Most
of what a lexer and a parser decide is decided by the C standard, and most of
what an analysis decides is settled by measurement. A few records per phase is
the shape to expect. Weeks with none are the normal case, and several in a week
is a sign the bar has slipped rather than a sign of progress.

## Conventions

* Filename `NNNN-short-slug.md`, numbers consecutive.
* The four digits are there so that sorting the filenames as text puts them in
  the order they were written. They are not a limit. A number is never reused,
  never renumbered, and stays with a record that is superseded, because it is
  what every link, doc comment and commit message refers to. Widening the
  padding later would break all of those at once, so a log that ever reaches
  `9999` carries on into five digits and fixes its ordering in the index above.
  A log that large is a sign the bar in *When not to write one* was too low.
* MADR statuses: `proposed`, `accepted`, `rejected`, `deprecated`,
  `superseded by ADR-NNNN`.
* Every record fills in **Confirmation**: which test or guard fails if the
  decision is violated, **and the change to the code that makes it fail**. This
  repository verifies guards by breaking the code and watching the suite go red,
  so a record should name that mutation. If nothing would fail, say so.
* A `proposed` status while the codebase already relies on the decision is itself
  a defect; say so in *Context and Problem Statement*.
* Keep the status in sync between an ADR's front matter and its row in this index.
* When a record says something is not yet enforced, and it then is, update
  **Confirmation** in the same change. That section answers what guards the
  decision now, not what was true when it was written, and a record that
  understates its own guards sends the next reader to redo finished work. It is
  the same obligation as the pointers below, in the section that matters most.
* When code a record names moves or is renamed, update the record's pointers to
  it. The reasoning is a record of what was thought at the time and is not
  rewritten, but the pointers are navigation rather than history, and a record
  whose pointers are dead is a record nobody can check.
* Prose here uses no em dashes.

## More Information

* [MADR 4.0](https://adr.github.io/madr/): the template this follows.
* [`adr-template.md`](./adr-template.md): copy this to start a new record.
