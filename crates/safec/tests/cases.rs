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
    // nullability check answering a different question. The second reaches the
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
    a_local_given_nothing_forgets_the_set_it_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_may_set_freed_then_written_through_an_alias: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // What a proof about a may-set survives when a value is built from its
    // operands, which is the half of `Held` that is not a may-fact. It carries
    // while the set does not grow, so an offset keeps it and an offset by
    // something that is itself a site does not: `i` is a parameter, so `q + i`
    // reaches `i`'s site too, and a set that has gained a site nothing freed
    // cannot support a proof about the one that was. See ADR-0024.
    a_free_after_an_offset_that_kept_the_set_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    an_offset_that_grew_the_set_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    // The same offset written with a local rather than a constant, which loses
    // the proof however little that local holds. This check does not read
    // types, so a local holding no site is an `int` and a pointer whose
    // allocation it lost at the same time: `p + n` and `base + ok` with `base`
    // read out of another pointer are one shape here. Keeping the proof for the
    // first keeps it for the second, which is a certainty about a value nothing
    // followed, so neither keeps it.
    an_offset_by_a_local_loses_the_proof_whatever_the_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
    // Three programs with one `free` each, and the boundary between two
    // reasons a report can be unproven. In the first two nothing established a
    // free at all, so the diagnostic says what this check lost rather than
    // that the value may have been freed already: `p` was given a constant and
    // holds no site, and `*pp` is a pointer this check follows locals rather
    // than the targets of. The third reaches the same caret with a site an
    // opaque call was handed, where `helper` may really have freed it, and it
    // keeps the older words. It is the only case whose suspicion rests on
    // nothing but the callee: every other program that keeps those words has
    // a `free` in it that this check saw. Answering `Unproven::Lost`
    // where the sites disagree fails it, along with everything else that
    // keeps them.
    //
    // Measured, on the two mutations that send the first two the other way:
    // answering `Unproven::Disagreement` where this check lost the pointer,
    // and reporting nothing there at all, each fail those two and the eleven
    // cases whose text this issue changed, and nothing else in the suite.
    a_free_of_a_pointer_that_never_held_an_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
    a_free_of_a_null_constant: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--deny-unknown"],
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
    an_unproven_free_is_an_error_under_deny_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--deny-unknown"],
    an_unproven_use_is_an_error_under_deny_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--deny-unknown"],
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
    an_unproven_dereference_is_an_error_under_deny_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--deny-unknown"],

    a_block_declaration_carries_its_initializer: ["--emit", "ast"],
    a_block_declaration_does_not_leave_its_block: ["--emit", "ast"],
    a_braced_initializer_is_refused: ["--emit", "ast"],
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
    block_declaration: ["--emit", "ast"],
    block_declaration_without_a_semicolon: ["--emit", "ast"],
    block_function_declaration: ["--emit", "ast"],
    call: ["--emit", "ast"],
    each_declarator_derives_its_own_type: ["--emit", "ast"],
    one_declaration_declares_several_names: ["--emit", "ast"],
    edges_of_a_branch_and_a_loop: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    every_shape_the_artifact_spells: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    compound_assignment: ["--emit", "ast"],
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
