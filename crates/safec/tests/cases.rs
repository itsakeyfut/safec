//! C programs, and the output the compiler is expected to produce for them.
//!
//! A case is a `.c` file under `cases/` and up to three expected-output files
//! beside it. Adding one is those files and a line in the table below; no Rust
//! is written for it, which is the point. There were two fixtures here for as
//! long as adding a third meant writing a test.
//!
//! What is being pinned is an interface. A caller redirects `--emit` output,
//! greps it and diffs it, so a `contains` check is not an assertion about it:
//! it goes on passing while the shape somebody depends on changes underneath.
//!
//! `ariadne` ends a caret line with spaces, so an expected file does too.
//! Trailing whitespace in one is content rather than dirt, and an editor or a
//! hook that strips it breaks a case for a reason with nothing to do with the
//! compiler.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One `#[test]` per case, and the roster the unlisted-file guard reads, from
/// one literal table.
///
/// Both come out of the same entries on purpose. A roster maintained separately
/// from the tests is a second copy to drift.
macro_rules! cases {
    ($($name:ident : [$($arg:literal),* $(,)?]),* $(,)?) => {
        /// Every case the table names.
        const CASES: &[&str] = &[$(stringify!($name)),*];

        $(
            #[test]
            fn $name() {
                run_case(stringify!($name), &[$($arg),*]);
            }
        )*
    };
}

// The table is written out rather than discovered by walking `cases/`.
// See ADR-0007 for why, and for what it rejected.
cases! {
    // The memory check's cases, kept together because what each one is for is
    // a row of one table: `docs/safety-model.md`'s three-valued model met by a
    // program that proves it, one that leaves it unproven, and one that does
    // neither.
    a_value_freed_twice: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_value_freed_through_a_copy: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_parameter_freed_twice: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same function with its only caller in view, passing null. See
    // ADR-0027: a parameter ranges over what any caller may pass.
    a_double_free_in_a_function_the_only_caller_passes_null_to_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_branch_that_allocates_either_way: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_value_used_after_it_was_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same free and the same use, in one full expression with nothing
    // ordering them. C17 6.5 p3 leaves the operands of `+` unsequenced, so one
    // allowed order reads `*p` first and the program is defined; which order an
    // implementation picks is unspecified and this compiler does not get to
    // choose. The pair is written both ways round because the answer used to
    // turn on which side the free was written, and what decided was that
    // ADR-0010 makes a call end a block. See ADR-0022.
    //
    // **The third is the same program with the use read first**, which the
    // walk meets before it meets the free. A forward walk only looks back, so
    // that half is answered by carrying the read forwards to the free instead:
    // see ADR-0023. `*p` on its own is not read until the addition is built,
    // which is after the call, so the pair above are answered by the free
    // marking what follows it; put the use inside a call and it is read first
    // and the read is what waits.
    an_unsequenced_free_and_use_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    the_same_program_with_the_operands_swapped_is_not_proved_either: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_unsequenced_use_the_check_meets_first_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same, for the two reads an *element* carries rather than a call or a
    // branch: writing through the pointer, and evaluating a place for no reason
    // but the evaluation. Neither is reached by any case above, whose reads are
    // all carried by a terminator, so without these the element half of the
    // recording can be deleted with the suite green. Measured, which is how
    // they came to be here.
    a_write_through_a_pointer_the_check_meets_first_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_discarded_read_the_check_meets_first_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the two edges of that: a read of something this free is not about,
    // and a read that cannot have happened on the path the free is on. The
    // first says the sites are compared rather than the spans, and the second
    // says the reads are carried along the graph's edges rather than along the
    // order the blocks were written in.
    a_use_of_another_pointer_before_a_free_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_use_and_a_free_on_two_arms_of_one_conditional_are_not_both_reached: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the read that has to cross a merge to reach the free at all: it is
    // inside one arm of a `?:` that the free is outside of, so nothing but the
    // join carries it. The case above puts the two on opposite arms and asks
    // for silence; this one puts the read on an arm and the free after the
    // merge and asks for a report, which is the only shape where losing the
    // join's union of the carried reads is a silence rather than a noise.
    a_read_inside_one_arm_before_a_free_survives_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And what the report is allowed to *name*. The read may have gone through
    // either allocation, so `allocated here` would be a caret on one of two
    // lines with nothing to choose between them, which is RK-035's may-set
    // mistake made about a label rather than about a proof. Its `.stderr` has
    // no such caret, and folding the two with anything but `same` puts one
    // back.
    a_read_of_either_of_two_allocations_before_a_free_names_neither: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the read the free's *own* argument evaluation performed, which C17
    // 6.5.2.2 p10's first sentence orders before the call unconditionally. The
    // carried read has to stop at that point like it stops at any other, so
    // neither of these gets an `SC0402`; the `SC0403` each still carries is the
    // nullability check answering a different question, and the `SC0404` is
    // ADR-0036's, because an offset read through a pointer is not one this
    // check can evaluate. The second reaches the
    // same place through a nested call, which is the spelling where the read is
    // further from the free than an operand of it.
    //
    // The first writes through `p` before the free so that the program is one
    // C defines: reading an allocation nobody wrote to is 7.22.3.4 p2's
    // indeterminate value and the offset would leave the object, and a guard
    // is worth more when what it guards is defined. The second cannot be
    // repaired that way, because `g`'s result is not this check's to know, and
    // it is here for its shape.
    //
    // `Element::ArgumentsEvaluated` is what they are about, and ADR-0026 is
    // why it says less than `Element::Sequenced`.
    //
    // Mutation: the arm in `memory.rs` that reads that element doing nothing.
    // These two fail on their `.stderr` with the `SC0402` back, and nothing
    // else fails. The rule's other half is the lowering that emits it, and
    // mutating that fails these two on their `.stdout` along with every other
    // artifact holding a call, so the halves are mutated apart. RK-039.
    a_read_in_a_frees_own_argument_is_ordered_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_read_in_a_frees_argument_through_a_call_is_ordered_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same question asked of a call this check cannot read, which may have
    // freed what it was handed and cannot say. The pair is the asymmetry
    // itself: the first reads through `p` in the operand beside the call, where
    // C17 6.5 p3 orders neither against the other, and the second puts a
    // sequence point between them and has to stay silent. Without the first,
    // `used_before` can go back to refusing every callee but `free` with the
    // suite green.
    //
    // Its `.stderr` is where the words are pinned: `allocated here` and `used
    // here, perhaps after the free`, and **no** `freed here` caret and no
    // 6.5.2.2 p10 note, because nothing established a free. That is the whole
    // of what `Unproven::Disagreement` buys over `Unproven::Unsequenced` here.
    //
    // Both test `p` before reading it so that what they pin is this check
    // rather than the nullability one, whose `SC0403` would otherwise be in
    // both files and would make the second case's silence a sentence rather
    // than an empty file. They write through `p` first for the reason the pair
    // above give.
    //
    // **The second's call is written `h(p) + 1`, and the `+ 1` is the whole of
    // why it guards anything.** Spelled `h(p)`, the call is enclosed by nothing
    // C leaves unsequenced, so ADR-0026's marker is emitted at its own
    // arguments and empties the carried reads a second time; measured, the case
    // then survived the removal of *either* clearing and named neither. RK-039
    // is a mutation that nothing fails because two rules hold it. Under the
    // `+`, the marker is suppressed and the sequence point at the end of the
    // statement before is the only thing left holding the silence.
    //
    // Mutations, each applied alone and the failure read:
    //
    // * refuse `Callee::Opaque` in `used_before` again. The first fails on its
    //   `.stderr`, which goes empty.
    // * give the opaque case `Unproven::Unsequenced` and the call's span as
    //   `freed`. The first fails on its `.stderr`, which gains a `freed here`
    //   caret and the 6.5.2.2 p10 note, about a free no program here performs.
    // * the `Element::Sequenced` arm stops clearing the carried reads. Both
    //   fail: the second gains an `SC0402` about `g(*p)` a statement earlier,
    //   and the first gains a second one about `*p = 1`.
    an_unsequenced_use_before_an_opaque_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_use_an_opaque_call_is_sequenced_after_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A controlling expression that is exactly a dereference is read by the
    // branch itself, because it needs no temporary, so the sequence point at
    // the end of it belongs after the branch and not before. C17 6.8 p4, and
    // both arms: the body and the edge that skips it are each after the
    // condition. The third has nothing ordering it, because a `?:`
    // below a `+` is enclosed by something C leaves unsequenced, so that one
    // reports.
    a_condition_read_through_a_pointer_is_sequenced_before_the_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_condition_read_through_a_pointer_is_sequenced_before_the_other_arm: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_condition_read_through_a_pointer_is_sequenced_before_the_loop_exits: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_condition_read_through_a_pointer_in_an_unsequenced_operand_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Three shapes where asking for a sequence point would be asking the
    // wrong question, each of which review found this check getting wrong.
    // A double free runs both frees whichever order C picks, so 6.5 p3
    // settles nothing about it and the proof stands. A free already ordered
    // before an expression proves a use inside that expression, and stays the
    // one to name, however the frees inside it are ordered. And where several
    // frees are folded into one answer, the earliest span and the conjunction
    // of their orders can come from different frees, so the caret that would
    // say `freed here` is dropped rather than paired with a note about a free
    // it is not pointing at.
    a_double_free_in_one_expression_does_not_turn_on_the_order: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_already_sequenced_is_the_one_a_later_free_keeps: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_may_set_where_one_free_is_sequenced_and_one_is_not: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A sequencing operator below something C leaves unsequenced orders its
    // own parts and nothing outside them, so none of these three is a proof.
    // One case per operator, because the guard is written once per operator
    // and review measured that removing any one of them leaves the suite
    // green: the comma's was held, and `&&`, `||`, `?:` and a call's
    // arguments were not. Each mutation makes the compiler **certain** about
    // an order C has not chosen, which is the direction that matters.
    a_comma_inside_a_call_argument_orders_nothing_outside_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_logical_and_inside_an_unsequenced_operand_orders_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_conditional_inside_an_unsequenced_operand_orders_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The four constructs that do order their operands, one case each, because
    // C17 Annex C names four and a list implemented three-quarters of the way
    // leaves a reader asking which quarter. 6.5.17 p2, 6.5.13 p4, 6.5.14 p4 and
    // 6.5.15 p4 in that order, and each is a proof rather than a suspicion.
    a_comma_sequences_a_free_before_a_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_logical_and_sequences_a_free_before_a_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_logical_or_sequences_a_free_before_a_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_conditional_sequences_a_free_before_a_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_value_read_after_it_was_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_use_after_a_free_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_dereference_of_a_pointer_with_no_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_call_that_is_not_known_to_allocate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The other half of reading the callee's name: a call that allocates is
    // also a call that does not touch what it was handed, and the case above
    // says nothing about that.
    //
    // **`malloc` is declared taking a pointer, and that is the case rather
    // than an accident of it.** This check recognises an allocation by the
    // name and never by the type, so the declaration is free to take whatever
    // reaches a site, and in this subset only a pointer does. Written with
    // C's own prototype the call does not type-check: `clang -std=c17
    // -pedantic-errors` reports `incompatible pointer to integer conversion`
    // for `malloc(q)` against `int`. It accepts this one with
    // `-Wincompatible-library-redeclaration`, which every case here declaring
    // `malloc` already draws.
    //
    // Mutation: have `Callee::Allocates` poison its arguments as
    // `Callee::Opaque` does. The site `q` holds becomes `Unknown` at the
    // middle call and this fails on its `.stderr`, which gains a
    // `warning[SC0401]` on the first free.
    an_allocating_call_leaves_its_argument_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_subscript_of_a_freed_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_constant_subscript_of_a_freed_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same program with `&p` in it, which is the pair that says the fold
    // aligned the two spellings rather than only changing one. The subscript
    // used to be proved here and the dereference never was: taking `p`'s
    // address makes it unprovable under ADR-0017, and the subscript reached a
    // proof only by arriving as a shape that rule did not see. Aligning them
    // costs a proof, and ADR-0021 says why that is the right direction.
    a_subscript_of_an_escaped_pointer_is_suspected_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    two_pointers_used_after_a_free_on_one_line: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    one_pointer_used_after_a_free_on_two_lines: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_freed_pointer_read_in_an_argument: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_either_of_two_locals_names_no_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_one_of_two_allocations_by_name: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_a_may_set_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Silent, and for two reasons since ADR-0027: the local forgot the set it
    // freed, and `p = 0; free(p);` is a call C defines as doing nothing. Which
    // of the two is working can no longer be read off this case, so the
    // forgetting is asked of the lattice directly by
    // `a_local_given_a_constant_forgets_the_set_it_freed` in
    // `crates/safec-ir/tests/freed.rs`, where the assignment is a constant
    // this subset cannot write.
    a_local_given_nothing_forgets_the_set_it_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_may_set_freed_then_written_through_an_alias: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // What a proof about a may-set survives when a value is built from its
    // operands, which is the half of `Held` that is not a may-fact. It carries
    // while the set does not grow, and an offset by an integer does not grow
    // it: C17 6.5.6 p8 keeps the result inside the object the *pointer* operand
    // points into, whatever the index happens to be. `i` is a parameter and so
    // a site, and the result reaches it no longer. See ADR-0024 for the proof's
    // rule and ADR-0030 for which operands it counts.
    a_free_after_an_offset_that_kept_the_set_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_offset_by_an_integer_parameter_keeps_the_proof: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same offset by an integer that is not a site at all, which is the
    // other half of that: the two cases differ in whether there was anything to
    // pick up, and neither picks it up. A set that *has* grown loses the proof
    // still, and no C this frontend accepts writes one, so
    // `an_offset_by_a_second_pointer_loses_the_proof` holds that from
    // hand-built IR instead.
    an_offset_by_an_integer_local_keeps_the_proof: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the may-fact beside the proof. A local that lost the name for what it
    // held says so, and arithmetic on it builds a pointer that has lost it too.
    // ADR-0018 holds that for a copy and nothing held it for arithmetic, so the
    // union the accumulator does of that bit could be deleted with the suite
    // green.
    a_pointer_built_by_arithmetic_from_a_local_that_lost_its_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_dereference_after_a_may_set_was_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_dereference_in_a_condition_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_dereference_in_a_while_condition_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_dereference_in_a_for_condition_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_loop_whose_condition_reads_what_its_body_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_loop_that_allocates_and_frees_each_turn_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_saved_across_a_loop_that_allocates_again: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_copy_of_a_local_that_lost_its_allocation_lost_it_too: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_local_that_kept_one_allocation_and_lost_another: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_double_free_across_a_loop_names_the_free_that_is_wrong: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_the_previous_turns_pointer_leaves_the_new_one_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_local_given_something_fresh_forgets_what_it_lost: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `free(p); p = 0;`, the commonest hygiene C has, and silent for the two
    // reasons the case above is. `a_local_given_a_constant_forgets_the_site_it_held`
    // is where the lattice half is asked on its own.
    a_pointer_set_to_nothing_after_a_free_holds_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_live_read_and_a_freed_one_at_one_span: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_unproven_read_and_a_freed_one_at_one_span: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_escaped_local_read_twice_at_one_span: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_freed_read_and_an_unproven_one_at_one_span: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_discarded_dereference_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_discarded_subscript_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_discarded_dereference_in_a_comma_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_comma_whose_right_operand_replaces_the_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_comma_that_frees_after_it_reads: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_discarded_dereference_in_a_for_initialiser_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_discarded_dereference_in_a_for_step_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_discarded_name_gets_no_element: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_dereference_through_an_address_of_a_dereference: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_address_of_a_dereference_aliases_what_it_came_from: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_address_of_a_subscript_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_address_of_a_double_dereference: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_dereference_after_a_comma_in_a_condition: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_subscript_in_a_condition_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_dereference_in_a_conditional_expression_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    two_allocations_are_freed_once_each: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_allocation_replaced_before_it_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_call_this_check_cannot_read_between_two_frees: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_through_a_pointer_the_check_does_not_follow: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Two programs with one `free` each, and the boundary between two reasons
    // a report can be unproven. In the first nothing established a free at
    // all, so the diagnostic says what this check lost rather than that the
    // value may have been freed already: `*pp` is a pointer this check follows
    // locals rather than the targets of. The second reaches the same caret
    // with a site an opaque call was handed, where `helper` may really have
    // freed it, and it keeps the older words. It is the only case whose
    // suspicion rests on nothing but the callee: every other program that
    // keeps those words has a `free` in it that this check saw. Answering
    // `Unproven::Lost` where the sites disagree fails it, along with
    // everything else that keeps them.
    //
    // **There were three, and the third has moved down beside the null
    // constant.** `int *p = 0; free(p);` was here to hold the words a lost
    // pointer gets, on the grounds that a local given a constant holds no
    // site. ADR-0027's exemption now answers that program before the words are
    // reached, so it says nothing at all and belongs with the other spelling
    // of it.
    a_free_read_out_of_another_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_after_a_call_this_check_cannot_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the one this check is meant to say nothing about at all. C17
    // 7.22.3.3 p2: "If `ptr` is a null pointer, no action occurs." So a null
    // constant handed to `free` is written on purpose and is not a pointer
    // this check lost. Nothing held that until this case: making a constant
    // argument answer `Reached::Lost` puts a warning on a program C defines,
    // and fails this case and nothing else in the suite.
    //
    // It pins the null half only. `Allocations::touching` skips every
    // constant, and the clause above supports it for this one, so `free(17)`
    // stays silent and this case does not say otherwise. The comment on that
    // arm says where the other half belongs.
    a_free_of_a_null_constant: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same program with the constant given a name first, which the clause
    // does not distinguish and `clang` accepts identically. It is silent
    // because the nullability check established the pointer is null and
    // `asked` leaves such an argument out, which is ADR-0027; dropping that
    // filter puts `this frees a pointer this check stopped following` back on
    // a program C defines and fails this case.
    a_free_of_a_pointer_proved_null: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The two halves of what that exemption is allowed to rest on, one case
    // each, because each is a way of getting the nullness right in form and
    // wrong in fact, and being wrong there is a double free reported by
    // nobody.
    //
    // The first is read through the escape mask. `p` is written through its
    // own address, so the lattice still calls it null while the program has
    // given it an allocation; reading the value rather than
    // `Nullability::known` exempts both frees and this case goes silent.
    //
    // The second is asked where the free runs. `p` is established null at the
    // entry of the block that frees it and is not null by the time the free is
    // reached, so recording the row before the block's elements rather than
    // after exempts a proved double free of `q`'s site. Measured: each
    // mutation takes its own case to exit 0 and leaves the other reporting.
    // And the argument the exemption is not allowed to read at all. `pp` is
    // established null and `*pp` is a question about what it points at, which
    // the nullability lattice is keyed by the local and cannot ask; answering
    // it with `pp`'s own nullness takes the `SC0401` off this case and leaves
    // the `SC0403` alone. Measured: dropping the `projection.is_empty()` guard
    // in `established_null` fails this case and nothing else in the suite.
    // And the local the exemption may not call null at all, because it does
    // not hold a pointer. `nullness_of` answers `Null` for a constant zero
    // without asking what it is assigned to, which costs nothing where the
    // answer is only read about a dereference; read to exempt a free, it took
    // the `SC0401` off this program and left nothing in its place. C17 6.3.2.3
    // p3 makes a null pointer constant an integer constant expression
    // converted to a pointer type, and an `int` lvalue holding zero is
    // neither. `clang` refuses this program under 6.5.2.2 p2, which this
    // compiler does not do yet and #154 is about; until it does, what it
    // should not do is go quiet.
    // And the free C has not ordered the assignment before. The operands of
    // `+` are unsequenced, C17 6.5 p3, so on the order that runs the right one
    // first this frees the pointer the line above already freed. The
    // nullability lattice has no notion of order and says so; the marker that
    // does is `Element::ArgumentsEvaluated`, which ADR-0026 emits only where no
    // unsequenced operator encloses the call, and its absence here is what
    // refuses the exemption. Dropping that term reports nothing at all about a
    // double free this check watched, which is the bottom row of `CLAUDE.md`'s
    // list, and fails this case.
    a_free_in_an_unsequenced_operand_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same defect where the block holding the free has no elements at all.
    // A call ends a block, so the `g(0)` between the two operands puts the
    // write in one block and the free at the terminator of the next, and that
    // block carries neither a marker nor anything else. The rule reads
    // `elements.last()`, so this is the `None` arm, and it is the only case
    // that reaches it: treating `None` as ordered leaves this program silent
    // about the double free and fails nothing else.
    a_free_in_an_unsequenced_operand_across_a_call_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_an_int_that_holds_zero_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_read_out_of_a_pointer_proved_null: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_proved_null_before_its_address_escaped_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_that_stopped_being_null_before_the_free_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A free of a pointer that is not the start of its allocation, which C17
    // 7.22.3.3 p2 makes undefined and which this check once followed to the
    // allocation and said nothing about. See ADR-0036. The first three are the
    // proof: a constant offset either way, and the increment the issue that
    // found this was written about. The fourth is the offset this check cannot
    // evaluate, which is unproven and so an error in this compilation.
    a_free_of_a_pointer_past_the_start_of_an_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_a_pointer_an_increment_moved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_a_pointer_before_the_start_of_an_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_a_pointer_offset_by_an_integer_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The join, both ways. Two paths that each moved the pointer off the start
    // are both off it, whatever the distances; one that did not leaves the
    // answer open.
    // `+` with the constant on the left, which C17 6.5.6 p8 makes the same
    // addition. Mutation: in `memory.rs::offset_of`, drop the arm that reads
    // the constant on the left. This fails with the proof down to `may`.
    a_free_of_a_constant_plus_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A pointer moved off the start and back is at the start again, and this
    // check carries no distance to know it, so the second move proves nothing.
    // The program is one C defines. Mutation: in `offset_of`, stop requiring
    // the followed operand to be at the start. This fails with a proved
    // `SC0404` about a free C defines.
    a_free_of_a_pointer_moved_back_to_the_start_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Two allocations, one offset, and no `allocated here`: naming either line
    // would be a caret on an allocation the value may not hold, RK-035. Both
    // are made before the branch rather than on its arms: allocated on an arm,
    // each site meets the other arm's `Live(None)` at the join and arrives with
    // no line to name, so the fold has nothing to get wrong. Mutation: in `memory.rs::interior`, fold `made` by keeping the first
    // site's, or by keeping the last site's. Each fails on the label, and both
    // directions are measured because RK-038 is a fold guarded on one side.
    a_free_past_the_start_of_either_of_two_allocations_names_neither: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_offset_on_both_arms_is_still_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_offset_on_one_arm_only_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The two that must stay silent. The first is ADR-0021's fold reaching this
    // check as no arithmetic at all, and guards that record rather than anything
    // here. The second is what a local holding no site answers, on one of the
    // commonest shapes in C: a path that holds null meeting one that allocated.
    a_free_of_a_pointer_plus_zero_is_a_free_of_the_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_of_an_allocation_made_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_whose_address_escaped: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_replaced_through_its_own_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_replaced_through_its_alias_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_allocation_given_to_an_escaped_local_after_the_escape: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_allocation_shared_with_a_local_whose_address_escaped: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_through_an_escaped_local_is_seen_by_a_sharer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_written_through_an_alias_this_check_follows: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_through_a_pointer_that_reached_another_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_through_a_pointer_that_may_land_elsewhere_keeps_what_was_there: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_through_an_address_plus_one_is_not_a_write_to_the_local: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_subscript_write_is_the_write_it_is_defined_as: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_zero_added_to_an_integer_keeps_its_operation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_through_a_pointer_plus_zero_on_the_left: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_through_a_pointer_minus_zero: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_address_taken_on_one_arm_is_written_through_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same shape as the case above, on a program C defines on both paths:
    // the other arm gives `pp` a pointer this check cannot follow rather than
    // a null one it would be undefined to write through. A guard for a
    // soundness rule should not rest on a program C has already given up on.
    a_write_through_a_pointer_with_one_target_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A write through a pointer whose own address escaped may land anywhere,
    // whichever order the two happen in. The second case is the one that says
    // the answer cannot be recorded on the pointer's own row: `pp = &p` after
    // the escape gives `pp` a fresh row, and a fact written there would have
    // gone with the old one. See ADR-0028.
    a_write_through_a_pointer_whose_own_address_escaped: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_through_a_pointer_whose_address_escaped_before_it_was_given_its_target: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A call this check cannot read may write a fresh pointer through any
    // address that has escaped, so a `free` afterwards cannot say which
    // allocation it took. Both of these are programs C defines and both were
    // an `error` at exit 1, reported against a **sharer**: what the escape
    // already took away is the report about the escaped local itself, so a
    // case that frees or reads through that local alone observes nothing.
    // RK-048 is that hole and ADR-0029 is the rule.
    a_call_this_check_cannot_read_may_have_replaced_what_an_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_certain_write_does_not_survive_a_call_this_check_cannot_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // What that rule costs, kept where it can be seen: a double free through a
    // sharer that `main` proved before the call was allowed to replace what
    // `p` holds. It is a warning here because the callee may have.
    a_double_free_through_a_sharer_after_an_opaque_call_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A free cannot un-free an allocation, so a free this check could not
    // follow has nothing to say about a site an earlier free it *could* follow
    // already proved. Writing `Unknown` over that site anyway threw the proof
    // away, and the site is shared, so what lost it was the sharer's report.
    // Found by review.
    a_free_this_check_could_not_follow_leaves_a_proved_free_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The other half of the trade the rule makes. The downgrade is only
    // acceptable because the flag still fails the build, and this is the case
    // that says so: same program as the sharer case above, one flag on.
    a_double_free_a_call_took_the_proof_of_still_fails_a_build_that_denies_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the direction the cheap versions of that rule fail in. Clearing the
    // escaped local's row at the call instead leaves this program with nothing
    // to say about the read, which is the bottom of `CLAUDE.md`'s list while a
    // false positive is row 4. See ADR-0029.
    a_use_after_free_through_an_escaped_local_is_still_reported_after_an_opaque_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the other order, which keeps the rule from being the whole of what
    // an escape means: a call that ran **before** the address escaped cannot
    // have written through it, so the proof survives. RK-065 is a guard that
    // held only the order it was written in.
    an_opaque_call_before_the_escape_leaves_the_proof_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same rule through the door ADR-0029 named and declined: the writer
    // is a write in this function rather than a callee. Both of these were an
    // `error` at exit 1 about a program C defines, and both report through a
    // sharer for RK-048's reason. The second is here although one mutation
    // fails both, because the two are the ways a pointer gets out of this
    // check's sight: through a parameter, and through a chain of locals with
    // no call and no parameter in it. A fix keyed on either shape alone passes
    // the other. See ADR-0031.
    a_write_through_a_pointer_this_check_cannot_follow_may_have_replaced_what_an_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_that_replaces_what_an_escaped_local_holds_needs_no_call_and_no_parameter: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The other half of the trade, as ADR-0029's own flag case says it for the
    // call: the downgrade is only acceptable because the flag still fails the
    // build. Same program as the first case above, one flag on.
    a_use_after_free_a_write_took_the_proof_of_still_fails_a_build_that_denies_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // What keeps that rule from costing everything, and the only case that
    // holds it: a write through a pointer to an `int` cannot put a pointer
    // anywhere, so the escaped local keeps what it held and the double free
    // below stays proved. Marking every escaped local instead drops this
    // `error[SC0401]` to a warning and exit 1 to exit 0.
    a_write_through_a_pointer_to_an_int_leaves_what_an_escaped_local_holds_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The half of the door that a write with no target at all does not reach:
    // one named target, and a path on which the pointer was never given one,
    // so the write may land beside that target in an escaped local the union
    // says nothing about. Narrowing the rule to an empty target set leaves
    // this program an `error` at exit 1.
    a_write_that_may_land_beside_its_target_may_have_replaced_what_an_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The first case above with the alias written inline instead of read into
    // a temporary, which is the same program and a place with two `Deref`s.
    // This check follows a write through exactly one, so the rule has to fire
    // for every projection it declines rather than for the shape it can
    // follow: keyed on that shape, the two spellings of one program answered
    // opposite ways. Two review lenses found it independently.
    a_write_through_more_than_one_deref_may_have_replaced_what_an_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The exception C17 6.5 p7 carries, and the one write that reaches an
    // escaped local of every type: a character lvalue may access an object of
    // any type, so copying one pointer's object representation over another's
    // is defined. No cast is needed to get a `char *` that aliases a pointer,
    // because the `void *` round trip is implicit both ways, and this
    // frontend accepts it. Found by review, which compiled the program with
    // `clang -std=c17 -pedantic-errors` and ran it under AddressSanitizer to
    // show there is no use after free in it.
    a_write_through_a_character_pointer_may_have_replaced_what_any_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the write the rule must not fire on: ADR-0028's certain one, which
    // lands in its target and nowhere else, beside an escaped local of the
    // same type that keeps its proof. Firing at every write drops this
    // `error[SC0401]` to a warning.
    a_write_this_check_is_certain_about_leaves_what_another_escaped_local_holds_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_local_that_was_never_given_a_pointer_writes_nowhere: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_through_an_alias_keeps_what_it_carried_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_through_an_alias_that_carries_no_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_write_through_an_alias_leaves_a_sharer_s_proof_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_escaped_local_that_reaches_no_site_at_all: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_reassigned_after_its_address_escaped: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_rebuilt_by_arithmetic_after_its_address_escaped: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_address_that_escaped_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_escape_on_one_arm_and_an_allocation_on_the_other: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_escape_on_one_arm_and_a_shared_allocation_on_the_other: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_address_taken_of_a_local_declared_in_a_loop: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The storage pair ADR-0012 emits, read by the memory check rather than by
    // the lowering that writes it. A local declared inside a loop is given
    // fresh storage each time round, and the only path from its `StorageDead`
    // back to it is the back edge, so this is the one shape where the previous
    // iteration's facts can still be standing.
    //
    // **The write before the assignment is what makes it visible.**
    // `Known::clear` clears the local's edge to its sites and not the sites
    // themselves, so a stale `Freed` can only be observed by a read that
    // happens before the local is given anything. `*p` on an indeterminate
    // pointer is a defect with a check of its own that does not exist yet, and
    // when that check lands this case will have something to say about it: the
    // program is chosen for a read this compiler currently answers nothing
    // about, which is RK-010's shape.
    //
    // Mutation: both the `StorageLive` and the `StorageDead` arm of
    // `Allocations::element` to no-ops. The free from the previous iteration
    // arrives at the write and this fails on its `.stderr`, which gains a
    // `warning[SC0402]` beside the null one.
    a_scope_reentered_forgets_what_its_local_held: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_where_one_of_two_allocations_is_live: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_double_free_is_silent_at_safety_off: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
    an_unproven_free_is_silent_at_safety_off: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
    // The other side of ADR-0033, and the only cases that reach it: an unproven
    // conclusion is an error wherever a check runs, so a warning needs the run
    // to have asked for one. Without these three the arm of `certainty_note`
    // that is not promoted has no observer outside the renderer's own tests.
    an_unproven_free_is_a_warning_under_allow_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
    an_unproven_use_is_a_warning_under_allow_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
    // What a run is told when the level it asked for is not the level it gets,
    // which is ADR-0035. Two independent things lower it, so two of these are
    // one cause each and two are the edges that a gate on one cause alone gets
    // wrong.
    //
    // Mutation: in `driver.rs::undelivered`, compare `options.safety` against
    // `SafetyLevel::IMPLEMENTED` rather than against `Options::delivered`, which
    // drops the artifact half. The second fails and the first stays green, which
    // is the shape `CLAUDE.md` calls the worst defect this project has had: two
    // axes, and a gate that answers one of them.
    //
    // Mutation: answer the conclusion `Unsafe` rather than `Unknown`, which is
    // how `Diagnostic::concluded` builds an error instead. All three of the
    // reporting cases fail and the fourth is the one whose `.exit` goes from 0
    // to 1: the conclusion is the whole of what lets `--allow-unknown` reach
    // this report.
    //
    // **The third is documentation rather than a discriminating guard, and that
    // is measured rather than hoped.** It is the only case anywhere that names a
    // satisfiable level beside an artifact that carries no check, so nothing
    // else pins the combination; but every mutation tried on the gate fails it
    // together with all 76 bare dump cases rather than alone. Reporting on the
    // artifact without asking the level fails 84 tests, this among them. There
    // is no mutation that isolates it, because the derived default already makes
    // a bare dump resolve to `off`, so "report above `off`" and "report when
    // delivered differs" agree on every input in the corpus.
    a_level_with_no_checks_behind_it_is_not_delivered: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
    an_artifact_that_stops_before_the_ir_delivers_no_checks: ["--emit", "ast", "--safety", "memory"],
    a_level_that_was_not_asked_for_is_not_reported: ["--emit", "ast", "--safety", "off"],
    a_migrating_run_is_told_what_was_not_established_and_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "lifetime", "--allow-unknown"],
    // Both causes at once, which the two above have one each of. A review found
    // the report speaking whichever cause was written first, and its remedy
    // sending the reader to `--emit safety-ir`, which is refused again for the
    // other reason: ADR-0034 makes a remedy the change that would make the
    // program compile, so half of that one was a promise this run breaks.
    //
    // Mutation: choose the note and the remedy with one `if` on
    // `options.emit.reaches_the_ir()`, as it was written. This fails and the two
    // single-cause cases stay green, which is the shape of the defect rather
    // than its size.
    //
    // Mutation: offer `--allow-unknown` whatever the level. This fails: the
    // level here is `strict` and `Cli::check` refuses that pair, so following
    // the remedy is an argument conflict rather than a build.
    both_reasons_a_level_can_go_undelivered_are_said_at_once: ["--emit", "ast", "--safety", "strict"],
    a_double_free_is_found_on_a_backend_run: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    a_discarded_dereference_reaches_the_backend: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    // `pp[0]` is `*pp`, and the backend can write `*pp`. It used to refuse
    // this with `SC0801`, because the subscript built an addition and the
    // backend cannot write pointer arithmetic; ADR-0021 folded the addition
    // away and the refusal went with it. `an_ir_shape_the_backend_cannot_write`
    // is the case that holds the refusal itself, which `pp[1]` still earns.
    a_zero_subscript_reaches_the_backend: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],

    // The nullability check's cases, kept together for the reason the memory
    // check's are: one table, whose rows are the three-valued model met by a
    // program that proves the answer, one that leaves it unproven, and one
    // that tests the pointer and so needs neither.
    //
    // The row that proves it.
    a_null_pointer_dereferenced_is_unsafe: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // **Three tests of a pointer are three cases and not one**, because the
    // shapes reach the check differently: `if (p)` hands the pointer's own
    // place to the terminator with no element in the block, and `p != 0` and
    // `p == 0` put a comparison above it that has to be read back through the
    // block. A reader of the C cannot tell those apart, which is why each is
    // held.
    a_pointer_tested_before_it_is_dereferenced: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_compared_against_zero_before_it_is_dereferenced: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_whose_null_arm_returns_is_not_null_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A fourth shape, and the reason it is here rather than left out: C17
    // 6.5.3.3 p5 says `!E` is equivalent to `(0==E)`, so this is the case
    // above written shorter and a reader cannot tell them apart. It reached
    // the check as `Unary "Not"` and was read by nobody, which made the two
    // spellings two answers.
    a_pointer_tested_with_a_logical_negation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The rows that leave it unproven: a parameter, whose nullness is a
    // caller's fact, and an allocation, whose nullness is the allocator's.
    a_parameter_dereferenced_without_a_test_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `docs/roadmap.md`'s own headline example. C17 7.22.3.4 p3 makes a
    // `malloc` result either null or the allocation, so this warns, and the
    // roadmap's example is a program this compiler has something to say about.
    an_allocation_dereferenced_without_a_test_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The address of a local is the only thing this check can prove not null,
    // and a store through a pointer that may hold that address is what takes
    // the proof back. Without it this program said nothing at all, while `p`
    // was provably null at the write: `docs/safety-model.md`'s worst answer,
    // found by review. One case for one rule: a callee handed `&p` is the same
    // escape through a different door.
    a_pointer_written_through_its_own_address_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The two that hold ADR-0025, and the second is the one that says the fact
    // is per path: the arm that skips the dereference joins back in.
    a_pointer_dereferenced_twice_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_dereferenced_on_one_arm_is_not_proved_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // One element that dereferences two pointers, one proved null and one
    // nothing is known about. The words do not name the value, so the two
    // cannot be told apart at one caret and only the worse is said.
    a_proved_null_dereference_beside_an_unproven_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same pair with the worse one written second, which is what tells
    // "the worst of them" apart from "the first of them": the places an
    // element dereferences are collected destination first, so the case above
    // has them agreeing and only this one does not.
    a_proved_null_dereference_after_an_unproven_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A call reading through a pointer it then assigns to. What this pins is
    // the answer rather than the rule behind it: a call's result lands in a
    // fresh temporary here and is copied out in an element of its own, so the
    // order the arguments and the destination are applied in cannot be seen
    // from any C this frontend lowers.
    // `a_call_that_reads_a_pointer_and_writes_it_keeps_neither` in
    // `crates/safec-ir/tests/nulls.rs` is what holds that.
    a_call_whose_destination_it_dereferences: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_null_dereference_is_silent_at_safety_off: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
    an_unproven_dereference_is_a_warning_under_allow_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],

    // `_Nonnull`, the first annotation: believed by the body, checked at every
    // call, refused everywhere else. See ADR-0037, whose Confirmation names
    // the mutation each of these fails under.
    //
    // One case per stage the annotation passes through, for RK-033's reason:
    // the tree, a declaration's IR, and a definition read by the analysis.
    a_nonnull_parameter_is_read_into_the_tree: ["--emit", "ast"],
    // The parameter's own pointer is the last one written, not the first.
    // Mutation: have `parameter_list` read `derivations.first()`, which drops
    // this annotation in silence; only this case fails.
    a_nonnull_after_the_last_star_of_a_parameter_is_read: ["--emit", "ast"],
    a_nonnull_parameter_of_a_declaration_reaches_the_ir: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_parameter_declared_nonnull_is_dereferenced_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // What the body stopped carrying, the caller carries.
    a_null_constant_passed_to_a_nonnull_parameter_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_local_proved_null_passed_to_a_nonnull_parameter_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_nothing_established_passed_to_a_nonnull_parameter_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_nothing_established_passed_to_a_nonnull_parameter_is_a_warning_under_allow_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
    each_argument_to_a_nonnull_parameter_is_asked_about_on_its_own: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A call through `void g();` passes nothing for a parameter the body
    // believes. Mutation: zip the arguments with the parameters in
    // `report_arguments`; only this case fails, and it goes silent.
    a_nonnull_parameter_a_call_passes_no_argument_for_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `g(*pp)` asks two questions at one caret: whether `pp` is null, and
    // whether what it holds is. Mutation: skip an argument with a projection
    // in `report_arguments`; only this case fails, losing the `SC0405`.
    an_argument_read_through_a_pointer_is_asked_about_as_a_dereference_and_as_an_argument: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The two ways a caller discharges it, which are what keep the check from
    // reporting every call.
    a_tested_pointer_passed_to_a_nonnull_parameter_is_silent: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_nonnull_parameter_passed_on_to_another_is_silent: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // What the body believes is a fact at its entry, and no more than that.
    a_nonnull_parameter_whose_address_escaped_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_nonnull_parameter_given_a_null_in_the_body_is_proved_null: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_null_passed_to_a_nonnull_parameter_is_silent_at_safety_off: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
    // Everywhere it cannot apply, one case per reason `parser.rs::placed`
    // gives.
    a_nonnull_not_after_a_star_is_refused: ["--emit", "ast"],
    // The second of two after one `*`. Mutation: give it the label above;
    // only this case fails.
    a_second_nonnull_on_one_pointer_is_refused: ["--emit", "ast"],
    a_nonnull_on_a_pointer_inside_a_parameter_is_refused: ["--emit", "ast"],
    a_nonnull_on_a_file_scope_object_is_refused: ["--emit", "ast"],
    a_nonnull_on_a_local_is_refused: ["--emit", "ast"],
    a_nonnull_on_a_return_type_is_refused: ["--emit", "ast"],
    a_nonnull_on_a_parameter_of_a_function_pointer_is_refused: ["--emit", "ast"],
    a_nonnull_on_a_parameter_of_a_block_scope_function_is_refused: ["--emit", "ast"],
    // C17 6.7.6.3 p8 adjusts a parameter of function type to a pointer, so it
    // declares no function, and the label must not say it does. Mutation:
    // choose the block-scope label on `own` alone; only this case fails.
    a_nonnull_on_a_parameter_of_a_parameter_that_is_a_function_is_refused: ["--emit", "ast"],
    // The disagreement `lowering.rs::agree` refuses, both ways round.
    declarations_that_disagree_about_nonnull_are_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_definition_that_disagrees_with_a_later_declaration_about_nonnull_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Mutation: compare only the first parameter in `agree`; only this fails.
    declarations_that_disagree_about_a_later_parameter_are_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `void g();` declares no parameters, so it cannot be what the rest agree
    // with. Mutation: have `declare_one` call `agree` for `()` as well; only
    // this case fails, and it goes silent.
    an_unprototyped_declaration_does_not_stand_in_for_the_first_prototype: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The promise on a declaration and not on the definition. The body is
    // built from the definition, so it believes nothing and its dereference
    // is reported beside the refusal.
    a_declaration_that_says_nonnull_where_its_definition_does_not_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],

    // A hatch: a function definition whose unproven conclusions are listed
    // rather than reported. See ADR-0038, whose Confirmation names the
    // mutation each of these fails under.
    //
    // One case per stage it passes through, for RK-033's reason: the tree, the
    // IR, and the listing.
    a_hatch_is_read_into_the_tree: ["--emit", "ast"],
    // The pair the record is about: one body, reported outside a hatch and
    // not inside one. Mutation: have `Lowering::body` never call
    // `Function::unchecked`; the first of the two fails.
    an_unproven_dereference_inside_a_hatch_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    the_same_dereference_outside_a_hatch_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Mutation: have `route` keep nothing in `hatched`, which is the hatch as
    // a suppression; this fails.
    an_unproven_dereference_inside_a_hatch_is_listed_and_not_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
    // Mutation: answer `!in_a_hatch` for `Unsafe` in `route`; this fails.
    a_proved_double_free_inside_a_hatch_is_still_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
    // The silent direction of getting the function wrong: a conclusion about
    // a function after a hatch, credited to the hatch, is not reported. The
    // hatch is the unit's first function so that a finding naming the first
    // one lands on it. Mutation: have either check's `Finding::function` name
    // `unit.functions().next()`; this fails.
    a_function_after_a_hatch_is_still_answered_for: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
    // The two checks' conclusions interleaved by caret, where each check's own
    // come out in a run. Mutation: drop the sort in `dump_hatches`; this fails.
    what_a_hatch_concluded_is_listed_in_the_order_it_was_written: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
    // The boundary is the prototype, and the checked side reads it.
    a_null_passed_to_a_nonnull_parameter_of_a_hatch_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // What a hatch may have done to what it was handed is assumed to be the
    // worst, because it cannot yet say otherwise: ADR-0032's default.
    freeing_what_was_handed_to_a_hatch_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A hatch's body is the one whose unproven conclusions are not reported,
    // so what the caller cannot see it do is assumed to be the worst: every
    // allocation still live is unproven after a call to one. Mutation: drop
    // the loop over `value.state` in `memory.rs`'s `Callee::Opaque` arm; the
    // first two fail, and the first is a use after free going silent.
    what_a_hatch_frees_through_what_it_was_handed_is_unproven_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_allocation_a_hatch_was_not_handed_is_unproven_after_it_too: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Mutation: make that loop mark every site rather than the live ones; this
    // fails, a proved double free becoming unproven.
    a_free_proved_before_a_call_to_a_hatch_stays_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `&&` writes both operands at one caret. Mutation: keep the first finding
    // at a caret in `nullability::findings`' `dedup_by` rather than the worst;
    // this fails, and the proved dereference builds.
    a_proved_null_dereference_beside_an_unproven_one_in_a_hatch_is_still_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
    // An attribute `sema::resolve` refused is not a hatch, even on the run that
    // is written anyway. Mutation: have the lowering mark a hatch wherever an
    // attribute is present; this fails.
    an_unproven_dereference_behind_a_refused_attribute_is_still_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Mutation: have `dump_hatches` list every hatch's conclusions under each;
    // this fails.
    each_hatch_lists_only_what_was_concluded_inside_it: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
    // The listing is the count of hatches, whether or not a check ran.
    every_hatch_is_listed_at_safety_off_with_nothing_under_it: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
    a_program_with_no_hatch_lists_none: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
    // Everywhere it cannot apply, and every attribute that is not it: one case
    // per place `parser.rs` and `sema.rs` refuse one.
    a_hatch_on_a_declaration_is_refused: ["--emit", "ast"],
    an_attribute_with_no_argument_is_refused: ["--emit", "ast"],
    an_attribute_with_nothing_in_it_is_refused: ["--emit", "ast"],
    an_attribute_whose_argument_is_not_a_string_is_refused: ["--emit", "ast"],
    an_attribute_list_of_more_than_one_is_refused: ["--emit", "ast"],
    an_attribute_with_a_second_argument_is_refused: ["--emit", "ast"],
    an_attribute_whose_string_is_two_strings_is_refused: ["--emit", "ast"],
    an_attribute_other_than_annotate_is_refused: ["--emit", "ast"],
    an_annotation_other_than_the_hatch_is_refused: ["--emit", "ast"],
    a_second_attribute_before_a_definition_is_refused: ["--emit", "ast"],
    an_attribute_after_a_specifier_is_refused: ["--emit", "ast"],
    an_attribute_in_a_block_is_refused: ["--emit", "ast"],
    an_attribute_on_a_parameter_is_refused: ["--emit", "ast"],

    // What code this check cannot read may reach: an allocation is exposed
    // once it may, every opaque call unproves every exposed one, and may
    // return any of them. See ADR-0039, whose Confirmation names the mutation
    // each of these fails under.
    //
    // Reported, each a use after free or a double free for some definition
    // of the callees C permits.
    what_a_callee_frees_through_a_pointer_stored_in_the_heap_is_unproven_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    what_a_callee_frees_through_a_pointer_stored_in_a_local_is_unproven_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_call_may_return_what_it_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_call_handed_an_address_may_return_what_is_behind_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_call_may_return_what_an_earlier_call_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_call_in_a_loop_may_return_what_it_returned_last_turn: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_allocation_exposed_on_one_arm_is_unproven_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_stored_on_one_arm_is_reached_through_what_holds_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_free_proved_before_a_call_stays_proved_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    the_old_pointer_realloc_was_handed_is_unproven_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_freed_pointer_handed_to_realloc_is_freed_twice: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A local whose address a call kept earlier is reached by every call
    // after it, handed anything or nothing.
    a_pointer_in_a_local_whose_address_an_earlier_call_kept_is_unproven_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Exposed on one arm and holding the pointer on the other: the closure
    // over contents has to run over every exposed allocation, not only the
    // ones a call has just marked.
    a_pointer_in_a_table_exposed_on_the_other_arm_is_unproven_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The callee had what it returned, and may have kept it.
    what_a_call_returned_is_unproven_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `memset` frees nothing and still exposes what it was handed.
    what_memset_was_handed_is_unproven_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `memcpy` can be handed a local's address and write a pointer into it,
    // so a free through that local proves nothing about what a sharer holds.
    a_local_memcpy_is_handed_the_address_of_may_hold_something_else_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A loop through one call writes one site, and what the call returns may
    // be what was freed through that site last turn.
    a_call_in_a_loop_may_hand_back_what_was_freed_last_turn: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // A write two levels down is not recorded as contents, so it exposes what
    // it carries at once.
    a_pointer_stored_two_levels_down_is_reached_through_what_holds_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Refused and well defined: two costs ADR-0039 accepts, pinned so that a
    // change to either is seen. The first is #253.
    a_free_on_reallocs_failure_branch_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_call_after_an_allocation_was_exposed_may_return_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Built.
    what_memset_returns_is_the_allocation_it_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    memcpy_frees_neither_of_its_arguments: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    what_strcpy_returns_is_the_allocation_it_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_allocation_no_call_can_reach_stays_proved_across_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_pointer_stored_in_the_heap_is_not_exposed_until_what_holds_it_is: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_allocation_made_again_at_a_site_is_not_the_one_exposed_before: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `realloc`'s size is not asked whether it was freed.
    the_size_realloc_is_handed_is_not_asked_whether_it_was_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],

    a_block_declaration_carries_its_initializer: ["--emit", "ast"],
    a_block_declaration_does_not_leave_its_block: ["--emit", "ast"],
    a_braced_initializer_is_refused: ["--emit", "ast"],
    // The two codes a constant this compiler cannot read is reported under,
    // and `--emit safety-ir` rather than `--emit ast` because what each one
    // pins is that there is exactly one report. `types.rs` says it and the
    // lowering says nothing more, which is only visible on a run that lowers.
    // A value `int` does not hold is this compiler's gap, and `clang 20.1.6
    // -std=c17 -pedantic-errors` compiles this program; a spelling that is no
    // constant at all is the program's, and `clang` refuses it too.
    a_constant_no_integer_type_here_can_hold: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_spelling_that_is_not_a_constant: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The suffix is refused rather than read, and the note says why: C
    // computes `-6 / 3u` as an unsigned division. Reading `3u` as an `int`
    // compiled this program to a signed division and reported nothing, which
    // is the one shape this stage exists to stop.
    a_suffixed_constant_has_no_type_here: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_comma_in_a_controlling_expression: ["--emit", "ast"],
    a_dangling_else: ["--emit", "ast"],
    a_definition_that_is_not_a_function: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_function_the_ir_cannot_hold: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_declaration_is_not_a_body: ["--emit", "ast"],
    a_failed_parse_reports_no_names: ["--emit", "ast"],
    a_file_scope_declaration_carries_its_initializer: ["--emit", "ast"],
    a_later_declarator_is_not_in_scope_in_an_earlier_initializer: ["--emit", "ast"],
    a_lexical_error_leaves_no_ir: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The `--emit llvm-ir` cases, kept together because what each is for is
    // only visible beside the others. `every_operator` is the one that stops
    // the operator table being a table nothing checks: without it, spelling
    // `BitAnd` as `or`, `Mul` as `add`, `Le` as `lt`, `Neg` as `add` and
    // `BitNot` as `xor 0` all passed the whole suite, which is RK-001's shape.
    // `conversions_and_a_constant_condition` is the same for C17 6.3.1.3 and
    // 6.5.2.2 p7: `c = 300` and `narrow(300)` both answer 44, and a constant
    // that ignored its destination's type passed everything before it. `mix`
    // is there because every other call in the suite has one parameter or two
    // of one type, so pairing each argument with the wrong parameter passed
    // everything too.
    //
    // The two `extended` cases are one program on two machines, because what
    // differs is the machine: `x86_64-unknown-linux-gnu` asks for `signext` and
    // `armv7-unknown-linux-gnueabihf` for `zeroext`, and the `int` beside the
    // `char` is what says the rule reads a width rather than a type. Every
    // other `--emit llvm-ir` case is on a target that asks for nothing, so
    // these two are the only place in the tree an attribute appears at all.
    //
    // Every one of these is also in `llvm.rs`, which hands it to `clang`. The
    // text and whether the text is LLVM are two claims.
    a_narrow_unsigned_value_is_extended_without_a_sign: ["--emit", "llvm-ir", "--target", "armv7-unknown-linux-gnueabihf"],
    a_narrow_value_is_extended_where_the_target_asks: ["--emit", "llvm-ir", "--target", "x86_64-unknown-linux-gnu"],
    an_ir_shape_the_backend_cannot_write: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    llvm_ir_follows_the_target: ["--emit", "llvm-ir", "--target", "aarch64-unknown-linux-gnu"],
    llvm_ir_of_conversions_and_a_constant_condition: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    llvm_ir_of_every_operator: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    llvm_ir_of_pointers_branches_and_a_loop: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    the_mvp_becomes_llvm_ir: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    a_parse_error_leaves_no_ir: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_directive_stops_the_input_it_is_in: ["--emit", "ast"],
    a_tree_deeper_than_the_indent_shows: ["--emit", "ast"],
    abstract_function_parameter: ["--emit", "ast"],
    abstract_function_type_parameter: ["--emit", "ast"],
    add: ["--emit", "tokens"],
    an_array_length_stops_at_a_comma: ["--emit", "ast"],
    an_initializer_becomes_a_store: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_initializer_can_name_what_it_initializes: ["--emit", "ast"],
    an_initializer_stops_at_the_comma: ["--emit", "ast"],
    array_declaration: ["--emit", "ast"],
    array_length_is_not_evaluated: ["--emit", "ast"],
    assigning_the_wrong_type: ["--emit", "ast"],
    // C17 6.7.9 p11 gives an initializer the constraints of simple
    // assignment, so the spelling a declaration uses is the same rule.
    //
    // Mutation: have `types.rs::Checker::receivers_in` insert nothing for a
    // `Stmt::Declaration`. The `SC0302` goes from both of these and both fail.
    initializing_with_the_wrong_type: ["--emit", "ast"],
    // C17 6.5.16.2's two constraints, which are not the rule for a plain
    // `=`: `p += 1` is allowed and holds `a_compound_assignment_on_a_pointer_
    // computes_into_a_pointer` silent. The first and the last put the primary
    // caret on the value, the middle two on the place, because that is the
    // operand the rule refuses.
    //
    // Mutation: have `types.rs::Checker::type_of` stop calling
    // `compound_assignment`. All four go silent and exit 0. Mutation: put the
    // primary label on the value always. The middle two move their caret.
    // Mutation: put it on the place always. The outer two move theirs.
    adding_a_pointer_into_an_integer: ["--emit", "ast"],
    multiplying_a_pointer_in_place: ["--emit", "ast"],
    shifting_a_pointer_in_place: ["--emit", "ast"],
    subtracting_a_pointer_from_a_pointer_in_place: ["--emit", "ast"],
    // C17 6.5.5 to 6.5.14, one clause per operator: the same `SC0306`, for a
    // binary operator. Every row but `p == 1 - 1` is an error under `clang
    // --target=x86_64-unknown-linux-gnu -std=c17 -pedantic-errors`, and that
    // one is `docs/frontend.md`'s null pointer constant table. The control
    // holds every pairing the clauses allow, so a clause answered too strictly
    // fails as surely as one answered too loosely; the unit test
    // `a_binary_operator_answers_for_every_operator_and_operand` holds the
    // same rows one at a time.
    //
    // Mutation: have `types.rs::Checker::binary` stop calling
    // `binary_operable`. The first four go silent and exit 0. Mutation: have
    // the `==` arm refuse a null pointer constant, or the relational arm two
    // pointers. The control reports.
    a_pointer_is_not_an_arithmetic_operand: ["--emit", "ast"],
    a_comparison_of_a_pointer_with_what_c_does_not_allow: ["--emit", "ast"],
    an_additive_operand_c_does_not_allow: ["--emit", "ast"],
    a_void_operand_is_refused_by_every_binary_operator: ["--emit", "ast"],
    every_binary_operator_takes_what_c_allows: ["--emit", "ast"],
    // C17 6.5.6 p2 and 6.5.16.2 p1: `+` and `+=` step only a pointer to a
    // complete object type, and `void`, a function and an array of unknown
    // length are not one. `clang` accepts the first three lines as a GNU
    // extension unless `-pedantic-errors`, which is why `docs/frontend.md`
    // lists them, and refuses the array either way. The last two lines are
    // the control: a pointer to `int` is stepped in both spellings in
    // silence.
    //
    // Mutation: have `unsteppable` answer `None` for `void`. Lines 3 and 4 go
    // silent. For a function, line 6 does, and for an array of unknown
    // length, lines 7 and 8. Mutation: drop the note from either report. Its
    // lines lose it. Mutation: have `unsteppable` refuse `int`. The control
    // reports.
    arithmetic_on_a_pointer_to_something_that_is_not_a_complete_object: ["--emit", "ast"],
    // The same rule in its other two spellings: C17 6.5.2.4 p2 and 6.5.3.1
    // p2 define an increment as `+= 1`, and 6.5.2.1 p1 gives a subscript the
    // same constraint. Lines 4 to 12 are refused, and `clang -pedantic-errors`
    // refuses the same nine. `1[v]` and `1[g]` are there because the pointer
    // can be either operand of `[]`, and `g[1]` and `1[g]` because a function
    // is refused only as the pointer 6.3.2.1 p4 makes of it. Lines 13 to 17
    // are the control, in the same run so that their silence is asserted
    // beside reports.
    //
    // Mutation: have `increment` stop asking `unsteppable`. Lines 4 to 7 go
    // silent. Mutation: have `subscript` stop asking it. Lines 8 to 12 do.
    // Mutation: have `subscript` ask only when the base is the pointer. Lines
    // 10 and 12 go silent. Mutation: drop `decayed` from the base in
    // `subscript`. Line 11 goes silent, and from the index, line 12.
    // Mutation: swap the two increments' clauses. Lines 4 to 7 change note.
    // Mutation: have `unsteppable` refuse `int`. The control reports.
    a_step_by_increment_or_subscript_on_a_pointer_to_something_that_is_not_a_complete_object: ["--emit", "ast"],
    // Why the rule matters past the message, as for the initializer below:
    // `i = p * 1` writes a multiplication over an `int *` into a local
    // declared `int`, which is the shape `docs/c-family.md`'s fourth
    // requirement forbids. `--emit safety-ir` so that the run reaches the
    // lowering, which a type error does not stop.
    //
    // Mutation: have `binary` stop calling `binary_operable`. The `.stderr`
    // goes empty and the `.exit` goes to 0. Mutation: answer `None` rather
    // than `int` for a refused operation. `SC0304` joins each report,
    // calling the program a gap in this compiler.
    an_allocation_multiplied_into_an_integer_is_a_type_error: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // Why the rule matters past the message. ADR-0030 has the memory check
    // drop the operand of an addition that is declared `int`, so an
    // allocation reaching `i` through this initializer is one that check is
    // handed and cannot see. `--emit safety-ir` so that the run reaches it,
    // which a type error does not stop. Under the mutation above the
    // `SC0302` goes and so does anything about `p`: what is left is
    // `SC0404` about `free(r)`, measured, where the same program answered
    // `SC0401` about `p` before ADR-0030.
    an_allocation_initialized_into_an_integer_is_a_type_error: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    block_declaration: ["--emit", "ast"],
    block_declaration_without_a_semicolon: ["--emit", "ast"],
    block_function_declaration: ["--emit", "ast"],
    call: ["--emit", "ast"],
    each_declarator_derives_its_own_type: ["--emit", "ast"],
    one_declaration_declares_several_names: ["--emit", "ast"],
    edges_of_a_branch_and_a_loop: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    every_shape_the_artifact_spells: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    compound_assignment: ["--emit", "ast"],
    // The same two operators, at the one type whose operation does not happen
    // at `int`. C17 6.5.6 p8 makes `p + 1` a pointer, and ADR-0030 has the
    // memory check read a local's declared type to tell the pointer operand of
    // an addition from the integer beside it and drop the integer, so a
    // temporary declared `int` holding an allocation is one that check can be
    // handed and not see. `--emit safety-ir` because the declaration being
    // pinned is the IR's: `compound_assignment` and `increment` pin the tree
    // these are read from and stop there, and the tree is where this is right.
    //
    // Mutation: have `lowering.rs::promoted` answer `Ty::Int` for a pointer
    // place again. These two fail and nothing else in the suite does, which is
    // why they are here: the fix is invisible to every other case.
    a_compound_assignment_on_a_pointer_computes_into_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_increment_of_a_pointer_computes_into_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // And the same through a projection, because `promoted` is handed the
    // whole place rather than its base local: `*pp` is an `int *` where `pp`
    // is an `int **`, and the two cases above cannot tell the difference
    // because their places have no projection at all.
    //
    // Mutation: have `promoted` ask about `Place::local(place.local)` instead
    // of `place`. Only this case fails.
    a_compound_assignment_through_a_dereferenced_pointer_computes_into_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    conditional: ["--emit", "ast"],
    empty_character_constant: ["--emit", "tokens"],
    empty_parameter_list: ["--emit", "ast"],
    every_declarator_rule: ["--emit", "ast"],
    expression_statement: ["--emit", "ast"],
    for_statement: ["--emit", "ast"],
    for_with_only_a_condition: ["--emit", "ast"],
    for_with_only_a_step: ["--emit", "ast"],
    for_with_only_an_initialiser: ["--emit", "ast"],
    for_without_clauses: ["--emit", "ast"],
    function_returning_pointer: ["--emit", "ast"],
    if_statement: ["--emit", "ast"],
    incomplete_array: ["--emit", "ast"],
    increment: ["--emit", "ast"],
    missing_semicolon: ["--emit", "ast"],
    mvp_program: ["--emit", "ast"],
    a_scope_that_opens_and_closes: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // `wasm32` on purpose, and not the triple every other IR case names: no
    // CI runner and no developer machine hosts it, so this is the case that
    // fails wherever `--target` stops being honoured. On a machine that hosts
    // the triple the others name, they cannot tell the two apart.
    a_target_the_host_is_not: ["--emit", "safety-ir", "--target", "wasm32-unknown-unknown"],
    places_a_pointer_reaches: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    the_mvp_lowers_to_blocks_and_edges: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    not_a_declaration: ["--emit", "ast"],
    parentheses_regroup: ["--emit", "ast"],
    parsed_function: ["--emit", "ast"],
    pointer_declaration: ["--emit", "ast"],
    pointer_to_function: ["--emit", "ast"],
    precedence_additive: ["--emit", "ast"],
    precedence_assignment: ["--emit", "ast"],
    precedence_bitwise_and: ["--emit", "ast"],
    precedence_bitwise_or: ["--emit", "ast"],
    precedence_bitwise_xor: ["--emit", "ast"],
    precedence_comma: ["--emit", "ast"],
    precedence_conditional: ["--emit", "ast"],
    precedence_equality: ["--emit", "ast"],
    precedence_logical_and: ["--emit", "ast"],
    precedence_logical_or: ["--emit", "ast"],
    precedence_multiplicative: ["--emit", "ast"],
    precedence_relational: ["--emit", "ast"],
    precedence_shift: ["--emit", "ast"],
    several_declarators_each_become_a_function: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    several_declarators_each_become_a_local: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    several_items_and_statements: ["--emit", "ast"],
    returning_the_wrong_type: ["--emit", "ast"],
    subscript: ["--emit", "ast"],
    too_few_arguments: ["--emit", "ast"],
    too_many_arguments: ["--emit", "ast"],
    undeclared_identifier: ["--emit", "ast"],
    unexpected_character: ["--emit", "tokens"],
    unexpected_characters: ["--emit", "tokens"],
    unreadable_expression: ["--emit", "ast"],
    unsupported_directive: ["--emit", "tokens"],
    unterminated_block_comment: ["--emit", "tokens"],
    unterminated_character_constant: ["--emit", "tokens"],
    unterminated_string_literal: ["--emit", "tokens"],
    void_is_not_the_only_parameter: ["--emit", "ast"],
    while_statement: ["--emit", "ast"],
}

/// Everything in `cases/` belongs to a case the table names.
///
/// The table is the definition and the directory follows it, which is the same
/// rule read from the other end. There are three ways to be in there and be
/// dead, and one guard answers for all of them rather than for the first:
///
/// * a `.c` nobody listed is never run;
/// * a `.stdout` left behind by a renamed case is compared against nothing, and
///   then waits for the next case to reuse the name, which starts life failing
///   against content from a case it never heard of;
/// * a subdirectory hides either of those from a walk that does not recurse,
///   and grouping the corpus by phase is the obvious thing to reach for as it
///   grows.
///
/// Dead weight wearing the appearance of coverage is the failure this corpus
/// exists to avoid, so the directory answers for every entry it has.
///
/// Mutation: put anything in `cases/` that the table does not name, at the top
/// level or in a subdirectory of it. This test fails and no other does.
#[test]
fn every_file_in_the_corpus_belongs_to_a_case_in_the_table() {
    const EXPECTED: [&str; 4] = ["c", "stdout", "stderr", "exit"];

    let mut strays: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(cases_dir()).expect("the cases directory is in the repository") {
        let path = entry.expect("a directory entry can be read").path();
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();

        let belongs = path.is_file()
            && path
                .extension()
                .is_some_and(|ext| EXPECTED.iter().any(|known| ext == *known))
            && CASES.contains(&stem.as_str());

        if !belongs {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            strays.push(name.into_owned());
        }
    }
    strays.sort();

    assert!(
        strays.is_empty(),
        "nothing in the table names these, so nothing runs them and nothing \
         says so: {strays:?}"
    );
}

/// Where the cases live, and the directory the compiler is run from.
fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases")
}

/// Run one case and compare all three of its outputs.
///
/// The compiler is run **from** `cases/` and handed a bare file name, because it
/// echoes back the path it was given: `safec --emit tokens
/// crates/safec/tests/cases/add.c` prints
/// `crates/safec/tests/cases/add.c:3:1 keyword "int"`, while the same run from
/// inside the directory prints `add.c:3:1 keyword "int"`. That is what makes an
/// expected file mean the same thing on every machine, and it is why passing a
/// path here would silently break every case that reports a position.
///
/// `--color never` is passed rather than relied on. `ColorMode::Auto` resolves
/// against whether the stream is a terminal, and a test whose meaning depends
/// on not being one changes meaning when somebody runs it differently.
fn run_case(name: &str, args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .current_dir(cases_dir())
        .args(["--color", "never"])
        .args(args)
        .arg(format!("{name}.c"))
        .output()
        .expect("the compiler binary was built for this test");

    let code = output
        .status
        .code()
        .unwrap_or_else(|| panic!("case `{name}`: the compiler was killed by a signal"));

    check(name, "stdout", &output.stdout);
    check(name, "stderr", &output.stderr);
    check(name, "exit", &expected_exit(code));
}

/// The exit code, as the contents of an expected-output file.
///
/// Text, and empty for zero, so that one rule covers all three streams: absent
/// means empty, and for this one empty means zero.
///
/// Split out so that the rule has a guard that states it, rather than leaving
/// it implicit in what the expected files happen to contain: the cases that
/// exit non-zero demonstrate it many times over and this says it once.
///
/// Mutation: return `Vec::new()` whatever the code. The unit test below fails,
/// which is the one that says what the rule is, and so does every case that
/// exits non-zero. **How many that is is not written here**, because it counts
/// the cases that happen to exist and a case is added most weeks; it was nine
/// when this was written and is many times that now. See RK-028.
fn expected_exit(code: i32) -> Vec<u8> {
    if code == 0 {
        Vec::new()
    } else {
        format!("{code}\n").into_bytes()
    }
}

#[test]
fn a_failing_exit_code_is_written_down_and_a_successful_one_is_not() {
    assert!(expected_exit(0).is_empty());
    assert_eq!(expected_exit(1), b"1\n");
    assert_eq!(expected_exit(2), b"2\n");
}

/// Compare one stream against its expected file, or rewrite that file when
/// blessing.
///
/// An absent expected file means the stream has to be empty. Forgetting to
/// write one is not a silent pass: the case then asserts emptiness, and any
/// output at all fails it.
fn check(name: &str, ext: &str, actual: &[u8]) {
    let path = cases_dir().join(format!("{name}.{ext}"));

    if blessing() {
        bless(&path, actual);
        return;
    }

    let expected = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("{}: {error}", path.display()),
    };

    assert!(
        actual == expected,
        "case `{name}`: {ext} does not match {}\n\
         --- expected ---\n{}\n--- actual ---\n{}\n{}\n\
         Run the suite again with SAFEC_BLESS=1 to write what the compiler \
         produced, then read the diff.",
        path.display(),
        String::from_utf8_lossy(&expected),
        String::from_utf8_lossy(actual),
        first_difference(&expected, actual),
    );
}

/// The first line the two disagree on, escaped so that it can be read.
///
/// Printed beside the two blocks because some of what a case pins is invisible
/// on a terminal. `ariadne` ends a caret line with spaces, so the failure this
/// module's comment warns about, an editor stripping them, produces two blocks
/// that look identical and differ by two bytes. Told only that they do not
/// match, the next move is `SAFEC_BLESS=1`, which is the one move that must
/// never be made without reading the difference first.
///
/// Mutation: return `String::new()`. `an_invisible_difference_is_still_shown`
/// fails.
fn first_difference(expected: &[u8], actual: &[u8]) -> String {
    let expected = String::from_utf8_lossy(expected);
    let actual = String::from_utf8_lossy(actual);

    for (index, (want, got)) in expected.lines().zip(actual.lines()).enumerate() {
        if want != got {
            return format!(
                "first difference, line {}:\n  expected {want:?}\n  actual   {got:?}",
                index + 1
            );
        }
    }

    format!(
        "every line they share is equal, so they differ in how many there are: \
         expected {}, actual {}",
        expected.lines().count(),
        actual.lines().count()
    )
}

/// A difference nobody can see still has to be spelled out.
///
/// Mutation: make `first_difference` return `String::new()`. This fails.
#[test]
fn an_invisible_difference_is_still_shown() {
    let shown = first_difference("a\n   x\nb\n".as_bytes(), "a\n   x  \nb\n".as_bytes());

    assert!(shown.contains("line 2"), "{shown}");
    assert!(shown.contains("\"   x\""), "{shown}");
    assert!(shown.contains("\"   x  \""), "{shown}");
}

/// One being a prefix of the other leaves no line to point at.
#[test]
fn a_difference_only_in_length_says_so() {
    let shown = first_difference("a\nb\n".as_bytes(), "a\n".as_bytes());

    assert!(shown.contains("expected 2, actual 1"), "{shown}");
}

/// Write an expected file, or remove it when the stream is empty.
///
/// Removing rather than leaving nothing behind keeps the directory in the form
/// the absent-file rule describes. A blessing that left zero-byte files would
/// make the tree disagree with what the next reader is told.
fn bless(path: &Path, actual: &[u8]) {
    if actual.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("{}: {error}", path.display()),
        }
    } else {
        std::fs::write(path, actual).unwrap_or_else(|error| {
            panic!("{}: {error}", path.display());
        });
    }
}

fn blessing() -> bool {
    decide_blessing(is_set("SAFEC_BLESS"), is_set("CI"))
}

fn is_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

/// Whether to rewrite the expected files instead of comparing against them.
///
/// Split from the environment so that the rule can be tested at all. Setting an
/// environment variable is `unsafe` in this edition, and the gate rejects
/// `unsafe` anywhere under `crates/`, so a test that reached for `set_var`
/// would fail the build rather than guard anything.
///
/// The refusal is keyed on `CI`, not on `gate.sh`. `.github/workflows/ci.yml`
/// runs `cargo test --workspace` directly and never goes through the gate, so
/// guarding the gate would have left the one place that matters able to rewrite
/// its own expectations and report success. It refuses loudly rather than
/// quietly comparing instead, because a run that was asked to write and did not
/// is a run whose result means something other than it appears to.
///
/// Mutation: return `asked` without the assertion. Then
/// `blessing_is_refused_under_ci` fails.
fn decide_blessing(asked: bool, under_ci: bool) -> bool {
    assert!(
        !(asked && under_ci),
        "SAFEC_BLESS is set under CI. Expected output is written by a person \
         who then reads the diff; a run that rewrites its own expectations \
         proves nothing."
    );
    asked
}

#[test]
#[should_panic(expected = "SAFEC_BLESS is set under CI")]
fn blessing_is_refused_under_ci() {
    decide_blessing(true, true);
}

/// Nothing is written unless it was asked for, and asking is the only thing
/// that turns it on.
#[test]
fn expected_files_are_rewritten_only_when_blessing_is_asked_for() {
    assert!(decide_blessing(true, false));
    assert!(!decide_blessing(false, false));
    assert!(!decide_blessing(false, true));
}

/// Blessing writes what the compiler produced, and takes the file away when it
/// produced nothing.
///
/// Guarded here rather than through the corpus, because no case runs with
/// blessing on: `gate.sh` unsets the variable and the harness refuses under CI,
/// which between them mean the whole of `bless` would otherwise be code that
/// nothing in the suite can break.
///
/// Mutation: swap the two branches of `bless`, so that it removes the file when
/// there is output and writes an empty one when there is not. This test fails
/// and no other does.
#[test]
fn blessing_writes_the_output_and_removes_the_file_when_there_is_none() {
    let path = std::env::temp_dir().join(format!("safec_bless_{}.stdout", std::process::id()));
    let _ = std::fs::remove_file(&path);

    bless(&path, b"one\n");
    assert_eq!(
        std::fs::read(&path).expect("blessing wrote the file"),
        b"one\n"
    );

    bless(&path, b"");
    assert!(!path.exists(), "an empty stream leaves no file behind");

    // Blessing an absent file with nothing to write is the ordinary case for a
    // stream that was empty last time too, and is not an error.
    bless(&path, b"");
}
