//! C programs, and the output the compiler is expected to produce for them.
//!
//! A case is a `.c` file in a group's directory under `cases/` and up to three
//! expected-output files beside it. Adding one is those files and a line in the table below; no Rust
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
///
/// **A block per group, and the tests flat.** The group is the directory under
/// `cases/` a case's files live in, and nothing else: every test is still a
/// top-level function named after its case, so a test's name and every record
/// that cites one are what they were before the corpus was grouped, and the
/// same name in two groups is `E0428` rather than two tests a citation cannot
/// tell apart. See ADR-0007.
macro_rules! cases {
    ($($group:literal => { $($name:ident : [$($arg:literal),* $(,)?]),* $(,)? })*) => {
        /// Every case the table names, with the group it lives in.
        const CASES: &[(&str, &str)] = &[$($(($group, stringify!($name))),*),*];

        $($(
            #[test]
            fn $name() {
                run_case($group, stringify!($name), &[$($arg),*]);
            }
        )*)*
    };
}

// The table is written out rather than discovered by walking `cases/`.
// See ADR-0007 for why, and for what it rejected.
//
// A group is a section of this table, and a case goes in the group whose
// section it belongs to. Its files go in the directory of that name.
cases! {
    "memory" => {
        // The memory check's cases, kept together because what each one is for
        // is a row of one table: `docs/safety-model.md`'s three-valued model
        // met by a program that proves it, one that leaves it unproven, and one
        // that does neither.
        a_value_freed_twice: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_value_freed_through_a_copy: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_freed_twice: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same function with its only caller in view, passing null. See
        // ADR-0027: a parameter ranges over what any caller may pass.
        a_double_free_in_a_function_the_only_caller_passes_null_to_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_branch_that_allocates_either_way: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_value_used_after_it_was_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same free and the same use, in one full expression with nothing
        // ordering them. C17 6.5 p3 leaves the operands of `+` unsequenced, so
        // one allowed order reads `*p` first and the program is defined; which
        // order an implementation picks is unspecified and this compiler does
        // not get to choose. The pair is written both ways round because the
        // answer used to turn on which side the free was written, and what
        // decided was that ADR-0010 makes a call end a block. See ADR-0022.
        //
        // **The third is the same program with the use read first**, which the
        // walk meets before it meets the free. A forward walk only looks back,
        // so that half is answered by carrying the read forwards to the free
        // instead: see ADR-0023. `*p` on its own is not read until the addition
        // is built, which is after the call, so the pair above are answered by
        // the free marking what follows it; put the use inside a call and it is
        // read first and the read is what waits.
        an_unsequenced_free_and_use_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        the_same_program_with_the_operands_swapped_is_not_proved_either: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_unsequenced_use_the_check_meets_first_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same, for the two reads an *element* carries rather than a call
        // or a branch: writing through the pointer, and evaluating a place for
        // no reason but the evaluation. Neither is reached by any case above,
        // whose reads are all carried by a terminator, so without these the
        // element half of the recording can be deleted with the suite green.
        // Measured, which is how they came to be here.
        a_write_through_a_pointer_the_check_meets_first_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_discarded_read_the_check_meets_first_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the two edges of that: a read of something this free is not
        // about, and a read that cannot have happened on the path the free is
        // on. The first says the sites are compared rather than the spans, and
        // the second says the reads are carried along the graph's edges rather
        // than along the order the blocks were written in.
        a_use_of_another_pointer_before_a_free_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_use_and_a_free_on_two_arms_of_one_conditional_are_not_both_reached: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the read that has to cross a merge to reach the free at all: it
        // is inside one arm of a `?:` that the free is outside of, so nothing
        // but the join carries it. The case above puts the two on opposite arms
        // and asks for silence; this one puts the read on an arm and the free
        // after the merge and asks for a report, which is the only shape where
        // losing the join's union of the carried reads is a silence rather than
        // a noise.
        a_read_inside_one_arm_before_a_free_survives_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And what the report is allowed to *name*. The read may have gone
        // through either allocation, so `allocated here` would be a caret on
        // one of two lines with nothing to choose between them, which is the
        // may-set mistake of concluding from one member, made about a label
        // rather than about a proof. Its `.stderr` has no such caret, and
        // folding the two with anything but `same` puts one back.
        a_read_of_either_of_two_allocations_before_a_free_names_neither: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the read the free's *own* argument evaluation performed, which
        // C17 6.5.2.2 p10's first sentence orders before the call
        // unconditionally. The carried read has to stop at that point like it
        // stops at any other, so neither of these gets an `SC0402`; the
        // `SC0403` each still carries is the nullability check answering a
        // different question, and the `SC0404` is ADR-0036's, because an offset
        // read through a pointer is not one this check can evaluate. The second
        // reaches the same place through a nested call, which is the spelling
        // where the read is further from the free than an operand of it.
        //
        // The first writes through `p` before the free so that the program is
        // one C defines: reading an allocation nobody wrote to is 7.22.3.4 p2's
        // indeterminate value and the offset would leave the object, and a
        // guard is worth more when what it guards is defined. The second cannot
        // be repaired that way, because `g`'s result is not this check's to
        // know, and it is here for its shape.
        //
        // `Element::ArgumentsEvaluated` is what they are about, and ADR-0026 is
        // why it says less than `Element::Sequenced`.
        //
        // Mutation: the arm in `memory/transfer.rs` that reads that element
        // doing nothing. These two fail on their `.stderr` with the `SC0402`
        // back, and nothing else fails. The rule's other half is the lowering
        // that emits it, and mutating that fails these two on their `.stdout`
        // along with every other artifact holding a call, so the halves are
        // mutated apart: a mutation is measured by which assertion it broke,
        // not only by whether one did.
        a_read_in_a_frees_own_argument_is_ordered_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_in_a_frees_argument_through_a_call_is_ordered_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same question asked of a call this check cannot read, which may
        // have freed what it was handed and cannot say. The pair is the
        // asymmetry itself: the first reads through `p` in the operand beside
        // the call, where C17 6.5 p3 orders neither against the other, and the
        // second puts a sequence point between them and has to stay silent.
        // Without the first, `used_before` can go back to refusing every callee
        // but `free` with the suite green.
        //
        // Its `.stderr` is where the words are pinned: `allocated here` and
        // `used here, perhaps after the free`, and **no** `freed here` caret
        // and no 6.5.2.2 p10 note, because nothing established a free. That is
        // the whole of what `Unproven::Disagreement` buys over
        // `Unproven::Unsequenced` here.
        //
        // Both test `p` before reading it so that what they pin is this check
        // rather than the nullability one, whose `SC0403` would otherwise be in
        // both files and would make the second case's silence a sentence rather
        // than an empty file. They write through `p` first for the reason the
        // pair above give.
        //
        // **The second's call is written `h(p) + 1`, and the `+ 1` is the whole
        // of why it guards anything.** Spelled `h(p)`, the call is enclosed by
        // nothing C leaves unsequenced, so ADR-0026's marker is emitted at its
        // own arguments and empties the carried reads a second time; measured,
        // the case then survived the removal of *either* clearing and named
        // neither, a mutation nothing fails because two rules hold it. Under
        // the `+`, the marker is suppressed and the sequence point at the end
        // of the statement before is the only thing left holding the silence.
        //
        // Mutations, each applied alone and the failure read:
        //
        // * refuse `Callee::Opaque` in `used_before` again. The first fails on
        //   its `.stderr`, which goes empty.
        // * give the opaque case `Unproven::Unsequenced` and the call's span as
        //   `freed`. The first fails on its `.stderr`, which gains a `freed
        //   here` caret and the 6.5.2.2 p10 note, about a free no program here
        //   performs.
        // * the `Element::Sequenced` arm stops clearing the carried reads. Both
        //   fail: the second gains an `SC0402` about `g(*p)` a statement
        //   earlier, and the first gains a second one about `*p = 1`.
        an_unsequenced_use_before_an_opaque_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_use_an_opaque_call_is_sequenced_after_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A controlling expression that is exactly a dereference is read by the
        // branch itself, because it needs no temporary, so the sequence point
        // at the end of it belongs after the branch and not before. C17 6.8 p4,
        // and both arms: the body and the edge that skips it are each after the
        // condition. The third has nothing ordering it, because a `?:` below a
        // `+` is enclosed by something C leaves unsequenced, so that one
        // reports.
        a_condition_read_through_a_pointer_is_sequenced_before_the_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_condition_read_through_a_pointer_is_sequenced_before_the_other_arm: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_condition_read_through_a_pointer_is_sequenced_before_the_loop_exits: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_condition_read_through_a_pointer_in_an_unsequenced_operand_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Three shapes where asking for a sequence point would be asking the
        // wrong question, each of which review found this check getting wrong.
        // A double free runs both frees whichever order C picks, so 6.5 p3
        // settles nothing about it and the proof stands. A free already ordered
        // before an expression proves a use inside that expression, and stays
        // the one to name, however the frees inside it are ordered. And where
        // several frees are folded into one answer, the earliest span and the
        // conjunction of their orders can come from different frees, so the
        // caret that would say `freed here` is dropped rather than paired with
        // a note about a free it is not pointing at.
        a_double_free_in_one_expression_does_not_turn_on_the_order: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_already_sequenced_is_the_one_a_later_free_keeps: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_may_set_where_one_free_is_sequenced_and_one_is_not: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A sequencing operator below something C leaves unsequenced orders its
        // own parts and nothing outside them, so none of these three is a
        // proof. One case per operator, because the guard is written once per
        // operator and review measured that removing any one of them leaves the
        // suite green: the comma's was held, and `&&`, `||`, `?:` and a call's
        // arguments were not. Each mutation makes the compiler **certain**
        // about an order C has not chosen, which is the direction that matters.
        a_comma_inside_a_call_argument_orders_nothing_outside_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_logical_and_inside_an_unsequenced_operand_orders_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_conditional_inside_an_unsequenced_operand_orders_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A parameter written as an array is the pointer C17 6.7.6.3 p7 makes
        // it, so its function lowers and a read after its free is one.
        // Mutation: leave a parameter's type as written; the function is
        // `SC0304`, a type the IR cannot hold, and the read goes unasked.
        a_parameter_declared_as_an_array_is_followed_as_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // But not one whose length C17 6.9.1 p10 evaluates on entry, here a
        // free that makes the read of `*p` a use after free: refused as before
        // parameters were adjusted, until #382 evaluates it. Mutation: lower
        // every adjusted parameter; this exits 0 with its free gone.
        a_parameter_whose_array_declarator_is_lost_by_adjusting_it_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The four constructs that do order their operands, one case each,
        // because C17 Annex C names four and a list implemented three-quarters
        // of the way leaves a reader asking which quarter. 6.5.17 p2, 6.5.13
        // p4, 6.5.14 p4 and 6.5.15 p4 in that order, and each is a proof rather
        // than a suspicion.
        a_comma_sequences_a_free_before_a_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_logical_and_sequences_a_free_before_a_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A read through a `void *` is `void` and is still a read: the
        // lowering discards it where it stands, as a comma's left operand or
        // on the arm of a `void` `?:` that reads it, rather than dropping it
        // for its type. Mutation: have `discard`, `second` and `merge` ask
        // `is_void` rather than `pushes`; the first two go silent, and the
        // third is reported on both arms at the whole conditional.
        a_void_read_as_a_commas_left_operand_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_void_read_on_an_arm_of_a_void_conditional_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_void_read_on_one_arm_is_read_on_that_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_logical_or_sequences_a_free_before_a_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_conditional_sequences_a_free_before_a_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_value_read_after_it_was_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_use_after_a_free_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_dereference_of_a_pointer_with_no_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_that_is_not_known_to_allocate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other half of reading the callee's name: a call that allocates is
        // also a call that does not touch what it was handed, and the case
        // above says nothing about that.
        //
        // **`malloc` is declared taking a pointer, and that is the case rather
        // than an accident of it.** This check recognises an allocation by the
        // name and never by the type, so the declaration is free to take
        // whatever reaches a site, and in this subset only a pointer does.
        // Written with C's own prototype the call does not type-check: `clang
        // -std=c17 -pedantic-errors` reports `incompatible pointer to integer
        // conversion` for `malloc(q)` against `int`. It accepts this one with
        // `-Wincompatible-library-redeclaration`, which every case here
        // declaring `malloc` already draws.
        //
        // Mutation: have `Callee::Allocates` poison its arguments as
        // `Callee::Opaque` does. The site `q` holds becomes `Unknown` at the
        // middle call and this fails on its `.stderr`, which gains a
        // `warning[SC0401]` on the first free.
        an_allocating_call_leaves_its_argument_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_subscript_of_a_freed_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_constant_subscript_of_a_freed_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same program with `&p` in it, which is the pair that says the
        // fold aligned the two spellings rather than only changing one. The
        // subscript used to be proved here and the dereference never was:
        // taking `p`'s address makes it unprovable under ADR-0017, and the
        // subscript reached a proof only by arriving as a shape that rule did
        // not see. Aligning them costs a proof, and ADR-0021 says why that is
        // the right direction.
        a_subscript_of_an_escaped_pointer_is_suspected_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        two_pointers_used_after_a_free_on_one_line: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        one_pointer_used_after_a_free_on_two_lines: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_freed_pointer_read_in_an_argument: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_either_of_two_locals_names_no_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_one_of_two_allocations_by_name: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_a_may_set_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Silent, and for two reasons since ADR-0027: the local forgot the set
        // it freed, and `p = 0; free(p);` is a call C defines as doing nothing.
        // Which of the two is working can no longer be read off this case, so
        // the forgetting is asked of the lattice directly by
        // `a_local_given_a_constant_forgets_the_set_it_freed` in
        // `crates/safec-ir/tests/freed.rs`, where the assignment is a constant
        // this subset cannot write.
        a_local_given_nothing_forgets_the_set_it_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_may_set_freed_then_written_through_an_alias: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a proof about a may-set survives when a value is built from its
        // operands, which is the half of `Held` that is not a may-fact. It
        // carries while the set does not grow, and an offset by an integer does
        // not grow it: C17 6.5.6 p8 keeps the result inside the object the
        // *pointer* operand points into, whatever the index happens to be. `i`
        // is a parameter and so a site, and the result reaches it no longer.
        // See ADR-0024 for the proof's rule and ADR-0030 for which operands it
        // counts.
        a_free_after_an_offset_that_kept_the_set_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_offset_by_an_integer_parameter_keeps_the_proof: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same offset by an integer that is not a site at all, which is the
        // other half of that: the two cases differ in whether there was
        // anything to pick up, and neither picks it up. A set that *has* grown
        // loses the proof still, and no C this frontend accepts writes one, so
        // `an_offset_by_a_second_pointer_loses_the_proof` holds that from
        // hand-built IR instead.
        an_offset_by_an_integer_local_keeps_the_proof: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the may-fact beside the proof. A local that lost the name for
        // what it held says so, and arithmetic on it builds a pointer that has
        // lost it too. ADR-0018 holds that for a copy and nothing held it for
        // arithmetic, so the union the accumulator does of that bit could be
        // deleted with the suite green.
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
        // `free(p); p = 0;`, the commonest hygiene C has, and silent for the
        // two reasons the case above is.
        // `a_local_given_a_constant_forgets_the_site_it_held` is where the
        // lattice half is asked on its own.
        a_pointer_set_to_nothing_after_a_free_holds_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The free-and-null idiom: on the arm that freed it, nothing read
        // again holds the allocation, so the join takes the other arm's live
        // state. See ADR-0048. Built: returned, read after a null test, and
        // inside a loop. Mutation: have the join join every site as before;
        // all three are refused.
        a_pointer_freed_and_set_to_null_on_one_arm_is_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_freed_and_set_to_null_on_one_arm_is_read_after_a_null_test: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_freed_and_set_to_null_in_a_loop_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Doubted, where something read again still holds it: the pointer
        // itself with no null, a copy, a copy freed after, a slot of memory,
        // and the pointer read back through its address. Mutation: have
        // `live_in` leave out a local whose address is taken; the last builds.
        a_pointer_freed_on_one_arm_and_returned_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_copy_of_a_pointer_freed_and_set_to_null_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_copy_freed_after_a_free_and_null_on_one_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_stored_before_it_was_freed_and_set_to_null_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_freed_on_one_arm_and_read_back_through_its_address_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And where what holds it on the arm that freed it is of one kind and
        // the other arm holds it by another, which is the only shape where a
        // kind `held_sites` leaves out changes the answer: held on neither
        // side, the states join as before. Mutation: have `held_sites` leave
        // out `inside`; the first builds. Mutation: leave out locals' sites;
        // the second builds.
        a_pointer_stored_on_the_arm_that_freed_it_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_copy_made_on_the_arm_that_freed_it_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer that is a freed allocation on one path and a local's
        // address on the other is doubted, not proved: the path through `&x`
        // reads `x`. Under `--allow-unknown`, so a proof would be exit 1 and a
        // doubt is exit 0. Found by review of ADR-0048, whose join made the
        // first a proof; the second was one on `main`. Mutation: have
        // `Known::reached_by` leave out the `Partial` beside a local's
        // address; both become proofs.
        a_pointer_to_an_allocation_freed_on_one_arm_or_to_a_local_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_to_an_allocation_or_a_local_freed_after_the_join_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // `*p || (free(p), *p)`: the left read is live and says nothing, the
        // right is a proof at its own span. Mutation: give the operands'
        // writes the whole `||` span again; the proof moves onto the whole
        // condition.
        a_live_read_and_a_freed_one_in_one_or_are_reported_apart: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The left read is doubted, since a call the check cannot read was
        // handed `p`, and the right is proved after the free; each at its own
        // operand. Mutation: give the operands' writes the whole `||` span
        // again; the two fold into the proof at one caret.
        an_unproven_read_and_a_freed_one_in_one_or_are_reported_apart: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `p`'s address escaped, so both reads of `*p || (free(p), *p)` are
        // doubted, each at its own operand. Mutation: give the operands'
        // writes the whole `||` span again; the two fold into one report.
        an_escaped_local_read_twice_in_one_or_is_reported_twice: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The two reads of `*p || (q = &p, *p)` are reported where each is
        // written: the left proved, the right doubted once `p`'s address is
        // taken. Mutation: give the operands' writes the whole `||` span
        // again; the two fold into the proof at one caret. Which of a pair at
        // one caret stands is held as IR in `crates/safec-ir/tests/freed.rs`.
        a_freed_read_and_an_unproven_one_in_one_or_are_reported_apart: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // `*&p` is `p`, the other half of the same clause. Mutation: in
        // `begin_place`, drop the fold and always build the `&`; both go
        // silent.
        a_freed_pointer_returned_through_its_own_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_freed_pointer_read_through_its_own_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_freed_pointer_written_through_its_own_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_dereference_after_a_comma_in_a_condition: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The two reads of `c ? (free(p), *p) : *p` share the conditional's
        // caret: the first doubted, since `helper` was handed `p`, and the
        // second proved after the free. Mutation: have `supersedes` answer
        // `false` for `(Unknown, Unsafe)`; the proof is reported as a doubt.
        // Narrowing an arm's write to the arm moves this case.
        a_proof_in_one_arm_of_a_conditional_replaces_a_suspicion_in_the_other: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Each function underlines only the operand whose value decides,
        // after a free, at every place a condition or a `&&` or `||` operand
        // is recorded: the comma under `if`, `while`, `for` and `?:`, a chain
        // of two commas both ways round, and each side of a short circuit.
        // `c, c, *p` groups to the left and reaches `*p` in one step, so
        // `if_nested` writes the chain to the right, which is the one that
        // needs following. `if_left` pins that
        // a comma's left operand is underlined by its own read, since its
        // branch reads `c` and reports nothing, so no mutation of an origin
        // moves it. Mutation: take the whole controlling expression again at
        // one of the `if`, `while` or `for` sites, or the `?:` branch; that
        // function's row widens. Mutation: take a comma's left operand; the
        // comma rows move onto `c`. Mutation: unwrap one comma only, `if let`
        // for `while let` in `decides`; `if_nested` widens to `c, *p`, the
        // inner comma, since parentheses make no node.
        // Mutation: give the left or the right operand's write the whole
        // binary span again; `and_left` or `or_right` widens.
        a_condition_underlines_the_operand_that_decides: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_subscript_in_a_condition_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_dereference_in_a_conditional_expression_after_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        two_allocations_are_freed_once_each: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_replaced_before_it_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_this_check_cannot_read_between_two_frees: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call this check cannot read does not un-free what it is handed,
        // so the use after it is the proof it was before the call. Mutation:
        // in the `Callee::Opaque` arm, write `SiteState::Unknown` over what an
        // argument names outright; this and the case above become "may".
        a_free_proved_before_a_call_it_is_handed_to_stays_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A live allocation a call is handed may be freed by it, even where
        // every holder was handed to the same call by address, which exempts
        // it from what a call does to a holder out of its reach. What the
        // call is handed is the one place that says so. Mutation: in the
        // `Callee::Opaque` arm, start what the call may have freed empty
        // rather than from what it is handed; the doubt at `use2(&a)` goes.
        an_allocation_handed_to_a_call_beside_its_holders_address_is_doubted_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_through_a_pointer_the_check_does_not_follow: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two programs with one `free` each, and the boundary between two
        // reasons a report can be unproven. In the first nothing established a
        // free at all, so the diagnostic says what this check lost rather than
        // that the value may have been freed already: `*pp` is a pointer this
        // check follows locals rather than the targets of. The second reaches
        // the same caret with a site an opaque call was handed, where `helper`
        // may really have freed it, and it keeps the older words. It is the
        // only case whose suspicion rests on nothing but the callee: every
        // other program that keeps those words has a `free` in it that this
        // check saw. Answering `Unproven::Lost` where the sites disagree fails
        // it, along with everything else that keeps them.
        //
        // **There were three, and the third has moved down beside the null
        // constant.** `int *p = 0; free(p);` was here to hold the words a lost
        // pointer gets, on the grounds that a local given a constant holds no
        // site. ADR-0027's exemption now answers that program before the words
        // are reached, so it says nothing at all and belongs with the other
        // spelling of it.
        a_free_read_out_of_another_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_after_a_call_this_check_cannot_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the one this check is meant to say nothing about at all. C17
        // 7.22.3.3 p2: "If `ptr` is a null pointer, no action occurs." So a
        // null constant handed to `free` is written on purpose and is not a
        // pointer this check lost. Nothing held that until this case: making a
        // constant argument answer `Reached::Lost` puts a warning on a program
        // C defines, and fails this case and nothing else in the suite.
        //
        // It pins the null half only. `Allocations::touching` skips every
        // constant, and the clause above supports it for this one, so
        // `free(17)` stays silent and this case does not say otherwise. The
        // comment on that arm says where the other half belongs.
        a_free_of_a_null_constant: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same program with the constant given a name first, which the
        // clause does not distinguish and `clang` accepts identically. It is
        // silent because the nullability check established the pointer is null
        // and `asked` leaves such an argument out, which is ADR-0027; dropping
        // that filter puts `this frees a pointer this check stopped following`
        // back on a program C defines and fails this case.
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
        // The second is asked where the free runs. `p` is established null at
        // the entry of the block that frees it and is not null by the time the
        // free is reached, so recording the row before the block's elements
        // rather than after exempts a proved double free of `q`'s site.
        // Measured: each mutation takes its own case to exit 0 and leaves the
        // other reporting. And the argument the exemption is not allowed to
        // read at all. `pp` is established null and `*pp` is a question about
        // what it points at, which the nullability lattice is keyed by the
        // local and cannot ask; answering it with `pp`'s own nullness takes the
        // `SC0401` off this case and leaves the `SC0403` alone. Measured:
        // dropping the `projection.is_empty()` guard in `established_null`
        // fails this case and nothing else in the suite. And the local the
        // exemption may not call null at all, because it does not hold a
        // pointer. `nullness_of` answers `Null` for a constant zero without
        // asking what it is assigned to, which costs nothing where the answer
        // is only read about a dereference; read to exempt a free, it took the
        // `SC0401` off this program and left nothing in its place. C17 6.3.2.3
        // p3 makes a null pointer constant an integer constant expression
        // converted to a pointer type, and an `int` lvalue holding zero is
        // neither. `clang` refuses this program under 6.5.2.2 p2, which this
        // compiler does not do yet and #154 is about; until it does, what it
        // should not do is go quiet. And the free C has not ordered the
        // assignment before. The operands of `+` are unsequenced, C17 6.5 p3,
        // so on the order that runs the right one first this frees the pointer
        // the line above already freed. The nullability lattice has no notion
        // of order and says so; the marker that does is
        // `Element::ArgumentsEvaluated`, which ADR-0026 emits only where no
        // unsequenced operator encloses the call, and its absence here is what
        // refuses the exemption. Dropping that term reports nothing at all
        // about a double free this check watched, which is saying safe wrongly,
        // the worst answer `docs/safety-model.md` says this compiler can give,
        // and fails this case.
        a_free_in_an_unsequenced_operand_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same defect where the block holding the free has no elements at
        // all. A call ends a block, so the `g(0)` between the two operands puts
        // the write in one block and the free at the terminator of the next,
        // and that block carries neither a marker nor anything else. The rule
        // reads `elements.last()`, so this is the `None` arm, and it is the
        // only case that reaches it: treating `None` as ordered leaves this
        // program silent about the double free and fails nothing else.
        a_free_in_an_unsequenced_operand_across_a_call_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An `int` holding zero is not a null pointer constant, so this free
        // is not exempt, and C says so before any analysis: passing an `int`
        // for `free`'s `void *` is a constraint violation, `SC0302` since
        // #356, and the driver runs no check on a tree the type check
        // reported. Mutation: drop the gate after names and types in
        // `driver.rs::analysed`; the memory check's `SC0401` returns.
        a_free_of_an_int_that_holds_zero_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer that may be a local's address frees no allocation on that
        // path, C17 7.22.3.3 p2, and the join kept only the other path's
        // sites; so does a `realloc` of one. Mutation: drop
        // `may_be_a_locals_address` from `Allocations::touching`; every case
        // below fails, the first three going silent at a free or a
        // `realloc`. Mutation: drop it from
        // `Allocations::holds_something_unnameable`; the second free in
        // `a_free_after_one_that_may_have_freed_a_local_is_not_proved` is
        // proved a double free the path that took `&x` does not commit.
        a_free_of_a_pointer_that_may_hold_a_locals_address_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Why a pointer was lost, where two reasons meet at one free: a local's
        // address on one path and what memory held on the other says the
        // first, since that path frees what C forbids whatever the other
        // holds; a pointer never followed meeting one read out of memory says
        // neither (#213). Mutation: fold `MayBeALocal` into `Other` with the
        // rest in `LostReason::joined`; the first says nothing is wrong.
        // Mutation: keep the later reason rather than `Other` where two
        // disagree; the second names it. Keeping the earlier one instead
        // leaves the second as it is, since its earlier reason is `Other`,
        // and is held by
        // `a_free_of_a_pointer_read_out_of_a_parameter_doubts_a_read_through_the_parameter`.
        a_free_that_may_be_of_a_local_says_so_whatever_else_it_may_be: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_lost_for_two_reasons_at_once_names_neither: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_a_conditional_over_a_locals_address_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_realloc_of_a_pointer_that_may_hold_a_locals_address_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_after_one_that_may_have_freed_a_local_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_read_out_of_a_pointer_proved_null: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_proved_null_before_its_address_escaped_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_that_stopped_being_null_before_the_free_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A free of a pointer that is not the start of its allocation, which
        // C17 7.22.3.3 p2 makes undefined and which this check once followed to
        // the allocation and said nothing about. See ADR-0036. The first three
        // are the proof: a constant offset either way, and the increment the
        // issue that found this was written about. The fourth is the offset
        // this check cannot evaluate, which is unproven and so an error in this
        // compilation.
        a_free_of_a_pointer_past_the_start_of_an_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_a_pointer_an_increment_moved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_a_pointer_before_the_start_of_an_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_a_pointer_offset_by_an_integer_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The join, both ways. Two paths that each moved the pointer off the
        // start are both off it, whatever the distances; one that did not
        // leaves the answer open. `+` with the constant on the left, which C17
        // 6.5.6 p8 makes the same addition. Mutation: in
        // `memory/built.rs::offset_of`, drop the arm that reads the constant on
        // the left. This fails with the proof down to `may`.
        a_free_of_a_constant_plus_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer moved off the start and back is at the start again, and
        // this check carries no distance to know it, so the second move proves
        // nothing. The program is one C defines. Mutation: in `offset_of`, stop
        // requiring the followed operand to be at the start. This fails with a
        // proved `SC0404` about a free C defines.
        a_free_of_a_pointer_moved_back_to_the_start_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two allocations, one offset, and no `allocated here`: naming either
        // line would be a caret on an allocation the value may not hold. Both
        // are made before the branch rather than on its arms: allocated on an
        // arm, each site meets the other arm's `Live(None)` at the join and
        // arrives with no line to name, so the fold has nothing to get wrong.
        // Mutation: in `memory/report.rs::interior`, fold `made` by keeping the
        // first site's, or by keeping the last site's. Each fails on the label,
        // and both directions are measured because a fold has a wrong version
        // on each side and a case can hold only some of them.
        a_free_past_the_start_of_either_of_two_allocations_names_neither: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_offset_on_both_arms_is_still_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_offset_on_one_arm_only_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The two that must stay silent. The first is ADR-0021's fold reaching
        // this check as no arithmetic at all, and guards that record rather
        // than anything here. The second is what a local holding no site
        // answers, on one of the commonest shapes in C: a path that holds null
        // meeting one that allocated.
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
        // And `*(pp + (1 - 1))`, whose offset is a constant expression rather
        // than the literal: it lowers to the constant it is, so the zero is
        // folded as `pp[0]`'s is (ADR-0021) and the use after free is seen.
        // Mutation: lower a constant expression's operands rather than its
        // value; this goes silent.
        a_constant_zero_offset_is_the_write_it_is_defined_as: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_zero_added_to_an_integer_keeps_its_operation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_through_a_pointer_plus_zero_on_the_left: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_through_a_pointer_minus_zero: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_address_taken_on_one_arm_is_written_through_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same shape as the case above, on a program C defines on both
        // paths: the other arm gives `pp` a pointer this check cannot follow
        // rather than a null one it would be undefined to write through. A
        // guard for a soundness rule should not rest on a program C has already
        // given up on.
        a_write_through_a_pointer_with_one_target_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A write through a pointer whose own address escaped may land
        // anywhere, whichever order the two happen in. The second case is the
        // one that says the answer cannot be recorded on the pointer's own row:
        // `pp = &p` after the escape gives `pp` a fresh row, and a fact written
        // there would have gone with the old one. See ADR-0028.
        a_write_through_a_pointer_whose_own_address_escaped: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_through_a_pointer_whose_address_escaped_before_it_was_given_its_target: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call this check cannot read may write a fresh pointer through any
        // address that has escaped, so a `free` afterwards cannot say which
        // allocation it took. Both of these are programs C defines and both
        // were an `error` at exit 1, reported against a **sharer**: what the
        // escape already took away is the report about the escaped local
        // itself, so a case that frees or reads through that local alone
        // observes nothing. A guard whose only observer is that report is
        // vacuous, and ADR-0029 is the rule.
        a_call_this_check_cannot_read_may_have_replaced_what_an_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_certain_write_does_not_survive_a_call_this_check_cannot_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What that rule costs, kept where it can be seen: a double free
        // through a sharer that `main` proved before the call was allowed to
        // replace what `p` holds. It is a warning here because the callee may
        // have.
        a_double_free_through_a_sharer_after_an_opaque_call_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A free cannot un-free an allocation, so a free this check could not
        // follow has nothing to say about a site an earlier free it *could*
        // follow already proved. Writing `Unknown` over that site anyway threw
        // the proof away, and the site is shared, so what lost it was the
        // sharer's report. Found by review.
        a_free_this_check_could_not_follow_leaves_a_proved_free_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other half of the trade the rule makes. The downgrade is only
        // acceptable because the flag still fails the build, and this is the
        // case that says so: same program as the sharer case above, one flag
        // on.
        a_double_free_a_call_took_the_proof_of_still_fails_a_build_that_denies_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the direction the cheap versions of that rule fail in. Clearing
        // the escaped local's row at the call instead leaves this program with
        // nothing to say about the read, which is saying safe wrongly, while a
        // false positive is only a false report the reader can see. See
        // ADR-0029.
        a_use_after_free_through_an_escaped_local_is_still_reported_after_an_opaque_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the other order, which keeps the rule from being the whole of
        // what an escape means: a call that ran **before** the address escaped
        // cannot have written through it, so the proof survives. A guard for a
        // rule about two events holds only the order it was written in.
        an_opaque_call_before_the_escape_leaves_the_proof_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same rule through the door ADR-0029 named and declined: the
        // writer is a write in this function rather than a callee. Both of
        // these were an `error` at exit 1 about a program C defines, and both
        // report through a sharer, because a case observing only the escaped
        // local would observe nothing. The second is here although one mutation
        // fails both, because the two are the ways a pointer gets out of this
        // check's sight: through a parameter, and through a chain of locals
        // with no call and no parameter in it. A fix keyed on either shape
        // alone passes the other. See ADR-0031.
        a_write_through_a_pointer_this_check_cannot_follow_may_have_replaced_what_an_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_that_replaces_what_an_escaped_local_holds_needs_no_call_and_no_parameter: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other half of the trade, as ADR-0029's own flag case says it for
        // the call: the downgrade is only acceptable because the flag still
        // fails the build. Same program as the first case above, one flag on.
        a_use_after_free_a_write_took_the_proof_of_still_fails_a_build_that_denies_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What keeps that rule from costing everything, and the only case that
        // holds it: a write through a pointer to an `int` cannot put a pointer
        // anywhere, so the escaped local keeps what it held and the double free
        // below stays proved. Marking every escaped local instead drops this
        // `error[SC0401]` to a warning and exit 1 to exit 0.
        a_write_through_a_pointer_to_an_int_leaves_what_an_escaped_local_holds_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The half of the door that a write with no target at all does not
        // reach: one named target, and a path on which the pointer was never
        // given one, so the write may land beside that target in an escaped
        // local the union says nothing about. Narrowing the rule to an empty
        // target set leaves this program an `error` at exit 1.
        a_write_that_may_land_beside_its_target_may_have_replaced_what_an_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The first case above with the alias written inline instead of read
        // into a temporary, which is the same program and a place with two
        // `Deref`s. This check follows a write through exactly one, so the rule
        // has to fire for every projection it declines rather than for the
        // shape it can follow: keyed on that shape, the two spellings of one
        // program answered opposite ways. Two review lenses found it
        // independently.
        a_write_through_more_than_one_deref_may_have_replaced_what_an_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The exception C17 6.5 p7 carries, and the one write that reaches an
        // escaped local of every type: a character lvalue may access an object
        // of any type, so copying one pointer's object representation over
        // another's is defined. No cast is needed to get a `char *` that
        // aliases a pointer, because the `void *` round trip is implicit both
        // ways, and this frontend accepts it. Found by review, which compiled
        // the program with `clang -std=c17 -pedantic-errors` and ran it under
        // AddressSanitizer to show there is no use after free in it.
        a_write_through_a_character_pointer_may_have_replaced_what_any_escaped_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the write the rule must not fire on: ADR-0028's certain one,
        // which lands in its target and nowhere else, beside an escaped local
        // of the same type that keeps its proof. Firing at every write drops
        // this `error[SC0401]` to a warning.
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
        // The storage pair ADR-0012 emits, read by the memory check rather than
        // by the lowering that writes it. A local declared inside a loop is
        // given fresh storage each time round, and the only path from its
        // `StorageDead` back to it is the back edge, so this is the one shape
        // where the previous iteration's facts can still be standing.
        //
        // **The write before the assignment is what makes it visible.**
        // `Known::clear` clears the local's edge to its sites and not the sites
        // themselves, so a stale `Freed` can only be observed by a read that
        // happens before the local is given anything. `*p` on an indeterminate
        // pointer is a defect with a check of its own that does not exist yet,
        // and when that check lands this case will have something to say about
        // it: the program is chosen for a read this compiler currently answers
        // nothing about, and it stops testing this the day that check has
        // something to say.
        //
        // Mutation: both the `StorageLive` and the `StorageDead` arm of
        // `Allocations::element` to no-ops. The free from the previous
        // iteration arrives at the write and this fails on its `.stderr`, which
        // gains a `warning[SC0402]` beside the null one.
        a_scope_reentered_forgets_what_its_local_held: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_where_one_of_two_allocations_is_live: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_double_free_is_silent_at_safety_off: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
        an_unproven_free_is_silent_at_safety_off: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
        // The other side of ADR-0033, and the only cases that reach it: an
        // unproven conclusion is an error wherever a check runs, so a warning
        // needs the run to have asked for one. Without these three the arm of
        // `certainty_note` that is not promoted has no observer outside the
        // renderer's own tests.
        an_unproven_free_is_a_warning_under_allow_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        an_unproven_use_is_a_warning_under_allow_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What a run is told when the level it asked for is not the level it
        // gets, which is ADR-0035. Two independent things lower it, so two of
        // these are one cause each and two are the edges that a gate on one
        // cause alone gets wrong.
        //
        // Mutation: in `driver.rs::undelivered`, compare `options.safety`
        // against `SafetyLevel::IMPLEMENTED` rather than against
        // `Options::delivered`, which drops the artifact half. The second fails
        // and the first stays green, which is the shape of the worst defect
        // this project has had: two axes, and a gate that answers one of them.
        //
        // Mutation: answer the conclusion `Unsafe` rather than `Unknown`, which
        // is how `Diagnostic::concluded` builds an error instead. All three of
        // the reporting cases fail and the fourth is the one whose `.exit` goes
        // from 0 to 1: the conclusion is the whole of what lets
        // `--allow-unknown` reach this report.
        //
        // **The third is documentation rather than a discriminating guard, and
        // that is measured rather than hoped.** It is the only case anywhere
        // that names a satisfiable level beside an artifact that carries no
        // check, so nothing else pins the combination; but every mutation tried
        // on the gate fails it together with all 76 bare dump cases rather than
        // alone. Reporting on the artifact without asking the level fails 84
        // tests, this among them. There is no mutation that isolates it,
        // because the derived default already makes a bare dump resolve to
        // `off`, so "report above `off`" and "report when delivered differs"
        // agree on every input in the corpus.
        a_level_with_no_checks_behind_it_is_not_delivered: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        an_artifact_that_stops_before_the_ir_delivers_no_checks: ["--emit", "ast", "--safety", "memory"],
        a_level_that_was_not_asked_for_is_not_reported: ["--emit", "ast", "--safety", "off"],
        a_migrating_run_is_told_what_was_not_established_and_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "lifetime", "--allow-unknown"],
        // Both causes at once, which the two above have one each of. A review
        // found the report speaking whichever cause was written first, and its
        // remedy sending the reader to `--emit safety-ir`, which is refused
        // again for the other reason: ADR-0034 makes a remedy the change that
        // would make the program compile, so half of that one was a promise
        // this run breaks.
        //
        // Mutation: choose the note and the remedy with one `if` on
        // `options.emit.reaches_the_ir()`, as it was written. This fails and
        // the two single-cause cases stay green, which is the shape of the
        // defect rather than its size.
        //
        // Mutation: offer `--allow-unknown` whatever the level. This fails: the
        // level here is `strict` and `Cli::check` refuses that pair, so
        // following the remedy is an argument conflict rather than a build.
        both_reasons_a_level_can_go_undelivered_are_said_at_once: ["--emit", "ast", "--safety", "strict"],
        a_double_free_is_found_on_a_backend_run: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        a_discarded_dereference_reaches_the_backend: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        // `pp[0]` is `*pp`, and the backend can write `*pp`. It used to refuse
        // this with `SC0801`, because the subscript built an addition and the
        // backend cannot write pointer arithmetic; ADR-0021 folded the addition
        // away and the refusal went with it.
        // `an_ir_shape_the_backend_cannot_write` is the case that holds the
        // refusal itself, which `pp[1]` still earns.
        a_zero_subscript_reaches_the_backend: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    }

    "nullability" => {
        // The nullability check's cases, kept together for the reason the
        // memory check's are: one table, whose rows are the three-valued model
        // met by a program that proves the answer, one that leaves it unproven,
        // and one that tests the pointer and so needs neither.
        //
        // The row that proves it.
        a_null_pointer_dereferenced_is_unsafe: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // **Three tests of a pointer are three cases and not one**, because the
        // shapes reach the check differently: `if (p)` hands the pointer's own
        // place to the terminator, whatever wrote it above, and `p != 0`
        // and `p == 0` put a comparison above it that has to be read back
        // through the block. A reader of the C cannot tell those apart, which
        // is why each is held.
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
        // roadmap's example is a program this compiler has something to say
        // about.
        an_allocation_dereferenced_without_a_test_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The address of a local is the only thing this check can prove not
        // null, and a store through a pointer that may hold that address is
        // what takes the proof back. Without it this program said nothing at
        // all, while `p` was provably null at the write:
        // `docs/safety-model.md`'s worst answer, found by review. One case for
        // one rule: a callee handed `&p` is the same escape through a different
        // door.
        a_pointer_written_through_its_own_address_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A constant condition decides its branch where it is lowered (#338).
        // The first is the direction a wrong fold goes silent in: the read
        // after an `if (0)` is reached and asked. Mutation: have
        // `lowering::decided` take the `then` arm where the constant is zero;
        // the first goes silent. Mutation: have it always build a `Branch`;
        // the other two are reported, from code no execution reaches.
        code_after_an_if_on_zero_is_still_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        code_under_an_if_on_zero_is_not_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_loop_on_zero_is_never_entered: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Code after a call that does not return is reached by nothing, and a
        // function defined here is read rather than believed by its name
        // (ADR-0051). Mutation: have the lowering give every call its
        // continuation; the first is reported. Mutation: have
        // `Lowering::does_not_return` ignore `defined`; the second and third
        // go silent. Mutation: have it ask the IR whether the callee has a
        // body instead of `defined`; the third goes silent, because a body
        // below the call is not lowered yet. The `SC0402` beside the `SC0403`
        // in both is the memory check doubting `p` after a call whose body it
        // does not read, as it does after `g();` for any `g` defined here.
        code_after_abort_is_not_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        code_after_a_function_defined_here_named_abort_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        code_after_a_function_defined_below_named_abort_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call inside a larger full expression keeps its edge, because a
        // read written before it is lowered after it. Mutation: drop the
        // `root` test where the call decides its continuation; both reads go
        // silent.
        a_read_written_before_a_call_that_does_not_return_inside_an_expression_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `for` on zero is not entered, so what follows it is reached and
        // asked. Mutation: have the `for` site swap its arms where the
        // condition is a constant; the loop is entered and never left, and
        // this goes silent.
        code_after_a_for_on_zero_is_still_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `(x, 1)` lowers to the constant 1 and is not a constant expression
        // (C17 6.6 p3), so the loop keeps its exit and what follows it is
        // asked. Mutation: have `Lowering::constant_expression` answer `true`
        // for everything; this goes silent.
        a_loop_on_a_comma_expression_is_not_taken_for_a_constant: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A dereference past the first in one place reads a pointer out of
        // memory, which has no row in the lattice, and is asked as unproven
        // (#333). The first is the program that built in silence, written
        // once as a read and once as an operand. Mutation: have
        // `nullability::report` ask the root local alone, which is the code
        // before #333; the first two fail and go silent.
        a_null_pointer_read_two_dereferences_down_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_two_dereferences_down_through_a_parameter_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What that costs, pinned: a test of `*pp` is kept nowhere, so the read
        // through it in place stays a doubt, and the remedy says so rather
        // than asking for the test the program already has. Copying the
        // pointer into a local and testing it is what proves it, compared
        // with `q != 0` or tested as `if (q)`, which are the two cases after
        // it and the remedy's promise.
        a_test_of_a_pointer_in_memory_does_not_settle_a_read_through_it_in_place: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_memory_into_a_local_and_compared_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_memory_into_a_local_and_tested_is_not_null_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `if (q)` right after a copy into `q` tests the pointer the copy put
        // there, as `q != 0` does. The branch was walked back to its last
        // write first, and a copy answered nothing (#334). Mutation: delete
        // the early return for a pointer condition in
        // `nullability::tested_against_null`; this and the case above it are
        // refused, along with every plain `if (p)` case, since nothing after
        // it answers a pointer.
        a_pointer_copied_into_a_local_and_tested_is_not_null_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A branch on a pointer this check has already settled refines
        // nothing, in both directions: a null pointer stays null on the arm
        // that tested it not null, and an address stays not null on the arm
        // that tested it null. Neither arm is reached. Mutation: delete the
        // settled-local guard in `Analysis::edge`; the first goes quiet about
        // a write through a null pointer, and the second reports one at a
        // write through `&x`.
        a_null_pointer_tested_and_read_through_on_the_arm_nothing_reaches_is_proved_null: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_address_compared_with_zero_is_not_null_on_the_arm_nothing_reaches: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An assignment used as a condition, `if ((q = p))`, branches on a
        // temporary copied from `q`, so the refinement has to be carried back
        // through the copy to `q`, and through `q = p` to `p`, which is what
        // each of these reads through. The comparison, the chain and the loop
        // whose call ends a block above the copy are the other three shapes
        // the lowering gives it (#336). Mutation: have
        // `nullability::copied_from` answer nothing; each of the four reports
        // `SC0403`, and so does the store case after them. Mutation: have it
        // stop after the first copy; the first three report it at `*p`, and
        // so does the store case.
        a_pointer_assigned_in_a_condition_is_not_null_on_the_taken_arm: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_in_a_comparison_against_zero_is_not_null_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_chain_of_assignments_in_a_condition_refines_every_pointer_in_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The memory check's two reports here are about `next`, a call it
        // cannot read, and are not this case's subject: what it holds is that
        // nothing says `*q` may be null.
        a_pointer_a_loop_assigns_from_a_call_and_tests_is_not_null_inside: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other arm of each: where the branch learned the pointer is null,
        // so is every local it was copied from, and a read through `p` there
        // is proved a null dereference. The double free is the same fact as
        // the memory check reads it, where a pointer proved null is a `free`
        // it exempts. Mutation: have `Analysis::edge` give the sources the
        // `then` arm's answer on both arms; the read through `p` in the first
        // case and the double free go quiet, and `*q` under `== 0` is reported
        // as a null dereference. Mutation: refine the sources on the taken arm
        // only; the first case's read is reported as unproven rather than
        // proved, and `*q` under `== 0` is doubted.
        a_pointer_assigned_in_a_condition_is_null_on_the_arm_not_taken: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_in_an_equality_with_zero_is_null_where_it_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_freed_twice_after_an_assignment_in_a_condition_is_not_exempt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A comma puts a sequence point between the assignment and the copy
        // the branch reads, and the walk steps over it. Mutation: have
        // `nullability::copied_from` stop at a marker; `*p` is reported.
        a_pointer_assigned_before_a_comma_is_not_null_on_the_taken_arm: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*s = 1` is a store through a pointer in the same block, above the
        // copies, so it cannot have changed the value they took. Mutation:
        // have `nullability::copied_from` answer nothing at such a store
        // rather than what it found below it; this reports `SC0403` at the
        // `return`.
        a_store_through_a_pointer_before_an_assignment_in_a_condition_keeps_the_refinement: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*p` proves `p` not null before the branch on `q`, a copy of it, so
        // the arm on which `q` is null is one no execution reaches, and what
        // `p` was proved stays. Only the read in the condition is doubted.
        // Mutation: have `Analysis::edge` refine a source this check has
        // settled; `return *p` is reported as a null dereference.
        a_pointer_read_through_before_a_copy_of_it_is_tested_keeps_what_the_read_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*p && **q` doubts `p` plainly and `q` through memory, each at its
        // own operand since #147, so each is told the remedy that settles it.
        // Mutation: give the operands' writes the whole `&&` span again; the
        // two fold into one report at one caret. The fold's own rule, that the
        // remedy through memory is the one kept, is held by
        // `a_doubt_through_memory_beside_a_plain_one_at_one_caret_keeps_its_remedy`
        // in `crates/safec-ir/tests/nulls.rs`.
        a_doubt_through_memory_beside_a_plain_one_is_told_its_own_remedy: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*p` and `**q` in the two arms of a `?:` share the conditional's
        // caret, and the remedy kept is the one through memory whichever arm
        // holds it. Mutation: have `Asked::joined` keep the first question,
        // or the second; one of the two functions takes the plain remedy.
        // Narrowing an arm's write to the arm moves this case.
        a_doubt_through_memory_in_either_arm_of_a_conditional_keeps_its_remedy: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The root's own answer still counts, and a proof outranks the doubt
        // beside it. Mutation: answer only the deeper question whenever there
        // is one; this fails as unproven where it expects proved.
        a_null_root_beside_a_pointer_read_out_of_memory_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The two that hold ADR-0025, and the second is the one that says the
        // fact is per path: the arm that skips the dereference joins back in.
        a_pointer_dereferenced_twice_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_dereferenced_on_one_arm_is_not_proved_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // One element that dereferences two pointers, one proved null and one
        // nothing is known about. The words do not name the value, so the two
        // cannot be told apart at one caret and only the worse is said.
        a_proved_null_dereference_beside_an_unproven_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same pair with the worse one written second, which is what tells
        // "the worst of them" apart from "the first of them": the places an
        // element dereferences are collected destination first, so the case
        // above has them agreeing and only this one does not.
        a_proved_null_dereference_after_an_unproven_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call reading through a pointer it then assigns to. What this pins
        // is the answer rather than the rule behind it: a call's result lands
        // in a fresh temporary here, where the callee returns something, and is
        // copied out in an element of its own, so the order the arguments and
        // the destination are applied in cannot be seen from any C this
        // frontend lowers.
        // `a_call_that_reads_a_pointer_and_writes_it_keeps_neither` in
        // `crates/safec-ir/tests/nulls.rs` is what holds that.
        a_call_whose_destination_it_dereferences: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_null_dereference_is_silent_at_safety_off: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
        an_unproven_dereference_is_a_warning_under_allow_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
    }

    "nonnull" => {
        // `_Nonnull`, the first annotation: believed by the body, checked at
        // every call, refused everywhere else. See ADR-0037, whose Confirmation
        // names the mutation each of these fails under.
        //
        // One case per stage the annotation passes through, because a feature
        // guarded at one stage is unguarded at every stage that reads it: the
        // tree, a declaration's IR, and a definition read by the analysis.
        a_nonnull_parameter_is_read_into_the_tree: ["--emit", "ast"],
        // And on the pointer a function declared at file scope returns, which
        // ADR-0050 opened. It was refused here until it had a meaning.
        a_nonnull_on_a_return_type_is_read_into_the_tree: ["--emit", "ast"],
        // The parameter's own pointer is the last one written, not the first.
        // Mutation: have `parameter_list` read `derivations.first()`, which
        // drops this annotation in silence; only this case fails.
        a_nonnull_after_the_last_star_of_a_parameter_is_read: ["--emit", "ast"],
        a_nonnull_parameter_of_a_declaration_reaches_the_ir: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_declared_nonnull_is_dereferenced_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What the body stopped carrying, the caller carries.
        a_null_constant_passed_to_a_nonnull_parameter_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_proved_null_passed_to_a_nonnull_parameter_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_nothing_established_passed_to_a_nonnull_parameter_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_nothing_established_passed_to_a_nonnull_parameter_is_a_warning_under_allow_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        each_argument_to_a_nonnull_parameter_is_asked_about_on_its_own: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `f`'s IR function takes its parameters from the prototype, not from
        // the `int f();` before it, so `f(0)` is held to `_Nonnull`, as it is
        // with the declarations the other way round, and so is a call written
        // before the prototype, and so is one after a later `int f();`.
        // Mutation: build the IR function from the declaration in hand in
        // `Lowering::declare_one` rather than from `standing_signature`; the
        // first two exit 0, and the disagreement case below prints the first
        // prototype's IR. Mutation: publish the latest declaration in
        // `Resolution::declared` rather than the standing one; the third
        // exits 0.
        a_nonnull_parameter_declared_after_a_declaration_without_one_is_held: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_written_before_the_prototype_is_held_to_its_nonnull_parameter: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_nonnull_parameter_is_held_after_a_later_declaration_without_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `g(*pp)` asks two questions at one caret: whether `pp` is null, and
        // whether what it holds is. Mutation: skip an argument with a
        // projection in `report_arguments`; only this case fails, losing the
        // `SC0405`.
        an_argument_read_through_a_pointer_is_asked_about_as_a_dereference_and_as_an_argument: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The two ways a caller discharges it, which are what keep the check
        // from reporting every call.
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
        a_nonnull_on_a_parameter_of_a_function_pointer_is_refused: ["--emit", "ast"],
        a_nonnull_on_a_parameter_of_a_block_scope_function_is_refused: ["--emit", "ast"],
        // C17 6.7.6.3 p8 adjusts a parameter of function type to a pointer, so
        // it declares no function, and the label must not say it does.
        // Mutation: choose the block-scope label on `own` alone; only this case
        // fails.
        a_nonnull_on_a_parameter_of_a_parameter_that_is_a_function_is_refused: ["--emit", "ast"],
        // The disagreement `lowering.rs::agree` refuses, both ways round.
        declarations_that_disagree_about_nonnull_are_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_definition_that_disagrees_with_a_later_declaration_about_nonnull_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: compare only the first parameter in `agree`; only this
        // fails.
        declarations_that_disagree_about_a_later_parameter_are_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `void g();` declares no parameters, so it cannot be what the rest
        // agree with. Mutation: have `declare_one` call `agree` for `()` as
        // well; only this case fails, and it goes silent.
        an_unprototyped_declaration_does_not_stand_in_for_the_first_prototype: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The promise on a declaration and not on the definition. The body is
        // built from the definition, so it believes nothing and its dereference
        // is reported beside the refusal.
        a_declaration_that_says_nonnull_where_its_definition_does_not_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    }
    "nullable" => {
        // `_Nullable`, the other specifier, and what level 5 does with a
        // pointer that writes neither. See ADR-0050, whose Confirmation names
        // the mutation each of these fails under.
        //
        // Read where `_Nonnull` is read, and into the tree. Mutation: have
        // `dump_declaration` print nothing for a specifier; this fails.
        a_nullable_parameter_is_read_into_the_tree: ["--emit", "ast"],
        // Below level 5 it restricts nothing: a `_Nullable` parameter is what
        // an unannotated one is. Mutation: lower `_Nullable` to a promise in
        // `Lowering::signature`, as `_Nonnull` is; the first goes silent and
        // the second is refused with `SC0405`.
        a_nullable_parameter_is_doubted_by_its_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_null_passed_to_a_nullable_parameter_is_accepted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Refused where `_Nonnull` is refused, which `parser.rs::placed` and
        // `Parser::core` answer for both alike.
        a_nullable_on_a_local_is_refused: ["--emit", "ast"],
        a_nullable_on_a_pointer_inside_a_parameter_is_refused: ["--emit", "ast"],
        // `clang` refuses this pair as two that conflict. Mutation: have
        // `Parser::core` ask only for `_Nonnull` again; the second becomes a
        // name and the refusal says something else.
        two_nullability_specifiers_on_one_pointer_are_refused: ["--emit", "ast"],
        // The two `clang` also accepts in C, refused as annotations this
        // compiler does not read. One after a `*`, one where a return's
        // pointer is, and one not after a `*`, which `Parser::core` answers.
        // Mutation: have `Parser::unread_specifier` answer `false`; each is
        // refused as a name instead, with `SC0201`.
        an_unspecified_nullability_is_refused: ["--emit", "ast"],
        a_nullable_result_specifier_is_refused: ["--emit", "ast"],
        an_unspecified_nullability_not_after_a_star_is_refused: ["--emit", "ast"],
        // Three answers, compared as written. Mutation: compare whether a
        // specifier was written rather than which, in `Lowering::agree`; the
        // first goes silent. Mutation: compare what the lowering carries; the
        // second goes silent, since `_Nullable` and nothing lower alike.
        declarations_that_disagree_between_nonnull_and_nullable_are_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        declarations_that_disagree_between_nullable_and_nothing_are_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A specifier on the pointer a function returns, read at every level.
        // Mutation: have the `Item::Function` arm of `dumps.rs::dump_item`
        // print nothing for `Function::return_nullability`; this fails.
        a_nullable_return_of_a_definition_is_read_into_the_tree: ["--emit", "ast"],
        // Accepted only on the derivation just before a declared function's
        // own, which is the pointer it returns. A pointer inside that one is
        // not, the pointer under a pointer at file scope is not a return at
        // all, and the pointer a function pointer's function returns is not a
        // declared function's, so nothing could check a call against it.
        // Mutation: let `Parser::placed` accept any derivation before the last
        // of a function; the first goes silent. Mutation: let it accept the one
        // before the last whatever the last is; the second does. The third
        // needs both at once, since its last derivation is a pointer and its
        // specifier is not the one before it.
        a_nonnull_on_a_pointer_inside_a_return_is_refused: ["--emit", "ast"],
        a_nonnull_on_a_pointer_to_a_pointer_at_file_scope_is_refused: ["--emit", "ast"],
        a_nonnull_on_the_return_of_a_function_pointer_is_refused: ["--emit", "ast"],
        // `_Nonnull` on a return is a promise at every level, asked at every
        // `return` and believed by every call, which is `SC0408`'s three rows
        // and a caller of each kind. Mutation: never set the written promise
        // in `Lowering::body`; the first three go silent and the fourth is
        // doubted.
        a_nonnull_return_is_checked_at_its_return_below_level_5: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_returned_from_a_nonnull_return_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The end of a body writes nothing to return, so the `Return` is what
        // is asked. Mutation: have `report_return` ask only where the block
        // wrote the return place; this goes silent.
        the_end_of_a_function_promising_a_nonnull_return_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The block that ends the body writes something, and not the return
        // place, so it is still the end. Mutation: have `findings` take any
        // write in the block as the `return`'s; this is told it may return
        // null at `x = 1` instead.
        the_end_of_a_body_after_a_statement_is_still_its_end: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A loop on a constant has no exit edge, so the end after it is not
        // reached, whatever the constant is but zero (#338). Mutation: have
        // `lowering::decided` always build a `Branch`; both are told they may
        // reach the end. Mutation: fold only the constants 0 and 1; the
        // second is.
        a_loop_left_only_by_a_return_does_not_reach_the_end: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_loop_on_any_constant_but_zero_does_not_reach_the_end: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same through a `for` and through unary `+`, which are the other
        // site that calls `decided` and the other constant expression
        // `Lowering::constant_expression` reads. Mutation: have the `for`
        // build a `Branch` again; the first is told it may reach the end.
        // Mutation: drop the unary `+` arm of `constant_expression`; the
        // second is.
        a_for_loop_on_a_constant_does_not_reach_the_end: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_loop_on_a_constant_under_unary_plus_does_not_reach_the_end: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `return;` where a pointer was promised is refused where it is, by
        // the type checker, and nothing after it runs. Mutation: drop the
        // gate after names and types in `driver.rs::analysed`; the
        // nullability check reads the `return;` as the end of the body and
        // an `SC0408` follows, false of this program. Mutation: accept a
        // `return` without a value in `Checker::receivers_in`; only that
        // `SC0408` is left.
        a_return_without_a_value_in_a_function_that_promised_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call to a function C says does not return has no edge out of it,
        // so the end after it is not reached (ADR-0051). Mutation: have the
        // lowering give every call its continuation; both are told they may
        // reach the end. Mutation: drop any one name from `DOES_NOT_RETURN`;
        // the second is, at that function.
        a_function_that_ends_in_abort_does_not_reach_the_end: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        each_function_that_cannot_return_ends_a_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: have the call's destination answer `Unknown` whatever the
        // callee promised, in `Analysis::terminator`; both are doubted.
        a_result_promised_nonnull_is_read_without_a_test: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The boundary: a function only declared here is believed where its
        // declaration writes `_Nonnull`. Mutation: never set the written
        // promise in `Lowering::declare_one`; this is doubted.
        a_result_a_declaration_promises_is_believed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And doubted where it writes `_Nullable`. Mutation: have
        // `lowering::written_promise` promise for either specifier; this goes
        // silent.
        a_nullable_result_is_doubted_by_its_caller: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The return's specifier agrees across every declaration, `void g();`
        // among them, which declares what `g` returns. Mutation: have
        // `Lowering::agree_on_return` record and never compare; both go
        // silent. Mutation: call it only for a prototype; the second does.
        declarations_that_disagree_about_a_nonnull_return_are_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_unprototyped_declaration_disagrees_about_a_return: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Level 5: the pointer a function defined here returns is not null
        // unless it is written `_Nullable`, asked at every `return` as a
        // written `_Nonnull` is, and believed by every call. Each run is
        // refused beside that, since level 5 asks for checks that are not
        // implemented yet; the default is what this run delivers of it.
        // Mutation: never set `Promise::Defaulted` in `Lowering::body`; the
        // first three go silent and the fourth is doubted.
        a_null_returned_where_level_5_promised_a_pointer_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        an_allocation_returned_where_level_5_promised_a_pointer_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        the_end_of_a_function_promising_a_pointer_at_level_5_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        a_result_level_5_promised_is_read_without_a_test: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        // The same program below level 5, where nothing is promised. This is
        // what keeps levels 1 to 4 answering as before. Mutation: compute
        // `nonnull_returns_by_default` as `true` in `driver.rs`; this goes silent,
        // and every existing case whose pointer function may return null
        // gains `SC0408`, `a_pointer_read_out_of_a_freed_table_and_returned`
        // among them.
        a_result_of_a_function_defined_here_is_doubted_below_level_5: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `_Nullable` takes the promise back: the result is doubted until a
        // test settles it. Mutation: let the default apply where `_Nullable`
        // was written, in `Lowering::body`; the untested read goes silent.
        a_nullable_return_is_tested_before_it_is_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        // The boundary, which believes nothing unannotated in either
        // direction: what a function only declared here returns is doubted,
        // and what one defined here with external linkage takes is too, so a
        // null passed to one needs nothing. Mutation: make the default in
        // `Lowering::declare_one` as well; the first goes silent. Mutation:
        // give an unannotated parameter a promise at level 5 in
        // `Lowering::signature`; the second goes silent and the third is
        // refused with `SC0405`.
        a_result_of_a_function_only_declared_here_is_doubted_at_level_5: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        a_parameter_of_a_function_defined_here_is_doubted_at_level_5: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        a_null_passed_to_a_function_only_declared_here_is_not_reported_at_level_5: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        // A hatch promises what it writes and nothing by default, since what
        // its body could not prove is listed rather than reported, and a
        // default would be believed by every caller and asked by nobody.
        // Mutation: drop `!hatch` from the default in `Lowering::body`; the
        // read in `main` goes silent.
        a_hatch_promises_nothing_by_default_at_level_5: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "strict"],
        // The level just below 5 makes no default either, which is the edge
        // of the comparison in `driver.rs`. Mutation: compute
        // `nonnull_returns_by_default` from `>= SafetyLevel::Thread`; this
        // goes silent.
        a_result_of_a_function_defined_here_is_doubted_at_level_4: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--safety", "thread"],
        // The return's three answers, compared as the parameter's are:
        // `_Nonnull` against `_Nullable` as well as either against nothing,
        // a definition before the declaration as well as after, and each
        // declarator of one declaration on its own. Mutation: compare whether
        // a specifier was written in `Lowering::agree_on_return`; the first
        // goes silent. Mutation: hand `declare_one` no specifier for a
        // definition; the second does. Mutation: drop the specifier
        // `named_declarator` returns for a later declarator, so it keeps the
        // first's; `g` promises and the third goes silent.
        declarations_that_disagree_between_nonnull_and_nullable_about_a_return_are_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_definition_that_disagrees_with_a_later_declaration_about_its_return_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_second_declarator_does_not_take_the_first_ones_return_specifier: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A function declared in a block is not one `lowering.rs::declare`
        // reads, so a specifier on its return would mean nothing. Mutation:
        // let `Parser::placed` accept a return under `Declares::BlockScope`;
        // this goes silent.
        a_nonnull_on_the_return_of_a_block_scope_function_is_refused: ["--emit", "ast"],
        // The second of two specifiers, whichever came first. Mutation: have
        // `Parser::core` ask whether the one before was `_Nonnull` only; this
        // is told to write it after the `*`, which it did.
        a_nonnull_after_a_nullable_on_one_pointer_is_refused: ["--emit", "ast"],
    }

    "hatch" => {
        // A hatch: a function definition whose unproven conclusions are listed
        // rather than reported. See ADR-0038, whose Confirmation names the
        // mutation each of these fails under.
        //
        // One case per stage it passes through, because a feature guarded at
        // one stage is unguarded at every stage that reads it: the tree, the
        // IR, and the listing.
        a_hatch_is_read_into_the_tree: ["--emit", "ast"],
        // The pair the record is about: one body, reported outside a hatch and
        // not inside one. Mutation: have `Lowering::body` never call
        // `Function::unchecked`; the first of the two fails.
        an_unproven_dereference_inside_a_hatch_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        the_same_dereference_outside_a_hatch_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: have `route` keep nothing in `hatched`, which is the hatch
        // as a suppression; this fails.
        an_unproven_dereference_inside_a_hatch_is_listed_and_not_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: answer `!in_a_hatch` for `Unsafe` in `route`; this fails.
        a_proved_double_free_inside_a_hatch_is_still_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // The silent direction of getting the function wrong: a conclusion
        // about a function after a hatch, credited to the hatch, is not
        // reported. The hatch is the unit's first function so that a finding
        // naming the first one lands on it. Mutation: have either check's
        // `Finding::function` name `unit.functions().next()`; this fails.
        a_function_after_a_hatch_is_still_answered_for: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // The two checks' conclusions interleaved by caret, where each check's
        // own come out in a run. Mutation: drop the sort in `dump_hatches`;
        // this fails.
        what_a_hatch_concluded_is_listed_in_the_order_it_was_written: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // The boundary is the prototype, and the checked side reads it.
        a_null_passed_to_a_nonnull_parameter_of_a_hatch_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a hatch may have done to what it was handed is assumed to be the
        // worst, because it cannot yet say otherwise: ADR-0032's default.
        freeing_what_was_handed_to_a_hatch_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A hatch's body is the one whose unproven conclusions are not
        // reported, so what the caller cannot see it do is assumed to be the
        // worst: every allocation still live is unproven after a call to one.
        // Mutation: drop the loop over `value.state` in `memory/transfer.rs`'s
        // `Callee::Opaque` arm; the first two fail, and the first is a use
        // after free going silent.
        what_a_hatch_frees_through_what_it_was_handed_is_unproven_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_a_hatch_was_not_handed_is_unproven_after_it_too: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: make that loop mark every site rather than the live ones;
        // this fails, a proved double free becoming unproven.
        a_free_proved_before_a_call_to_a_hatch_stays_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Inside a hatch, a call handed what may be a freed allocation is a
        // doubt, which is listed, so a proved use after the call is the only
        // report left. Mutation: in the `Callee::Opaque` arm, write
        // `SiteState::Unknown` over what an argument names outright; the use
        // becomes a doubt too, and this builds with nothing reported (#262).
        a_free_proved_before_a_call_handed_what_may_be_it_stays_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same one level down: the call is handed memory that holds the
        // freed pointer, and keeps the proof as it does for what it is handed
        // by name. Mutation: in the `Callee::Opaque` arm, write
        // `SiteState::Unknown` over a site the closure of what the call is
        // handed reaches but was not handed directly; this builds with
        // nothing reported.
        a_free_proved_before_a_call_handed_what_holds_it_stays_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Each operand of the `&&` is reported at its own span since #147, so
        // the proved `*q` is not folded with the unproven `*p` and is
        // reported rather than listed. The fold that used to decide this, the
        // worst kept at one caret, is held by
        // `a_proof_and_a_suspicion_at_one_caret_report_the_proof` in
        // `crates/safec-ir/tests/nulls.rs`.
        a_proved_null_dereference_beside_an_unproven_one_in_a_hatch_is_still_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // Both arms of a `?:` are written at the whole conditional's span, so
        // the proved `*q` and the unproven `*p` meet at one caret and the
        // worst is kept, which in a hatch is the difference between reported
        // and listed. Mutation: never swap in `nullability::findings`'
        // `dedup_by`; the proof is listed as a suspicion and the run exits 0.
        // Narrowing an arm's write to the arm moves this case.
        a_proved_null_dereference_in_one_arm_of_a_conditional_in_a_hatch_is_still_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // An attribute `sema::resolve` refused is an error, so the run stops
        // before the lowering: no hatch is marked and no check runs, and the
        // dereference behind it is not reported until the attribute is gone.
        // Mutation: drop the gate after names and types in
        // `driver.rs::analysed`; the `SC0403` returns.
        a_refused_attribute_stops_the_run_before_the_checks: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: have `dump_hatches` list every hatch's conclusions under
        // each; this fails.
        each_hatch_lists_only_what_was_concluded_inside_it: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // The listing is the count of hatches, whether or not a check ran.
        every_hatch_is_listed_at_safety_off_with_nothing_under_it: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc", "--safety", "off"],
        a_program_with_no_hatch_lists_none: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // Everywhere it cannot apply, and every attribute that is not it: one
        // case per place `parser.rs` and `sema.rs` refuse one.
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
    }

    "exposure" => {
        // What code this check cannot read may reach: an allocation is exposed
        // once it may, every opaque call unproves every exposed one, and may
        // return any of them. See ADR-0039, whose Confirmation names the
        // mutation each of these fails under.
        //
        // Reported, each a use after free or a double free for some definition
        // of the callees C permits.
        what_a_callee_frees_through_a_pointer_stored_in_the_heap_is_unproven_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        what_a_callee_frees_through_a_pointer_stored_in_a_local_is_unproven_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_may_return_what_it_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_handed_an_address_may_return_what_is_behind_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_may_return_what_an_earlier_call_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Its free of `t` was also `SC0404` until `slot`'s escaped address
        // was among what `get` may return: `Held::may_be_a_locals_address`
        // makes `touching` hand a `Reached::Lost`, and `interior` believes no
        // offset beside one, so the same free is reported as the double free
        // alone.
        a_call_in_a_loop_may_return_what_it_returned_last_turn: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And any escaped local's address, which a free of it would free
        // though it is no allocation. Mutation: give an opaque call's result
        // no `writes_to` edge in `Allocations::terminator`'s opaque arm;
        // `a_call_handed_a_locals_address_may_return_it` exits 0. Mutation:
        // give it an edge to every local, escaped or not;
        // `a_call_returns_none_of_this_functions_locals_where_none_escaped`
        // is refused. It shows only that: a local of another frame, or a
        // string, is still believed an allocation, #375.
        a_call_handed_a_locals_address_may_return_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_returns_none_of_this_functions_locals_where_none_escaped: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The edge is also what a write through the result follows. Mutation:
        // give it an edge to the first escaped local only; the `*r = p` that
        // may land in `y` is lost, and the read of `y` after `free(p)` is no
        // longer a use after free.
        a_write_through_what_a_call_returned_may_land_in_an_escaped_local: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local an earlier call was handed, not this one. Its free was
        // `SC0404` beside `SC0401` before, and is `SC0401` alone, for the
        // reason the loop case above gives. It is worded as the free of a
        // pointer this check stopped following rather than as a double free,
        // because `stash` returns `void` and so makes no site of its own for
        // `get` to return. Mutation: give the edge only
        // where this call was handed something; this goes silent, because
        // `stash` returns `void` and so leaves no result of its own for
        // `get` to hand back.
        a_call_may_return_a_locals_address_an_earlier_call_kept: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The cost, recorded rather than discovered: a constructor handed a
        // local's address and returning fresh memory is a correct program,
        // and its free is doubted, as one handed an allocation already was.
        // Same mutation as `a_call_handed_a_locals_address_may_return_it`;
        // this builds.
        a_constructor_handed_a_locals_address_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_exposed_on_one_arm_is_unproven_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_stored_on_one_arm_is_reached_through_what_holds_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_proved_before_a_call_stays_proved_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A use of the old pointer after a `realloc` that succeeded is a
        // proof: C17 7.22.3.5 p4 deallocates it, and the branch on the result
        // says the call succeeded. See ADR-0039. Mutation: have the non-null
        // arm free nothing; this goes back to a doubt.
        the_old_pointer_used_after_a_successful_realloc_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // so a free through that local proves nothing about what a sharer
        // holds.
        a_local_memcpy_is_handed_the_address_of_may_hold_something_else_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A loop through one call writes one site, and what the call returns
        // may be what was freed through that site last turn.
        a_call_in_a_loop_may_hand_back_what_was_freed_last_turn: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A write two levels down is not recorded as contents, so it exposes
        // what it carries at once.
        a_pointer_stored_two_levels_down_is_reached_through_what_holds_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The closure over what an allocation holds goes as deep as the tables
        // do. Mutation: stop pushing what `Known::expose` newly marks; this
        // fails.
        a_pointer_two_tables_deep_is_reached_through_both: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer that holds two allocations stores into both. Mutation:
        // record into the first container only; this fails.
        a_pointer_stored_through_either_of_two_tables_is_inside_both: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A free, or a use, of the old pointer on the branch where `realloc`
        // failed builds, since C17 7.22.3.5 p3 leaves it allocated, however the
        // branch tests the result. See ADR-0039. Mutation: have the branch
        // restore nothing; the first goes back to `SC0401`. Mutation: have it
        // read `==` and `!=` only; the second does.
        a_free_on_reallocs_failure_branch_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_on_reallocs_failure_branch_tested_with_not_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        the_old_pointer_used_on_reallocs_failure_branch_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same branch read off `if (q)` with `q` copied from the result just
        // above the branch, which the memory check learns from through the
        // nullability check's reading. Mutation: delete the early return for a
        // pointer condition in `nullability::tested_against_null`; the first
        // goes back to `SC0401` may, and the second's proof to a doubt (#334).
        a_free_on_reallocs_failure_branch_tested_by_a_copy_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_the_old_pointer_after_a_realloc_tested_by_a_copy_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Refused as before where the size may be zero, which leaves whether a
        // failed call frees implementation-defined. Mutation: read any size;
        // both build. Mutation: read any size but a literal zero; the second
        // builds.
        a_free_on_reallocs_failure_branch_with_size_zero_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_on_reallocs_failure_branch_with_a_variable_size_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And where the pointer tested may be something other than what
        // `realloc` returned: replaced through its address by a call this
        // check cannot read, or holding another allocation too. Mutation:
        // have the branch ignore `Held::lost`; the first builds. Mutation:
        // have it read the first site of several; the second builds.
        a_free_on_reallocs_failure_branch_after_its_result_was_replaced_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_on_a_branch_testing_more_than_reallocs_result_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And where the null tested may not be the call's, or the old
        // allocation may not be as the call left it, all found by review: a
        // copy of the result nulled by hand, the result nulled on one arm, and
        // the old pointer freed on one arm, reallocated again, or handed to a
        // call between the `realloc` and the branch. Each is a double free on
        // some execution. Mutation: have the branch ignore
        // `Held::returned_by`; the first two build. Mutation: have no call
        // forget a fact naming what it touches; the fourth loses its report at
        // the free on the null arm. Three guards overlap on the rest: with
        // touching and a join's disagreement both gone the third loses it,
        // and with touching and an exposure both gone the fifth does.
        a_free_on_a_branch_on_a_copy_of_reallocs_result_nulled_by_hand_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_on_a_branch_on_reallocs_result_nulled_on_one_arm_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        the_old_pointer_freed_on_one_arm_before_the_branch_is_reported_again: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        the_old_pointer_freed_after_a_second_realloc_before_the_branch_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        the_old_pointer_freed_after_a_call_was_handed_it_before_the_branch_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `if (*q)` tests what `realloc` returned points at, not whether it
        // returned null, so neither arm says what became of `p`, and the free
        // of it after a call that succeeded is still asked about. Mutation:
        // delete the early return for a dereferenced condition in
        // `nullability::tested_against_null`; the arm where `*q` is zero is
        // read as the call failing, and the `SC0401` at `free(p)` goes.
        a_branch_on_what_reallocs_result_points_at_says_nothing_about_the_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `realloc` of a pointer read out of memory remembers nothing:
        // what it was handed is not the local the place starts at. Mutation:
        // let a place with a projection through; three false proofs about
        // the allocation that held it.
        a_realloc_of_a_pointer_read_out_of_memory_leaves_what_held_it_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Refused and well defined: a cost ADR-0039 accepts, pinned so that a
        // change to it is seen.
        a_call_after_an_allocation_was_exposed_may_return_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Built.
        what_memset_returns_is_the_allocation_it_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        memcpy_frees_neither_of_its_arguments: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        what_strcpy_returns_is_the_allocation_it_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Every spelling of the family read by name. Mutation: drop any one of
        // `memmove`, `strncpy`, `strcat` or `strncat` from
        // `Allocations::callee`; this fails.
        what_the_other_library_copies_return_is_what_they_were_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_no_call_can_reach_stays_proved_across_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_stored_in_the_heap_is_not_exposed_until_what_holds_it_is: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A store through a pointer that holds a site and may also be a load is
        // unplaced for that part: `*tab` may be the caller's, and `release_all`
        // may reach `a` there on one arm. The second is the same store
        // unsequenced with a read before it and a call after it. Mutation: drop
        // `loaded` from `unnamed` in `Allocations::element`'s store arm; both
        // go silent, and the list case below builds. The third is a pointer
        // that may be either of two allocations this check follows, and stays
        // placed. Mutation: answer `true` for `unnamed`; the third reports. See
        // ADR-0044.
        a_store_through_a_pointer_that_may_be_a_load_exposes_what_it_stores: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_before_a_store_through_a_pointer_that_may_be_a_load_is_asked_at_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_store_through_a_pointer_that_may_be_either_of_two_allocations_exposes_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local a call this check cannot read may have written into is the
        // same, though it holds no site, on either arm and moved by arithmetic.
        // Mutation: set no `foreign` in `Known::replaced`; all three go silent.
        // Mutation: drop `foreign` from `unnamed`; all three go silent.
        // Mutation: keep one arm's `foreign` in `Held::joined`; the second goes
        // silent, the call being on the arm the join does not keep. Mutation:
        // carry none in `Held::accumulated`; the third goes silent.
        a_store_through_a_pointer_a_call_filled_in_exposes_what_it_stores: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_store_through_a_pointer_a_call_filled_in_on_the_first_arm_exposes_what_it_stores: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_moved_off_one_a_call_filled_in_is_still_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other side: a call this check cannot read writes only through
        // addresses that escaped. Mutation: set `foreign` in `Known::replaced`
        // on every local it may write, escaped or not; this reports.
        a_local_whose_address_no_call_was_given_is_not_one_a_call_filled_in: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The cost: a load of the function's own memory is exposed through like
        // any other, so a list built and appended to in one function is
        // refused, with a double free that cannot happen. Following a load to
        // where it was read from would remove it. See ADR-0044.
        a_list_built_and_appended_to_in_one_function_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer that lost the name for its site is unplaced too. Mutation:
        // drop `lost` from `unnamed`; the report at `a[0]` goes, and the one at
        // the store stays.
        a_store_through_a_pointer_that_lost_its_site_exposes_what_it_stores: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_made_again_at_a_site_is_not_the_one_exposed_before: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `realloc`'s size is not asked whether it was freed.
        the_size_realloc_is_handed_is_not_asked_whether_it_was_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `realloc` read by name: the new object holds what the old one held,
        // what it returns is named where it was allocated, a proved free before
        // it stays proved, and a pointer into an allocation is asked about as
        // `free` asks about one.
        a_table_realloc_grew_still_holds_what_it_held: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        what_realloc_returns_is_named_where_it_was_allocated: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_freed_before_realloc_stays_freed_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_into_an_allocation_handed_to_realloc_is_not_its_start: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C17 7.22.3 p1 holds `calloc` and `aligned_alloc` to what `malloc` is.
        what_calloc_returns_is_an_allocation_nobody_else_has: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        what_aligned_alloc_returns_is_an_allocation_nobody_else_has: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    }

    "loads" => {
        // What a pointer read out of memory reaches, for a call and for a
        // store, and a pointer parameter exposed where its function starts. See
        // ADR-0040, whose Confirmation names the mutation each of these fails
        // under.
        //
        // Reported.
        what_a_callee_frees_through_a_pointer_read_out_of_a_table_is_unproven_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_from_one_table_to_another_is_inside_both: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_table_read_out_of_a_holder_into_a_local_is_reached_through_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_into_a_local_whose_address_a_call_is_handed_is_reached_through_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_a_table_through_an_address_is_reached_through_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Reported, and what ADR-0040 accepts as its cost: every pointer
        // parameter read after any call this check cannot read, and freed after
        // one, is unproven.
        an_allocation_a_parameter_holds_is_unproven_after_any_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_may_return_what_a_parameter_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Built: a pointer holding no site is not a load, and a load through
        // one table reaches only what that table holds.
        a_null_pointer_handed_to_a_call_exposes_nothing_stored: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same distinction for a local whose address escaped, which
        // `Known::reach_of` answers. Mutation: test there for no site instead
        // of the bit; this fails.
        a_null_pointer_whose_address_a_call_is_handed_exposes_nothing_stored: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_one_table_exposes_only_what_that_table_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `argv` is the host's, and no call can free it. Mutation: drop the
        // `main` filter on `exposed_parameters`; this fails.
        the_arguments_the_host_hands_main_are_not_exposed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The bit a load leaves on a local: set by a load on either arm of a
        // join, carried through arithmetic, cleared by what the local is given
        // next, and never set by an integer. A load handed to `memset` is read
        // as one too.
        a_pointer_read_out_of_memory_on_one_arm_is_still_one_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_moved_off_a_load_is_still_a_load: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        what_memset_is_handed_out_of_a_table_is_unproven_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_given_something_else_after_a_load_is_no_longer_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_integer_read_out_of_memory_is_not_a_load: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A loop that allocates a table again leaves last turn's table holding
        // what it held, reached through a local that lost its name for it and
        // through what holds it. Mutation: clear the row in `Known::reborn`;
        // both fail.
        a_table_allocated_again_by_a_loop_still_holds_what_it_held: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],

        // A pointer read out of the function's own memory, dereferenced: it
        // holds what was stored where it was read from, and proves nothing with
        // it. See ADR-0045, whose Confirmation names the mutation each of these
        // fails under.
        //
        // Reported, after a free and after a call that reaches the table.
        // Mutation: a load holds no site again; both fail, and so does the
        // two-slot case below.
        a_use_after_free_through_a_pointer_read_out_of_memory_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_memory_is_unproven_after_a_call_that_reaches_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same two levels down, `**tab`, asked through what the table's own
        // allocation holds rather than through a load. Mutation: `used` not
        // asking `reached_below`; the first goes silent. Mutation: the second
        // level not marked as possibly incomplete; the second becomes a proof.
        a_read_two_levels_down_after_its_allocation_was_released_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_two_levels_down_after_a_free_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the first level is still asked first: a freed table is proved, as
        // it was before the second level was. Mutation: `used` asking only
        // `reached_below`; this goes silent.
        a_read_two_levels_down_through_a_freed_table_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read two levels down, `q = **t3`, is not given what one
        // level down holds: the report is at `**t3`, where the freed table is
        // read, and not again at `*q`. Mutation: `read_through` giving a load
        // sites whatever its projection; `*q` reports too.
        a_pointer_read_two_levels_down_holds_nothing_from_the_level_above: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read out of a load holds what was stored where it was
        // read from, so a chain of two loads is followed: reported, never
        // proved, and silent when what the chain reaches is live. Mutation:
        // restore `loaded` to `Known::stored_in`'s condition; the first goes
        // silent. Mutation: count `Reached::Partial` as a doubt; the second
        // reports.
        a_use_after_free_through_a_chain_of_two_loads_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_live_value_read_through_a_chain_of_two_loads_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same chain read by `Known::reached_below` rather than by a load:
        // `**q2`, where `q2` is a load. Mutation: have `reached_below` answer
        // nothing for a local that is a load; this goes silent.
        a_read_two_levels_down_through_a_load_after_a_free_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same chain written as one place: `q = **t3` read into a local,
        // and `***t3` dereferenced, each level asked in the order C reads it.
        // Mutation: have `read_through` follow only one `Deref` again; the
        // first goes silent. Mutation: have `used` ask no level past the
        // second; the second goes silent, while the third is still answered
        // at its second level. Mutation: have `used` ask the deepest level
        // only; the third goes silent. Mutation: count
        // `Reached::Partial` as a doubt; the fourth reports.
        a_use_after_free_through_a_chain_written_as_one_place_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_three_levels_down_after_a_free_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_three_levels_down_through_a_freed_middle_level_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // "In silence" is the memory check's: no use after free is reported.
        // The nullability check doubts the read through memory in place, as a
        // warning under `--allow-unknown` (#333).
        a_live_value_read_three_levels_down_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And one level deeper on each path, so that "however long" is held
        // past the depths above. Mutation: have `read_through` follow no more
        // than two `Deref`s; the first goes silent. Mutation: have `used` ask
        // no level past the third; the second goes silent.
        a_use_after_free_through_a_chain_of_three_loads_written_as_one_place_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_four_levels_down_after_a_free_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A load moved by arithmetic, `t3[i][i]` or `**t3 + i`, holds what it
        // held at an offset nobody said. `--allow-unknown`, so that the
        // nullability check's doubt about the subscript is a warning and the
        // memory check's `SC0402` is visible beside it rather than standing in
        // for it. Mutation: have `built_from` hold no sites for a load again;
        // the first three go silent about the free. Mutation: read a load's
        // sites one level down whatever its depth; the second and the third
        // go silent.
        // Mutation: count `Reached::Partial` as a doubt; the fourth reports.
        a_load_moved_by_a_subscript_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_load_moved_by_arithmetic_and_read_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_load_moved_by_arithmetic_and_handed_on_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_live_load_moved_by_a_subscript_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And the integer beside a load decides nothing about where the sum
        // points: `n` holds what `h` was handed, which is freed, and `*q` is
        // inside `p`, which is live. No `SC0402` at `return *q;`; the `SC0401`
        // is the free of what `h` may have freed. Mutation: let the integers
        // beside a load contribute again; this reports `SC0402` at `*q`.
        an_integer_beside_a_load_does_not_decide_where_it_points: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A pointer stored in a loop and freed, read back after the loop
        // allocates again at its site, is one this check stopped following
        // rather than the new allocation. See ADR-0045. `--allow-unknown`, so
        // that the nullability check's doubts are warnings beside it.
        //
        // Reported: through a local, through arithmetic, two levels down, and
        // freed on one arm only. Mutation: have `reborn` mark nothing; the
        // first four go silent. Mutation: have `read_through` ignore the mark;
        // the first, third and fourth go silent. Mutation: have `built_from`
        // ignore it; the second. Mutation: have `stale_below` read one level
        // only; the third. Mutation: drop `stale` at the join; the fourth.
        a_pointer_stored_and_freed_last_turn_is_doubted_after_the_loop_allocates_again: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And where a call that may have freed it reached it through memory,
        // which leaves it `Reachable` rather than `Unknown`: a doubt is gone to
        // `reborn` whichever it is. Found by review of ADR-0047. Mutation: have
        // `reborn` count `Reachable` as live; both go silent, the first through
        // the container's mark and the second through the copy's.
        a_pointer_stored_last_turn_that_a_call_may_have_freed_is_doubted_after_the_loop_allocates_again: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_last_turn_that_a_call_may_have_freed_is_doubted_when_stored_and_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_load_moved_by_arithmetic_and_stored_last_turn_is_doubted_after_the_loop_allocates_again: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_two_levels_down_freed_last_turn_is_doubted_after_the_loop_allocates_again: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_freed_on_one_arm_last_turn_is_doubted_after_the_loop_allocates_again: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And the same read as one place, `**t2`, through `reached_below`.
        // Mutation: have `reached_below` ignore the mark; this goes silent.
        a_read_two_levels_down_through_a_pointer_freed_last_turn_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And a marked container grown by `realloc`, whose copy of what it
        // holds carries the mark. Mutation: copy the row without the mark;
        // this goes silent.
        a_container_grown_by_realloc_keeps_its_mark: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And the entry a reborn site leaves in a container is kept, for what
        // a call handed the container reaches: `r`, stored in last turn's
        // allocation, may be freed by `release(t2)`. Mutation: drop the entry
        // where the container is marked; this goes silent.
        a_call_handed_a_container_of_a_reborn_site_reaches_what_it_held: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A load out of a marked allocation is doubted still once stored into
        // another, directly and through a local. Mutation: have a store not
        // mark its container for a value read out of a marked one; both go
        // silent.
        a_doubted_load_stored_into_another_container_stays_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_doubted_load_stored_through_a_local_into_another_container_stays_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // The bit that says so is carried as a load's is: across a join, and
        // through arithmetic. Mutation: have `Held::joined` drop it; the
        // first goes silent. Mutation: have `Held::accumulated` drop it; the
        // second goes silent.
        a_doubted_load_on_one_arm_stored_into_another_container_stays_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_doubted_load_moved_and_stored_into_another_container_stays_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And cleared by what a local is given next, and never set by a value
        // this check merely lost: a list built with nothing freed loses its
        // `head` every turn, and storing it marks nothing. Mutation: have
        // `Held::clear` keep the bit; the first reports. Mutation: have a store
        // mark its container for any lost value; the second reports.
        a_local_given_something_else_after_a_doubted_load_stores_it_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_list_built_in_a_loop_with_nothing_freed_is_walked_without_a_doubt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A load through a pointer to a local reads what the local holds, and
        // is lost where the local is. See ADR-0045. Mutation: have `stored_in`
        // read no edge to a local; the first and the fourth go silent about
        // the free. Mutation: have `read_through` ignore `lost_through`; the
        // third goes silent. Mutation: count `Reached::Partial` as a doubt;
        // the second reports.
        a_pointer_written_into_a_local_through_its_address_and_read_back_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_live_pointer_written_into_a_local_through_its_address_is_read_back_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_local_this_check_lost_read_through_its_address_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_to_either_of_two_locals_reads_what_both_hold: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // The same edge read by the other readers: one level below, `**t2`;
        // handed on, `deref(*t2)`; and moved by arithmetic, `*t2 + i`. Lost
        // where the local is, for each. Mutation: have `reached_below` ignore
        // `lost_through`; the first two go silent. Mutation: have
        // `built_from` ignore it; the third goes silent.
        a_read_two_levels_down_through_a_pointer_to_a_local_this_check_lost_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_to_a_local_this_check_lost_handed_on_through_it_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_load_moved_by_arithmetic_through_a_pointer_to_a_local_this_check_lost_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And the local's sites through those readers after a free. Mutation:
        // have `reached_below` leave out what the edge's local holds; both go
        // silent.
        a_read_two_levels_down_through_a_pointer_to_a_local_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_load_through_a_pointer_to_a_local_handed_on_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A copy of an object, and a store more than one dereference down,
        // are recorded where they land, so a load out of the destination reads
        // what was copied or stored. See ADR-0039 and ADR-0045. Mutation: have
        // `Callee::Copies` record nothing; the first two go silent. Mutation:
        // map `memcpy` back to `Callee::ReturnsFirst`; the same. Mutation:
        // have the store arm find no containers below one dereference; the
        // third goes silent. Mutation: count `Reached::Partial` as a doubt;
        // the fourth reports.
        a_pointer_copied_by_memcpy_into_a_table_and_freed_is_unproven_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_copied_by_memcpy_from_a_locals_address_is_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_store_two_levels_down_is_recorded_where_it_lands: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // "In silence" is the memory check's: no use after free is reported.
        // The nullability check doubts the read through memory in place, as a
        // warning under `--allow-unknown` (#333).
        a_live_pointer_copied_or_stored_two_levels_down_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What ADR-0045 accepts as its cost: a copy adds to what the
        // destination holds and replaces nothing, so a freed pointer it held
        // before is doubted after, as on `main`.
        a_copy_over_a_freed_pointer_is_doubted_though_it_replaced_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A deep store is exposed as well as recorded, since where it may land
        // is a lower bound: `*t3` may be what the caller stored. Mutation:
        // have a deep store not count as `unnamed`; the first goes silent at
        // `return *r`. And a copy out of an allocation a loop's rebirth marked
        // marks the destination. Mutation: have `Callee::Copies` mark nothing;
        // the second goes silent.
        a_store_two_levels_down_through_a_set_this_check_may_not_know_whole_is_exposed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_copy_out_of_an_allocation_a_loop_allocated_again_stays_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And a copy whose destination or source is itself read out of memory,
        // `memcpy(*pp, src, 8)` and `memcpy(tab, *ps, 8)`. Mutation: have
        // `Callee::Copies` read only plain locals as its arguments; both go
        // silent.
        a_copy_into_a_destination_read_out_of_memory_is_recorded_there: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_copy_from_a_source_read_out_of_memory_is_recorded: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A pointer read out of what a parameter points at is one this check
        // stopped following after a call it cannot read, which may free what
        // the caller stored. See ADR-0040. Mutation: never set
        // `Held::from_caller`; the first goes silent. Mutation: have the call
        // not make it lost; the same. Silent with no call between, and for
        // what the host hands `main`. Mutation: set it for every parameter;
        // the fourth reports.
        a_pointer_read_out_of_a_parameter_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_with_no_call_before_its_use_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What ADR-0040 accepts as its cost, one level in: any such call.
        a_pointer_read_out_of_a_parameter_is_doubted_after_any_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_mains_arguments_is_not_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // The bit is carried as a load's is: by a load moved by arithmetic, across
        // a join, and through a copy moved by arithmetic; and cleared by what a
        // local is given next. Mutation: have `built_from` ignore it; the first
        // goes silent. Mutation: have `Held::joined` drop it; the second loses
        // its doubt. Mutation: have `Held::accumulated` drop it; the third goes
        // silent. Mutation: have `Held::clear` keep it; the fourth reports.
        a_pointer_read_out_of_a_parameter_and_moved_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_on_one_arm_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_copied_by_arithmetic_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_local_given_something_else_after_a_load_out_of_a_parameter_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And by a load through a local that carries it, `pp = *ppp; q = *pp;`,
        // which is `q = **ppp` one load at a time. Mutation: have
        // `reads_caller_memory` read only the parameters' sites; it goes silent.
        a_pointer_read_out_of_a_parameter_through_a_chain_of_loads_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And through a store into this function's own memory, which marks
        // the allocation: read back after a call, or read back and then a
        // call, it is doubted; with no call it is not. See ADR-0040.
        // Mutation: have the store never mark `Known::from_caller`; the first
        // two go silent. Mutation: have the call never make a marked
        // allocation `Known::stale`; the first goes silent. Mutation: have
        // `reads_caller_memory` ignore a marked site; the second goes silent.
        // Mutation: have the store mark `Known::stale` at once; the third
        // reports. Mutation: have the join drop the mark; the fourth goes
        // silent. Mutation: have `realloc` drop it; the fifth goes silent.
        a_pointer_read_out_of_a_parameter_and_stored_is_doubted_when_read_back_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_stored_is_doubted_when_read_back_before_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_stored_with_no_call_is_read_back_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_stored_on_one_arm_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_stored_is_doubted_after_realloc_and_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And read back through more than one level, as one place or moved by
        // arithmetic, and copied by `memcpy` rather than stored. Found by
        // review. Mutation: have `reads_caller_memory` ask the local's own
        // allocations only; the first two go silent. Mutation: have `memcpy`
        // carry no `Known::from_caller`; the last two go silent.
        a_pointer_read_out_of_a_parameter_and_stored_is_doubted_when_read_back_two_levels_down_before_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_stored_is_doubted_when_read_back_two_levels_down_and_moved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_copied_by_memcpy_is_doubted_when_read_back_before_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        what_a_parameter_points_at_copied_by_memcpy_is_doubted_when_read_back_before_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A pointer that lost an allocation that may be gone, stored and read
        // back, is doubted as it is read directly: lost to a rebirth of a
        // freed allocation, read through its address, or made lost by a call
        // this check cannot read. See ADR-0045. Mutation: have the rebirth
        // never set `Held::stale_read`; the first two, and the moved one
        // below, go silent. Mutation:
        // have a load through a lost local carry nothing; the second goes
        // silent. Mutation: have the call not set it; the third goes silent.
        // Mutation: have the rebirth set it whether or not the allocation is
        // gone; `a_list_built_in_a_loop_with_nothing_freed_is_walked_without_a_doubt`
        // is doubted.
        a_pointer_lost_to_a_free_and_stored_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_lost_to_a_free_read_through_its_address_and_stored_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_stored_after_a_call_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And the same load moved by arithmetic, which `built_from` reads
        // rather than `read_through`. Mutation: have `built_from` carry
        // nothing through a lost local; this goes silent at the read.
        a_pointer_lost_to_a_free_read_through_its_address_moved_and_stored_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And through a pointer that may name either of two locals, the lost
        // one not first. Found by review. Mutation: have `stale_through` ask
        // the first local it points at only; this goes silent at the read.
        a_pointer_lost_to_a_free_read_through_one_of_two_addresses_and_stored_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A local's address stored in an allocation is remembered, and the
        // walk of a chain steps through the local as through an allocation:
        // read through `***t3`, read back as `m` and written through, and
        // written two levels down into the local. Silent with `r` live. See
        // ADR-0045. Mutation: have the store never mark
        // `Known::inside_locals`; the first three go silent. Mutation: have a
        // load carry no edges; the second goes silent. Mutation: have a store
        // two levels down land in no local; the third goes silent.
        a_locals_address_stored_in_an_allocation_is_followed_by_a_read_through_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_locals_address_read_back_out_of_an_allocation_is_written_through: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_write_through_a_locals_address_stored_in_an_allocation_lands_in_the_local: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // "In silence" is the memory check's: no use after free is reported.
        // The nullability check doubts the read through memory in place, as a
        // warning under `--allow-unknown` (#333).
        a_live_pointer_through_a_locals_address_stored_in_an_allocation_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And a local's own address edge below the first level, `t3 = &t2;
        // t2 = &slot; u = *t3; *u`. Silent with the value live and written
        // through the alias. Mutation: have `Known::level_below` ignore a
        // local's own edges; the first goes silent.
        a_load_through_two_locals_addresses_reads_what_the_last_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_live_value_through_two_locals_addresses_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And the table carried as `inside` is: across a join, by `realloc`
        // and by `memcpy`. Mutation: have the join keep only what both arms
        // hold, `realloc` copy no row, or `memcpy` copy no locals; the case
        // named for each goes silent.
        a_locals_address_stored_on_one_arm_is_followed_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_locals_address_stored_in_an_allocation_is_followed_after_realloc: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_locals_address_copied_by_memcpy_is_followed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And a local this check lost, passed through two levels down in one
        // place, `**t3` with `*t3 = &t2`, is doubted as `u = *t3; *u` is.
        // Found by review. Mutation: have `lost_through` ask the first level
        // only; this goes silent.
        a_local_this_check_lost_read_two_levels_down_through_a_stored_address_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A pointer to a local moved by an offset this check cannot read may
        // be the local, so the edge survives the arithmetic and a read
        // through it reads what the local holds: `po[k - 1]` as `*po`. A
        // write through it with nothing freed says nothing more. See
        // ADR-0019. Mutation: have `Held::moved_by_arithmetic` empty the
        // edge whatever the offset; the first two go silent at the read.
        a_pointer_to_a_local_moved_by_an_unknown_offset_reads_what_the_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_to_a_local_moved_by_an_unknown_offset_and_stored_reads_what_the_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_write_through_a_pointer_to_a_local_moved_by_an_unknown_offset_with_nothing_freed_is_silent: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And the same with the pointer moved being itself a load, through a
        // local's address or out of a heap box: `built_from` carries the
        // locals a load may be, as `read_through` does. Found by review.
        // Mutation: have `built_from` carry no locals for a load; both go
        // silent at the read.
        a_load_of_a_pointer_to_a_local_moved_by_an_unknown_offset_reads_what_the_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_to_a_local_read_out_of_memory_and_moved_by_an_unknown_offset_reads_what_the_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And a pointer read out of one parameter after another is freed.
        // See ADR-0040. Mutation: have the free not make such a local lost;
        // this goes silent.
        a_pointer_read_out_of_a_parameter_after_another_is_freed_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And a free of a pointer read out of caller memory, which a read
        // through the parameter reaches again. Found by review. Mutation:
        // have `frees_callers` ignore `Held::from_caller`; this goes silent at
        // the read. Mutation: run the rule after the free's own arm, which
        // returns early for such a pointer; the same.
        a_free_of_a_pointer_read_out_of_a_parameter_doubts_a_read_through_the_parameter: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // Silent: a loop that keeps last turn's allocation alive, and one that
        // stores and reads within a turn with nothing freed. Mutation: have
        // `reborn` mark whatever the old allocation's state; both report.
        a_pointer_kept_alive_last_turn_is_read_in_silence_after_the_loop_allocates_again: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_stored_and_read_in_one_turn_with_nothing_freed_is_silent: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What ADR-0045 accepts as its cost: a loop that stores, reads and
        // frees within one turn is doubted on the next, since a store does not
        // clear the mark while slots are not told apart.
        a_pointer_stored_and_read_in_one_turn_is_doubted_on_the_next: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A free of a load stays the doubt it was rather than a free of what
        // the load holds. Mutation: `touching` not answering `Lost` for a load;
        // this goes silent.
        a_free_of_a_pointer_read_out_of_memory_stays_a_doubt: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And what it writes is a doubt too. `q` is null, so `free(q)` frees
        // nothing and `*p` reads a live allocation; the read is doubted, never
        // proved. Mutation: drop `loaded` from `holds_something_unnameable`;
        // the free writes `Freed` and `*p` becomes a proof.
        a_free_of_a_pointer_read_out_of_memory_proves_nothing_about_what_it_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A load's sites are a lower bound, and are never what takes a proof
        // away. Each is a use after free that is proved in a hatch, where a
        // doubt is only listed, so losing the proof builds. Mutation: drop
        // `loaded` from `holds_something_unnameable`; the first builds.
        // Mutation: blank every site an opaque call reaches, as before; the
        // second builds.
        a_proved_use_after_free_in_a_hatch_stays_proved_after_a_free_of_a_load: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_proved_use_after_free_in_a_hatch_stays_proved_after_a_load_is_handed_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A live load is read in silence, since the marker is not a doubt of
        // its own. Mutation: count it as one; this reports.
        a_live_pointer_read_out_of_memory_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same marker read by a return and by an argument, which are asked
        // apart from a dereference. Mutation: let `verdict` settle with the
        // marker present; both become proofs.
        a_pointer_read_out_of_memory_after_a_free_is_not_proved_freed_when_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_memory_after_a_free_is_not_proved_freed_when_handed_on: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Reported, and what ADR-0045 accepts as its cost: what a table holds
        // is not told apart by slot and a store adds to it rather than
        // replacing it, so the live slot of a table whose other slot was freed
        // is doubted, and so is a slot stored again after what it held was
        // freed.
        a_live_slot_of_a_table_with_a_freed_one_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_slot_stored_again_after_what_it_held_was_freed_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer copied a byte at a time through `char`, which C17 6.5 p7
        // defines, is the pointer: a byte read out of memory is a load and
        // carries what was stored where it was read from. See ADR-0046, whose
        // Confirmation names the mutation each of these fails under.
        //
        // Reported, copied into a table, into a local whose address escaped,
        // and through a `char` temporary. Mutation: drop the byte from
        // `read_through`'s `loaded`; all three go silent.
        a_use_after_free_through_a_pointer_copied_a_byte_at_a_time_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_use_after_free_through_a_pointer_copied_a_byte_at_a_time_into_a_local_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_use_after_free_through_a_pointer_copied_a_byte_at_a_time_through_a_temporary_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Reported where the byte reaches what takes it as an operand rather
        // than through a local: added to nothing, `*d = *s + 0`, and handed to
        // a call that may keep it, `stash(*s)`. Mutation: leave the bytes out
        // of `built_from`'s loads, or out of `read_out`; each goes silent.
        a_use_after_free_through_a_pointer_copied_a_byte_at_a_time_by_arithmetic_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_use_after_free_through_bytes_handed_to_a_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Built, which is what keeps the byte out of `may_be_pointer`: a
        // character handed to a call this check cannot read is not everything
        // stored anywhere. Mutation: answer `true` for `Ty::Char` there; both
        // doubt a use after free and a double free at the table. Warnings
        // rather than refusals, because both run with `--allow-unknown`: each
        // reads a pointer out of the table in place, which the nullability
        // check doubts (#333).
        a_character_handed_to_a_call_beside_a_table_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_string_copied_a_byte_at_a_time_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // Built: a byte handed straight to a call reaches what was stored
        // where it was read, here nothing, and never everything stored
        // anywhere. Mutation: answer a byte in `read_out` with
        // `Known::stored`; this doubts a use after free and a double free at
        // the table, as warnings for the reason the two above give.
        a_byte_handed_straight_to_a_call_beside_a_table_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
    }

    "returns" => {
        // A pointer a function returns, asked at its `return` about every
        // allocation it may hold. See ADR-0041, whose Confirmation names the
        // mutation each of these fails under.
        //
        // Reported at the return, proved or not, and nothing at the caller's
        // read, which believes a call's result is live.
        a_function_that_returns_what_it_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_return_after_a_free_on_one_arm_only: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_freed_and_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_escaped_local_freed_and_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Out of a nested scope, whose storage the `return` ends after the
        // write into the return place. Mutation: in the lowering's `Return`
        // arm, emit the `StorageDead`s before that write; `p` is cleared
        // before `returned` asks about it and this goes silent.
        a_pointer_freed_and_returned_out_of_a_nested_scope: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A free of a two-member set, returned. Mutation: drop
        // `Reached::SetFreed` from what `returned` asks; this goes silent.
        a_free_of_either_of_two_allocations_then_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer kept from last turn of a loop names no allocation, which is
        // ADR-0018's `Lost`. Mutation: change the `Lost` row's words in
        // `memory_finding`; this fails, and nothing else reaches that row.
        a_pointer_kept_from_the_last_turn_of_a_loop_and_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A parameter's allocation is asked about like any other. A caller that
        // dereferences the result doubts it too, and one that hands it on as an
        // argument doubts it at the call (ADR-0042). Mutation: drop a
        // parameter's site from what `returned` asks unless it is
        // `SiteState::Freed`; all three lose the report at the return.
        a_parameter_freed_on_one_arm_is_doubted_at_its_return_and_by_its_caller: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_freed_on_one_arm_and_handed_on_by_its_caller: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_freed_then_handed_to_a_call_and_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Reported, and what ADR-0041 accepts as its cost: an allocation handed
        // to a call this check cannot read, a parameter returned after one, and
        // a parameter whose address was taken, each returned with no free in
        // the function at all.
        an_allocation_handed_to_a_call_and_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_returned_after_a_call_this_check_cannot_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_whose_address_escaped_returned_without_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Not asked about: an integer.
        an_integer_built_from_two_calls_is_not_asked_about_at_its_return: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Only the write into the return place is a return. Mutation: drop the
        // return-place test in `returned`; the copy into `q` is reported.
        a_freed_pointer_copied_but_not_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What leaves is `*tab`, a pointer read out of memory, and the read of
        // freed `tab` is `SC0402`'s. Mutation: ask a projected source in
        // `returned`; this gains an `SC0406` about `tab`.
        a_pointer_read_out_of_a_freed_table_and_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_table_read_back_after_a_loop_allocated_it_again_holds_what_it_held: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],

        // A pointer read out of memory and returned without passing through a
        // local is asked what it holds, as the same through a local is. See
        // ADR-0045. Mutation: have `returned` return early for a source with
        // a projection again; this goes silent.
        a_pointer_read_out_of_memory_and_returned_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same two levels down. Mutation: have `handed_reached` read one
        // level whatever the depth; this goes silent.
        a_pointer_read_two_levels_down_and_returned_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And a doubt about reading `*tab` is kept over one about what it
        // returns, as at a call, since the read comes first. Mutation: run
        // `returned` before `used`; this reports `SC0406` instead. Mutation:
        // have `returned` push rather than go through `say`; this reports
        // both at one caret.
        a_doubted_read_of_a_table_is_kept_over_what_it_returns: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    }

    "calls" => {
        // A pointer handed to a call is asked as a dereference of it would be,
        // whatever the callee is and wherever it is defined. See ADR-0042.
        // Mutation: drop the call to `handed` in `memory::findings`; the first
        // five go silent.
        a_freed_pointer_handed_to_a_function_defined_in_the_file: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_freed_pointer_handed_to_a_function_only_declared: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_freed_pointer_handed_to_a_function_that_frees_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_freed_and_handed_on: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: drop `Callee::ReturnsFirst` from the callees
        // `handed_places` answers.
        a_freed_pointer_handed_to_memcpy: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Doubts, which fail the build. Mutation: report only proofs in
        // `handed`; both go silent, and the callee believes what it was handed.
        a_pointer_freed_on_one_arm_and_handed_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_a_call_was_handed_is_doubted_when_handed_on: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What ADR-0042 accepts as its cost: a parameter handed to a second
        // call this check cannot read, with no free in the function at all.
        a_parameter_handed_to_a_second_call_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: drop the pointer-type condition in `handed_places`; this is
        // refused.
        an_integer_built_from_two_calls_is_not_asked_about_as_an_argument: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C may call `use` before the free. Mutation: answer `true` for
        // `Kind::ArgumentAfterFree` in `verdict`'s `ordered`; this becomes a
        // proof.
        a_call_unsequenced_with_a_free_is_not_proved_to_be_handed_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `say` collapses the second report as well, so the mutation is to drop
        // the repeated-local test in `handed_places` *and* push in `handed`;
        // two reports. Either alone leaves this green.
        a_freed_pointer_handed_twice_to_one_call_is_one_report: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Every argument is asked, not only the first: the freed pointer comes
        // after a constant and a live pointer. Mutation: walk
        // `arguments.iter().take(1)` in `handed_places`, or turn its
        // `continue`s into `break`s; this goes silent.
        a_freed_pointer_handed_after_other_arguments_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A dereference and an argument at one caret are two reports, the
        // dereference first. Mutation: call `handed` before the terminator's
        // `used` in `memory::findings`; the two change places.
        a_freed_pointer_handed_and_read_through_at_one_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What is handed is `*tab`, a pointer read out of memory, and the read
        // of freed `tab` is `SC0402`'s. Mutation: ask a projected argument in
        // `handed_places` and push in `handed`; this gains an `SC0407` about
        // `tab`. With `say`, the dereference's report at the call already holds
        // the key, so asking it alone leaves this green.
        a_pointer_read_out_of_a_freed_table_and_handed_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a call is handed is carried to a later call the same full
        // expression leaves unordered against it, as a dereference is: C may
        // run the free, the opaque call or `realloc` before `memset`. Mutation:
        // drop the loop that records `Read::Argument` in
        // `Allocations::terminator`; all three go silent. Mutation: answer
        // `Kind::UseAfterFree` for `Read::Argument` in `used_before`; all three
        // become `SC0402`.
        a_pointer_handed_to_a_call_the_check_meets_first_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_handed_to_a_call_before_an_opaque_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_handed_to_a_call_before_realloc_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A read carried to a later call this check cannot read is asked about
        // what that call may free beyond what it was handed: a parameter, and a
        // local stored where one points, were reachable to it before the read.
        // Each swapped spelling reported already. Mutation: answer `false` for
        // `beyond_its_arguments` in `used_before`'s filter, so only the
        // arguments are asked; all three go silent. Mutation: start
        // `PendingRead::reachable` empty in `Known::meeting`; all three go
        // silent. Mutation: read `exposed` as it stands rather than
        // `reachable_now` in `Known::meeting`; the second goes silent, because
        // `*t = a` marks nothing exposed until the next call.
        a_read_carried_to_an_opaque_call_that_reaches_it_by_exposure_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_of_a_local_stored_where_a_parameter_points_is_asked_at_a_later_opaque_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_handed_to_a_call_before_an_opaque_call_that_reaches_it_by_exposure_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `strlen(s)` has made `s` unknown by the time `strlen(t)` is asked,
        // and C may run `strlen(t)` first. Mutation: drop `read.reachable` from
        // the filter in `used_before`; the report at `strlen(s)` goes.
        each_of_two_calls_in_one_expression_is_asked_about_what_the_other_may_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other side: an allocation no call was handed and nothing stored
        // is not one a call this check cannot read may free. Mutation: count
        // every site as taken in `used_before`; this reports.
        a_read_of_an_allocation_nothing_exposed_is_not_asked_at_a_later_opaque_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A free and `realloc` free only what they are handed, so `a`, which
        // the store made reachable, is not asked at a free of `b`. Mutation:
        // answer `true` for `Callee::Frees` in `used_before`'s
        // `beyond_its_arguments`; the first reports `a` with `freed here` on
        // `free(b)`. Mutation: the same for `Callee::Reallocates`; the second
        // reports.
        a_read_is_not_asked_at_a_free_of_another_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_is_not_asked_at_realloc_of_another_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What the read's own call exposes is not reachable to a sibling call
        // before the read, since C orders a call's arguments before its body,
        // and nor is what a call enclosing that one exposes, which runs later
        // still. The third does report, about `keep`'s own argument, which
        // `memset` made reachable before `keep` ran; what it holds is that
        // `memset`'s argument is not told what `keep` exposes. Mutation: tell
        // every pending read in `Known::noticed`, owned or not; each of the
        // three gains a report. Mutation: own a read only by `inside`; the
        // same, through the pointer each call was handed. Mutation: own it only
        // by `entry.at == by`; the third alone gains one. The dereference in
        // the second is ordered before `keep2` by
        // `Element::ArgumentsEvaluated`, so it is not pending there.
        a_call_is_not_asked_about_an_allocation_only_the_read_s_own_call_exposed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_dereference_in_a_call_s_arguments_is_not_asked_about_what_that_call_exposed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_enclosing_the_read_s_own_call_does_not_make_it_reachable_to_a_sibling: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A read is told what an event makes reachable, not everything exposed
        // by then, which after `keep(a)` includes `a`. Mutation: in
        // `Known::expose`, tell it `reachable_now` once the new sites are
        // marked rather than the closure of those sites; `memset(b, ..)` tells
        // `keep`'s read about `a`, and this reports.
        a_call_exposing_another_allocation_does_not_make_the_read_s_reachable: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `release(tab)` reaches `a` through what `tab` holds, whatever exposed
        // it. Mutation: drop `own_reach` from the filter in `used_before`; this
        // goes silent.
        a_later_call_is_asked_about_what_it_reaches_through_what_it_is_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a later call reaches by itself includes what a pointer it reads
        // out of memory may be, and what every local whose address escaped
        // holds. Mutation: build `used_before`'s `own_reach` from `reach_of`
        // alone; the first goes silent. Mutation: from the arguments' sites and
        // `read_out` alone; the second goes silent.
        a_later_call_is_asked_about_what_a_pointer_it_reads_out_of_memory_reaches: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_later_call_is_asked_about_what_a_local_whose_address_escaped_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Something other than the read exposes `a` between it and the later
        // call, and C may run that and the later call first. `memset`'s own
        // argument is not reported, because only `memset` itself makes `a`
        // reachable to `release_all`. Mutation: drop the `noticed` in
        // `Known::expose`; the first goes silent. Mutation: drop the one after
        // a store into a reachable allocation in `Allocations::element`; the
        // second loses its report at `a[0]`, keeping the one at the write
        // through `t`.
        a_read_is_asked_about_what_another_call_in_the_expression_exposed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_is_asked_about_what_a_store_in_the_expression_exposed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What an event makes reachable is closed over what it holds: `memset`
        // exposes `tab`, and so `a` stored in it; storing `tab` into `box`,
        // which an earlier call exposed, does the same. The first is not asked
        // by `memset` itself, which frees nothing. Mutation: tell a read
        // `sites` rather than their closure in `Known::expose`; the first goes
        // silent. Mutation: the same for the store in `Allocations::element`;
        // the second loses its report at `a[0]`.
        a_read_is_asked_about_what_a_call_exposes_through_what_it_is_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_is_asked_about_what_a_store_makes_reachable_through_what_it_carries: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A store into an allocation nothing reaches makes nothing reachable.
        // Mutation: notice after every placed store in `Allocations::element`,
        // whatever its container; this reports.
        a_store_into_an_allocation_nothing_reaches_makes_nothing_reachable: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The exposure is on one arm. Mutation: keep one arm's `reachable` in
        // `Allocations`'s `join`; this goes silent.
        an_allocation_exposed_on_one_arm_is_reachable_to_a_read_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A hatch may free anything, as the forward walk already says after
        // one. Mutation: drop `anything` from the filter in `used_before`; this
        // goes silent.
        a_read_carried_to_a_hatch_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call this check cannot read is carried forwards as a read is, so an
        // exposure walked after it asks a read walked before it: C may run the
        // exposure, then the call, then the read. A store into an escaped
        // local, a library call that exposes what it is handed, a write into an
        // escaped target, and a read walked first. Mutation: record no call in
        // `Allocations::terminator`; all four go silent. Mutation: no notice
        // where an escaped local is assigned in `Allocations::element`; the
        // first and fourth go silent. Mutation: no notice for `targets` there;
        // the third goes silent.
        a_store_into_an_escaped_local_after_a_call_is_asked_of_a_read_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_library_call_exposing_after_a_call_is_asked_of_a_read_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_into_an_escaped_target_after_a_call_is_asked_of_a_read_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_before_a_call_and_an_exposure_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other side. An exposure of another allocation asks nothing; a
        // read the exposing call owns, or the pending call owns, is ordered
        // before it. Mutation: do not leave `by`'s own read out in
        // `Known::noticed`; the first two report. Mutation: do not leave the
        // pending call's own read out; the third reports.
        an_exposure_of_another_allocation_after_a_call_is_not_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_an_exposing_call_owns_is_not_asked_about_a_call_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_a_pending_call_owns_is_not_asked_about_an_exposure_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An exposure under `?:`, and a call pending on either arm. Mutation:
        // keep one arm's `after_call` at the join; the first goes silent, the
        // read being reported only where the marker clears it. Mutation: keep
        // one arm's calls at the join; the second goes silent, its call being
        // on the arm the join does not keep, and the third is the other order.
        an_exposure_on_one_arm_after_a_call_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_on_the_first_arm_is_pending_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_on_the_second_arm_is_pending_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call in the exposing call's arguments runs before it, so it cannot
        // free what that call exposes. Mutation: do not leave a call inside
        // `by`'s span out in `Known::noticed`; this reports.
        a_call_in_the_exposing_call_s_arguments_is_not_asked_about_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A read the walk meets after both the call and the event: the event
        // records what it made reachable while a call was pending, and the read
        // asks that when it is met. A write into an escaped target, and the
        // one-line `release_all() + use(memset(a, 0, 4))`. Mutation: record
        // nothing in `Known::exposed_after_call`; both go silent. Mutation: do
        // not ask it in `Known::meeting`; both go silent.
        a_read_met_after_a_call_and_a_write_into_an_escaped_target_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_library_call_exposing_inside_a_call_after_another_is_asked_of_its_result: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What C orders is not paired. A read and a call on exclusive arms of a
        // `?:` meet only at the join, and an escaped local's notice there would
        // pair them. Mutation: notice in `Known::unproved` rather than at the
        // assignment; this reports. A call in a store's own operands is ordered
        // before the store by C17 6.5.16 p3. Mutation: give a store's notice no
        // span; the second reports.
        a_read_and_a_call_on_exclusive_arms_are_not_paired_at_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_in_a_store_s_own_operands_is_not_asked_about_the_store: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Reported where the marker clears what is pending, so a free in the
        // same expression reports first, with `freed here`, and an exposure by
        // a call is reported though the next block starts by clearing the
        // reads. Mutation: report after every element instead; the first loses
        // its label. Mutation: report at no `ArgumentsEvaluated`; the second
        // goes silent.
        a_free_in_the_same_expression_keeps_its_label_over_a_pending_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_exposure_by_a_call_whose_next_block_clears_the_reads_is_still_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What was made reachable while a call was pending may be freed by the
        // time the expression ends, so a read in the next statement is asked.
        // Mutation: leave those sites as they were at `Element::Sequenced`; the
        // first goes silent. On one arm only, it survives the join. Mutation:
        // keep one arm's set at the join; the second goes silent, and the third
        // is the other order. Taking a local's address is an exposure too.
        // Mutation: no notice at the address taken; the fourth loses its report
        // at the read, keeping the one the escape makes.
        a_read_in_the_next_statement_after_an_exposure_and_a_call_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_exposure_on_one_arm_after_a_call_is_asked_of_a_read_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_exposure_on_the_other_arm_after_a_call_is_asked_of_a_read_after_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_address_taken_after_a_call_is_asked_of_a_read_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call in a write's own operands is ordered before the write by C17
        // 6.5.16 p3. Mutation: give the notice for a write into an escaped
        // target no span; this reports.
        a_call_in_a_write_s_own_operands_is_not_asked_about_the_write: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C sequences `p[0]` before `g`, and this is still refused: under `=`
        // the `?:` gets no sequence point, which is #178, so the read is
        // pending at `g`. This holds today's answer so that closing #178 moves
        // a named case. See ADR-0042.
        a_read_sequenced_before_an_opaque_call_under_an_assignment_is_still_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The argument crosses a join before the free. Mutation: answer
        // `Read::Dereference` in the `or_insert` of `Allocations`'s `join`;
        // this becomes `SC0402`.
        a_pointer_handed_to_a_call_inside_one_arm_before_a_free_survives_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `a` escaped, so `handed` doubts it at the call and `used_before`
        // doubts it again from the free, at the same caret. Mutation: push in
        // `handed` rather than going through `say`; two `SC0407` about `a` at
        // one caret. The second names the free and is the one that stands.
        // Mutation: have `supersedes` answer `false` for `(None, Some(_))`;
        // the report loses `freed here` and the p10 note.
        an_escaped_pointer_handed_to_a_call_before_a_free_is_one_report: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `g(*p)` is doubted twice at one caret, by the call `h` this check
        // cannot read and by the free, and only the second names a free. Both
        // orders are written, because which doubt arrives first is which
        // operand comes first. Mutation: have `supersedes` answer `false` for
        // `(None, Some(_))`; `call_first` loses `freed here` and the p10 note
        // on `g(*p)` and `h(p)`, and `free_first` does not move. Mutation:
        // answer `true` for `(Some(_), None)`; `free_first` loses them on
        // `g(*p)` instead. Mutation: keep the first of two suspicions
        // whatever they carry; `call_first` loses them.
        a_read_beside_a_call_and_a_free_is_told_the_same_whichever_is_written_first: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `g(*p)` is doubted at one caret by each of two frees, and both doubts
        // name their free. The inner `free(q)` runs first, as an argument, and
        // is written later; the doubt naming it arrives first and stands, so
        // `freed here` agrees with the `SC0401` on the same line. Mutation:
        // have two suspicions name the earlier span, as two proofs do; `g(*p)`
        // names the outer free, which the next report calls freed again.
        a_read_beside_two_frees_names_the_one_that_runs_first: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The read of `*tab` is doubted because `tab` may be freed, and what
        // it hands `release` is doubted because `q` may be, at one caret. The
        // second names a free and is a different kind, and the first stands.
        // Mutation: drop the kind arm from `supersedes`; the caret becomes an
        // `SC0407` about `q`.
        a_doubt_about_a_table_is_kept_over_a_doubt_about_what_it_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two reports about one full expression, and each is its own order: `g`
        // may free `a` before the free, which is the forward `SC0401`, and the
        // free may run before `g` reads `a`, which is the carried `SC0407`. The
        // second is the only thing the carry adds after a call this check
        // cannot read, because the first already fails the build. Mutation:
        // record `Read::Argument` only for `Callee::ReturnsFirst` in
        // `Allocations::terminator`; the `SC0407` goes and the exit code stays.
        an_argument_of_a_call_this_check_cannot_read_is_carried_to_a_later_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a call's own arguments read is behind it, C17 6.5.2.2 p10's
        // first sentence, wherever the call sits; ADR-0026's element says so
        // only for a call no unsequenced operator encloses, and these are the
        // calls below one. The first has no free in it at all. Mutation: drop
        // the `inside` test in `used_before`; all four are refused again, the
        // last with the `SC0402` its own note used to contradict, and the case
        // after them gains reports. See ADR-0043.
        a_call_nested_in_an_argument_is_ordered_before_the_call_around_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_what_a_nested_call_returns_is_ordered_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_in_an_argument_below_an_assignment_is_ordered_before_its_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_in_a_frees_own_argument_below_an_assignment_is_ordered_before_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The read is skipped at the call around it and not dropped: `strcpy`'s
        // argument still reaches the free beside `strlen`. Mutation: drop the
        // reads inside a call's span from `pending` at that call's transfer, in
        // `Allocations::terminator`, as well as skipping them in `used_before`;
        // the `SC0407` at `strcpy` goes, and nothing else fails.
        a_nested_call_is_still_carried_to_a_free_beside_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],

        // A pointer read out of memory and handed to a call without passing
        // through a local is asked what it holds, at the call and at a later
        // free the same full expression leaves unordered against it, as the
        // same through a local is. See ADR-0042 and ADR-0045.
        //
        // Mutation: have `handed_places` admit no projected argument again;
        // the first, the second and the fourth go silent. Mutation: have
        // `handed_reached` read one level whatever the depth; the second goes
        // silent. Mutation: have the terminator carry `reached_by(local)`
        // for an argument; the fourth goes silent. Mutation: drop `Read`
        // from `ReadKey`; the fourth reports `SC0402` instead. Mutation:
        // count `Reached::Partial` as a doubt; the third reports.
        a_pointer_read_out_of_memory_and_handed_on_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_two_levels_down_and_handed_on_after_a_free_is_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_live_pointer_read_out_of_memory_handed_on_and_returned_is_silent: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_memory_and_handed_on_is_carried_to_a_later_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A table and what it holds, handed to one call, are two places and
        // both are asked. Mutation: have `handed_places` ask one place per
        // local again; this goes silent, since the table is live. And it is
        // one report, with `*tab`'s own words, though `tab` holds a freed
        // pointer one level in. Mutation: key that finding under `tab`
        // rather than `*tab`; two reports. Mutation: ask it in the same pass
        // as each argument's own question; it takes `*tab`'s words.
        a_table_and_what_it_holds_handed_to_one_call_are_both_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a pointer handed to a call points at, one level in, is asked
        // about allocations proved freed: through a local's address, and
        // through this function's own memory. See ADR-0042. Mutation: have
        // `handed` never ask `handed_below`; the first two go silent.
        // Mutation: have `Known::unproved` make a freed site `Unknown` again;
        // the first goes silent. Mutation: have `handed_below` read edges to
        // locals only; the second goes silent. Mutation: leave out
        // `Reached::Partial`; both become proofs.
        the_address_of_a_freed_pointer_handed_to_a_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_table_holding_a_freed_pointer_handed_to_a_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And about one unproven, through this function's own memory: a call
        // this check cannot read may have freed it, or one path did.
        // Mutation: have `handed_below` ask about freed sites only; both go
        // silent. Through an address, what is only reachable is not asked,
        // which the live address below holds. Mutation: have `handed_below`
        // ask about every unproven or reachable site; that one reports.
        a_table_holding_a_pointer_a_call_may_have_freed_handed_to_a_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_table_holding_a_pointer_freed_on_one_path_handed_to_a_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Through an address too, where what made it unproven may have freed
        // it: one path freed it, before or after its address was taken, or a
        // call was handed the pointer itself. See ADR-0047. Mutation: have
        // `handed_below` exempt `Unknown` through the address; all three go
        // silent. Mutation: have the join give `Reachable` for a freed side;
        // the first does. Mutation: have an opaque call write `Reachable` on
        // what it is handed by value; the third does.
        a_pointer_freed_on_one_path_and_handed_by_address_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_whose_address_was_taken_before_a_free_on_one_path_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_handed_to_a_call_and_then_by_address_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And where the call held the pointer as a load, or reached it
        // through memory it was handed: either may free it, and neither can
        // replace `a`. Mutation: have the opaque call leave out what it holds;
        // both go silent. Mutation: take what it is handed without what that
        // memory holds; the second does.
        a_pointer_a_call_was_handed_through_a_load_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_a_call_reached_through_memory_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And handed straight out of memory, `release(*t)`, which is no local
        // the call names. Found by review. Mutation: leave `read_out` out of
        // what the call holds; this goes silent.
        a_pointer_a_call_was_handed_read_out_of_memory_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And not where only an address or an exposure reached it: a call
        // handed an address may have replaced what it freed, which is how C
        // hands a pointer to be replaced. Mutation: have every producer but an
        // address taken write `Unknown`; both are refused, and nothing else.
        // Mutation: have the opaque call write `Unknown` on all it reaches,
        // escaped locals included; both are refused.
        a_pointer_handed_by_address_twice_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_handed_by_address_around_another_call_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call that reached an allocation through another local's address,
        // handed or stored in memory, may have freed it and cannot replace
        // `a`, so `use2(&a)` is asked. Mutation: drop the call to
        // `Known::held_out_of_reach`; both build. The in-out idiom through a
        // copy still builds, because only a holder read after the call
        // counts. Mutation: have its `live_after` answer `true`; both of the
        // last two are refused. See ADR-0047.
        a_pointer_a_call_reached_through_another_locals_address_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_a_call_reached_through_a_locals_address_in_memory_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_copy_grown_twice_by_address_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_dead_copy_does_not_doubt_a_pointer_grown_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A holder is exempt only where code this check cannot read may hold
        // its address, not wherever its address was taken: `int **pa = &a;`
        // goes nowhere. Mutation: have `Known::held_out_of_reach` test
        // `escaped` instead of `handed_away`; the first goes silent. The
        // second and third reach `a` through memory a call can reach, which
        // hands nothing since #350: what memory may hold is a may-set, so the
        // in-out idiom through it is asked. Mutation: mark the locals memory
        // a call reached may hold; both build. The fourth stores the address
        // through a load, into memory this check does not model, which hands
        // nothing either, for the reason the cases below give. Mutation: mark
        // the local the stored value certainly names; it builds. The fifth
        // hands `a` away on one arm only. Mutation: have the join union
        // `handed_away`; it goes silent. See ADR-0047.
        a_pointer_whose_address_only_a_local_holds_is_asked_after_a_call_reached_it_through_a_copy: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_grown_through_the_memory_its_address_was_stored_in_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_whose_address_is_in_exposed_memory_is_asked_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_whose_address_is_stored_through_a_load_is_asked_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_handed_away_on_one_arm_is_asked_on_the_other_after_a_call_reached_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A holder read by value after the call counts as one read by
        // address does, and a holder of another allocation does not count.
        // Mutation: have `live_after` count only a local whose address is
        // taken, `&& self.live_in[0][local]`; the first goes silent.
        // Mutation: have `Known::held_out_of_reach` ask whether the holder
        // holds any site rather than this one; the second is refused.
        a_copy_read_by_value_after_a_call_reached_it_through_another_address_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_reached_through_a_copy_of_another_allocation_does_not_doubt_this_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A slot in memory holding what the call reached is marked, and a
        // pointer read out of it is asked where it is handed by address; a
        // slot nothing reads again costs nothing. Mutation: drop the marking
        // in `Known::held_out_of_reach`; the first builds. Mutation: drop
        // `|| stale` in `Known::handed_below`; the first builds. Mutation:
        // turn the site `Unknown` for a slot instead; the second is refused.
        // See ADR-0047.
        a_pointer_copied_out_of_a_slot_a_call_could_not_replace_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_slot_nothing_reads_again_does_not_doubt_a_pointer_grown_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a load out of a marked allocation carries goes where the
        // pointer goes: a copy, a pointer to it, arithmetic, and a store into
        // another allocation. Mutation: have `Held::joined` drop
        // `unreplaced_read`; the first builds. Mutation: have
        // `Known::unreplaced_through` ask the local handed only; the second
        // builds. Mutation: have `Held::accumulated` drop it; the third
        // builds. Mutation: drop the mark in the store transfer; the fourth
        // builds. And the mark makes nothing lost, so a load through such a
        // pointer is still followed. Mutation: set `lost` where the mark is
        // read in `read_through`; the read of the freed `x` in the fifth goes
        // silent. See ADR-0047.
        a_copy_of_a_pointer_read_out_of_a_slot_a_call_could_not_replace_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_a_slot_a_call_could_not_replace_is_asked_through_a_pointer_to_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_built_from_one_read_out_of_a_slot_a_call_could_not_replace_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_slot_a_call_could_not_replace_copied_into_another_allocation_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_through_a_pointer_read_out_of_a_slot_a_call_could_not_replace_is_still_followed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And where paths meet and where the allocation is copied or grown.
        // Mutation:
        // have `Held::joined` intersect `unreplaced_read`; the first builds.
        // Mutation: have the join intersect `Known::unreplaced`; the second
        // builds. Mutation: drop the mark where `built_from` reads a load;
        // the third builds. Mutation: drop it in the `memcpy` arm, or where
        // `realloc` carries a row to the new site; the fourth and fifth build.
        // See ADR-0047.
        a_pointer_read_out_of_a_slot_a_call_could_not_replace_on_one_arm_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_slot_a_call_could_not_replace_on_one_arm_is_asked_by_address_after_the_arms_meet: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        arithmetic_on_a_load_out_of_a_slot_a_call_could_not_replace_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_slot_a_call_could_not_replace_copied_by_memcpy_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_slot_a_call_could_not_replace_grown_by_realloc_is_asked_by_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A place of dereferences handed to a call is asked what it points at
        // as its load would be. The first was silent. Mutation: have
        // `report::handed` ask only a plain local one level in; the first
        // builds. Mutation: in `Known::handed_below`'s `Reachable` arm, ask a
        // place of dereferences whatever reached it; the third is refused.
        // The fourth is one report, not two, because the place's report one
        // level in is keyed as the `**k` handed beside it. Mutation: key it
        // `*k` instead; a second `SC0407` appears.
        //
        // And a call is taken to hold an address only where the argument
        // certainly names one local, since that exempts a holder. A place of
        // dereferences never does, so the second is asked at its second call
        // (a report, where marking from memory was a silence). Mutation: have
        // `Known::handed_to_a_call` mark a place's level as well; the fifth
        // builds, `grow` having been handed only `&b`. Mutation: have it mark
        // every local a plain argument may name; the sixth builds. See
        // ADR-0045 and ADR-0047.
        a_freed_pointer_whose_address_is_read_out_of_memory_and_handed_on_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_whose_address_is_read_out_of_memory_and_handed_on_twice_is_asked_at_the_second_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_live_pointer_whose_address_is_read_out_of_memory_and_handed_on_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_place_and_what_it_points_at_handed_to_one_call_are_one_report: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_address_overwritten_in_memory_does_not_hand_its_old_local_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_address_a_local_may_hold_on_one_arm_does_not_hand_that_local_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A load out of memory names one local here, but `*other` may have
        // written `*k` behind this check's back, so the edge is not all of
        // it. `b` is a second holder asked whatever `writes_elsewhere` says,
        // so this decides nothing about that test on its own; the case with
        // no second holder below is what holds it. See ADR-0047.
        a_single_address_read_out_of_memory_another_pointer_may_write_does_not_hand_its_local_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What memory a call can reach may hold is not handed to it: a slot
        // keeps every address it was ever given in `inside_locals`, and the
        // second's slot holds null when `stash` sees it. Mutation: mark the
        // locals memory a call reached may hold; the first builds. Mutation:
        // mark them only where `inside_locals` names one local and `inside`
        // no site, the rejected half-measure; the second builds. The second's
        // report at `release_ref(&b)` is a false one C defines whatever the
        // callees do: `stash` saw null, yet `inside_locals` still puts `a` in
        // every call's reach, the cost #345 is about. See ADR-0047.
        an_address_memory_may_hold_does_not_hand_its_local_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_address_a_slot_no_longer_holds_does_not_hand_its_local_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Nor is an address a store puts in memory this check does not model:
        // the slot may be written again before any code outside sees it, and
        // an `unnamed` pointer may be this function's own memory on another
        // path. Mutation: mark the local a stored value certainly names in
        // the store transfer; both build. See ADR-0047.
        an_address_stored_in_memory_and_overwritten_does_not_hand_its_local_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_address_stored_where_memory_may_be_this_functions_own_does_not_hand_its_local_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What an argument must be to hand a call a local, each part of
        // `Held::certain_target` and the skip before it. Mutation: ignore
        // `writes_elsewhere`; the first builds. Mutation: answer the first of
        // several edges; the second builds. Mutation: let a place of
        // dereferences hand its own edges; the third loses its second
        // report. See ADR-0047.
        a_single_address_read_out_of_memory_another_pointer_may_overwrite_is_asked_with_no_second_holder: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_argument_that_may_be_either_of_two_unrelated_locals_hands_neither_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_place_of_dereferences_handed_to_a_call_hands_no_local: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What ADR-0042 accepts as its cost: a callee that only writes there
        // is not told apart from one that reads.
        the_address_of_a_freed_pointer_handed_to_a_call_that_only_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And a live pointer stored in a table after its address escaped,
        // whose allocation the escape left unproven.
        a_table_holding_a_live_pointer_whose_address_escaped_is_doubted_when_handed_on: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A load out of memory carries a local's address as an edge, which
        // exempts what the local holds; what the memory itself holds is
        // still asked. Found by review. Mutation: have `handed_below` exempt
        // every site the edges reach; this goes silent.
        a_freed_pointer_held_in_memory_beside_a_locals_address_is_asked_when_handed_on: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A free or `realloc` of what may be the caller's may free anything
        // the caller can see, as a call this check cannot read may: a caller
        // may hand one allocation twice. See ADR-0040. Mutation: have the
        // rule never fire; the first goes silent. Mutation: have it fire for
        // `free` only; the second goes silent. Mutation: have it ask no place
        // of dereferences; the third loses its report at the read. Silent
        // for a read before the free, and for a free of the function's own
        // allocation. Mutation: have it fire for any free; the last reports.
        a_read_through_one_parameter_after_another_is_freed_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_through_one_parameter_after_another_is_reallocated_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_what_a_parameter_points_at_doubts_a_read_through_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_read_through_a_parameter_before_another_is_freed_is_silent: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_this_functions_own_allocation_leaves_a_parameter_alone: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Silent: the remedy the report gives, and a live pointer handed by
        // address. Mutation: have `handed_below` ask every unproven site,
        // including those through the address; the second reports.
        the_address_of_a_freed_pointer_set_to_null_is_handed_on_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        the_address_of_a_live_pointer_handed_to_a_call_is_not_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a call this check cannot read handed over, through an
        // out-parameter or as memory its result points at, is memory the
        // callee may free at the next such call, as a parameter's is. Each is
        // a use after free under AddressSanitizer against a callee that frees
        // what it handed out (#394). Mutation: drop the marking after
        // `Known::replaced` in the `Callee::Opaque` arm; the first builds.
        // Mutation: drop the opaque-result clause from
        // `Allocations::reads_caller_memory`; the second builds.
        a_pointer_a_call_wrote_through_an_address_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_what_a_call_returned_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same pointers read back through the address of the local that
        // holds them, `pr = &r; s = *pr;`, which carried nothing the local was
        // marked with, and the parameter's own route had the same gap. Found
        // by review. Mutation: drop `Known::caller_through` from
        // `Allocations::reads_caller_memory`; all three build.
        a_pointer_a_call_wrote_read_through_its_address_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_what_a_call_returned_and_read_through_an_address_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_a_parameter_and_read_through_an_address_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read through what may be either an opaque result or this
        // function's own allocation, moved by arithmetic after the load, and
        // copied out by `memcpy` before it is read: each a way of reaching
        // `Allocations::reads_caller_memory` the plain load does not take.
        // Found by review. Mutation: ask that an opaque result be every site
        // read through rather than any; the first builds. Mutation: skip the
        // opaque-result clause where an assignment builds a value from
        // operands; the second builds. Mutation: skip it where `memcpy`
        // reads its source; the third builds.
        a_pointer_read_out_of_what_may_be_a_call_s_result_or_this_function_s_allocation_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_moved_off_one_read_out_of_what_a_call_returned_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_out_of_what_a_call_returned_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A free through what a call returned frees memory the callee handed
        // over, as a free through a parameter frees the caller's, so a pointer
        // read out of it before the free is doubted after. Mutation: skip the
        // opaque-result clause in `Allocations::frees_callers`; the use is no
        // longer reported, and only the free is.
        a_free_through_what_a_call_returned_doubts_a_pointer_read_out_of_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `memcpy` into a local's address is a store into that local, and
        // lands there what a load of its source holds, every mark included:
        // the handed-over mark of a callee's or a caller's memory, and the
        // allocation itself, which was lost, so `*s` after `free(r)` said
        // nothing (#395). In the two whose allocation is freed, the `SC0401`
        // at the one `free(r)` and the `SC0403` at the use are older than
        // this: `&r` handed to `memcpy` is a call reaching `r`, and the
        // nullability check never trusts an escaped local; the `SC0402` at
        // `*s` is what each pins. Mutation: drop the union into the target
        // locals in the `Callee::Copies` arm; all four but the silent one
        // stop reporting the use. Mutation: take the targets only for a
        // destination of no dereference;
        // `..._through_a_pointer_to_a_local_...` stops reporting it. The
        // silent case is a limit, not a miss: what a callee handed over is
        // asked about only at a later call that may free it (ADR-0040).
        // Mutation: make what the copy lands lost at once; it reports.
        a_pointer_a_call_wrote_copied_by_memcpy_into_a_local_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_by_memcpy_out_of_a_parameter_into_a_local_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_by_memcpy_into_a_local_is_asked_after_its_allocation_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_by_memcpy_through_a_pointer_to_a_local_is_asked_after_its_allocation_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_a_call_wrote_copied_by_memcpy_and_handed_on_with_no_later_call_is_silent: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same copies with the source declared `void *`, which is how
        // `memcpy` is usually called: the copy is of bytes, so what lands does
        // not ask the source's type, and asking a load for it carried nothing
        // from one. Found by review. Mutation: build what lands from a typed
        // load of the source, `Allocations::read_through`; both build.
        a_pointer_a_call_wrote_copied_by_memcpy_from_a_void_pointer_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_by_memcpy_out_of_a_void_pointer_parameter_is_doubted_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other shapes a copy into a local takes, each a use `*s` or
        // `*slot` the copy has to land for: a source that is itself a place
        // of dereferences, `memcpy(&s, *ppr, 8)`; a destination two
        // dereferences down, `memcpy(**ppps, &r, 8)`; a local's address
        // copied and then stored through, which lands as an address edge
        // rather than a site; and `memmove`, which shares the arm. Found by
        // review. Each mutation below stops the named case reporting its
        // use. Read the source at its own local, dropping its dereferences:
        // `..._out_of_a_place_of_dereferences_...`. Read the destination's
        // level at most one dereference down: `..._through_two_pointers_...`.
        // Drop the address edges from what lands:
        // `a_local_s_address_copied_by_memcpy_...`. Read `memmove` as an
        // opaque call: `..._by_memmove_...`.
        a_pointer_copied_by_memcpy_out_of_a_place_of_dereferences_is_asked_after_its_allocation_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_by_memcpy_through_two_pointers_to_a_local_is_asked_after_its_allocation_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_s_address_copied_by_memcpy_carries_a_store_through_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_by_memmove_into_a_local_is_asked_after_its_allocation_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What `malloc` returns is this function's, not handed over, so a
        // pointer read out of it is not doubted at a later call; the
        // `SC0403`s are the nullability check's. Mutation: count
        // `Callee::Allocates` as handed over in
        // `Allocations::opaque_results_of`; a use is reported.
        a_pointer_read_out_of_this_function_s_own_table_is_not_handed_over: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // With no later call nothing is asked, so the output-parameter idiom
        // ADR-0017 declines to report stays unreported; its `SC0403` is the
        // nullability check's, which never trusts an escaped local. Mutation:
        // make what was handed over lost at once rather than at the next
        // call, through either route; the control for that route fails.
        a_pointer_read_out_of_what_a_call_returned_is_read_in_silence_with_no_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_a_call_wrote_through_an_address_is_not_doubted_freed_with_no_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    }

    "frontend" => {
        // What the frontend makes of a program: its tokens, its tree, and every
        // report from lexing, parsing or type checking, `SC0101` to `SC0399`,
        // whatever artifact the case asks for.
        a_block_declaration_carries_its_initializer: ["--emit", "ast"],
        a_block_declaration_does_not_leave_its_block: ["--emit", "ast"],
        a_braced_initializer_is_refused: ["--emit", "ast"],
        // The two codes a constant this compiler cannot read is reported under,
        // and `--emit safety-ir` rather than `--emit ast` because what each one
        // pins is that there is exactly one report on a run asked to go past
        // the type check. `types.rs` says it, and the driver lowers nothing
        // after it. A value `int` does not hold is this compiler's gap, and
        // `clang 20.1.6 -std=c17 -pedantic-errors` compiles this program; a
        // spelling that is no constant at all is the program's, and `clang`
        // refuses it too.
        a_constant_no_integer_type_here_can_hold: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_spelling_that_is_not_a_constant: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The suffix is refused rather than read, and the note says why: C
        // computes `-6 / 3u` as an unsigned division. Reading `3u` as an `int`
        // compiled this program to a signed division and reported nothing,
        // which is the one shape this stage exists to stop.
        a_suffixed_constant_has_no_type_here: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_comma_in_a_controlling_expression: ["--emit", "ast"],
        a_dangling_else: ["--emit", "ast"],
        a_definition_that_is_not_a_function: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A parameter the IR cannot hold. A pointer to an array, because an
        // array or a function parameter is a pointer by C17 6.7.6.3 p7 and p8
        // and the IR holds that.
        a_function_the_ir_cannot_hold: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An argument is checked against the parameter as C adjusts it: an
        // `int` given to `int a[4]` is given to an `int *`, and to `int
        // h(void)` an `int (*)(void)`. Mutation: leave a parameter's type as
        // written; both calls build.
        an_argument_is_checked_against_an_adjusted_parameter: ["--emit", "ast"],
        // C17 6.7.6.2 p1, once per declaration and wherever it is written: an
        // element that is not a complete object type, a length that is not an
        // integer, a literal length that is not greater than zero, at file
        // scope, in a block, in a parameter list, in one inside a pointer, in
        // what a function returns, and in a second declarator of one
        // declaration. `ok2`, whose length is a `char`, and the last four
        // declarations are the controls. Mutation: stop refusing
        // an element of unknown length, a `void` one, or a function one; the
        // `q` and `x`, the `r`, `g` and `nest`, or the `fp` report goes.
        // Mutation: stop asking a length's type, or its value; the `k`, or the
        // `z`, `h` and `w`, report goes. Mutation: refuse every element; the
        // controls report. Mutation: skip parameter lists; the parameter
        // reports go. Mutation: report every array of a declaration; `z` is
        // two. Mutation: stop at a function's return type; the `fr` report
        // goes. Mutation: ask only a declaration's first declarator; the `b1`
        // report goes. Mutation: refuse a `char` length, or take a `void` one;
        // `ok2` reports, or the `y` report goes.
        an_array_declarator_is_held_to_its_constraints: ["--emit", "ast"],
        // C17 6.7.9 p3: a function, `void` and a variable length array are
        // refused an initializer at file scope and in a block, each once under
        // `SC0310`, and an array of unknown size or of a constant length is
        // not, which p3 allows. `u`, `s`, `bu` and `bs` are still accepted in
        // silence: 6.7.9 p16 asks for a brace-enclosed list there, which is a
        // Semantics paragraph and not this check's. Mutation: stop asking
        // whether the declared type is a function, or `void`; `f` and `bf`,
        // or `v` and `bv`, go. Mutation: stop asking whether an array's
        // length has a value; `va` goes, and asking only the outermost one
        // lets `vb` through, whose inner length makes it a variable length
        // array too (6.7.6.2 p4); `vp` points at one and is not one, so it
        // may be initialized. Refusing every array with a
        // length makes `s` and `bs` report, and every array, `u` and `bu`
        // too. Mutation: walk only the items, or only the statements; the
        // block's reports, or the file's, go. Mutation: ask only a
        // declaration's first declarator; `second` and `bsecond` go. `fv` is
        // a variable length array at file scope, so 6.7.6.2 p2 refuses its
        // declarator under `SC0309` as well, and both are reported; dropping
        // the file-scope walk's arrays loses its `SC0310`.
        only_an_object_can_be_initialized: ["--emit", "ast"],
        // C17 6.7.2.1 and 6.5.2.3 are read: a struct definition with its
        // members, a declaration of a tag and nothing else, an object, a
        // parameter, a local, and `.` and `->`, each in the tree. The type
        // checker refuses every one under `SC0304` until #27 says what they
        // mean, the two declarations with no declarator included, and `g`,
        // which holds no struct, is not reported. Mutation: refuse `struct` in
        // `specifiers` again; this is `SC0201` at the first `struct`.
        // Mutation: let a parameter's type past the gate, or skip a member
        // access; a report goes. Mutation: read `->` as `.`; the dump's
        // `"->"` goes. `h` assigns one pointer to a struct to another, and
        // only its parameters are reported: a name whose type holds a struct
        // is given no type. Mutation: give it its type; `a = b` is also
        // refused as `SC0302`, since nothing yet says two structs are one.
        // The rest is every place a struct is read: two members in one
        // declaration, a definition with a declarator at file scope and in a
        // block, a tag declared in a block, a `for` and a parenthesised
        // declarator beginning with `struct`, a struct with no tag, an array
        // of one and a function returning one, each used. Mutation: stop
        // reading a member list's `,`; `int a, b` is `expected ;`. Mutation:
        // test a `for` or a parenthesised declarator for a specifier with
        // `specifier` alone; the `for` or `abstract`'s parameter is a syntax
        // error. Mutation: let `holds_a_struct` stop at an array or a
        // function; `many[0]` or `make()` gets a struct type, and `.x` on it
        // is refused as needing a struct, which it is.
        a_struct_is_read_and_refused_until_it_means_something: ["--emit", "ast"],
        // A struct with no tag declares nothing when nothing else is
        // declared (6.7 p2). Mutation: let any struct through without a
        // declarator; this loses its `SC0206`.
        a_struct_with_no_tag_and_no_declarator_declares_nothing: ["--emit", "ast"],
        // A declaration with no declarator has only its specifiers' type, and
        // the gate asks it, so a struct whose member breaks a constraint is
        // refused by the gate rather than accepted. The member itself is not
        // checked: nothing below a struct is, until #27. Mutation: leave a declaration's `specified`
        // type out of the gate; this exits 0.
        a_struct_whose_member_breaks_a_constraint_is_still_refused: ["--emit", "ast"],
        // 6.7.2.1 p1's grammar asks for a member, and `clang
        // -pedantic-errors` calls `struct S {};` a GNU extension. Mutation:
        // accept a `}` before any member; this loses its `SC0201`.
        a_struct_with_no_members_is_refused: ["--emit", "ast"],
        // Three hundred structs each the only member of the one around it,
        // past the parser's nesting limit, so a member list goes through
        // `Parser::deeper`. Mutation: read members without `deeper`; this is
        // a struct as deep as the source, which the parser does not refuse.
        a_struct_nested_past_the_limit_is_refused_rather_than_read: ["--emit", "ast"],
        // Two hundred and fifty structs, each reached from the one around it
        // through twenty `*`s. The parser bounds the two nestings apart, so
        // a walk of the type that recursed through both went the product of
        // them deep and overflowed the stack, with nothing reported.
        // Mutation: make `sema.rs`'s `walk_type` recurse again; the process
        // dies and this fails.
        a_struct_nested_through_long_pointer_chains_is_walked_without_recursion: ["--emit", "ast"],
        // `x` and `y` share one definition, so its member's undeclared `n` is
        // reported once, and `struct T`, declared with no declarator, still
        // has its `k` resolved. Walking the definition per declarator
        // reported `n` twice, and nested, took time exponential in the
        // depth. Mutation: drop `walked`; `n` is reported twice. Mutation:
        // skip a declaration's `specified` in sema; `k`'s report goes.
        a_struct_definition_shared_by_declarators_is_walked_once: ["--emit", "ast"],
        // `struct T` is defined inside a member and its own member `y` is
        // printed under `t`, the first member that reaches it, and not again
        // under `u`, which shares it. Mutation: drop `seen` in
        // `dump_fields`; `y` is printed twice. Mutation: print only the
        // specifiers' struct's members; `y` goes.
        a_struct_defined_inside_a_member_is_printed_once_where_it_is_defined: ["--emit", "ast"],
        // `struct` with no tag and no member list names nothing. Mutation:
        // accept it; this loses its `SC0201`.
        a_struct_with_neither_tag_nor_members_is_refused: ["--emit", "ast"],
        // A member's pointer promises nothing yet, so a nullability
        // specifier in one is refused as in a block. Mutation: let
        // `Declares::Member` take one; this is read.
        a_struct_member_takes_no_nullability_specifier: ["--emit", "ast"],
        // A hatch is a function definition, and a declaration of a tag has
        // no body. Mutation: drop the attribute check on that path; the hatch
        // is read onto `struct S`.
        a_hatch_on_a_declaration_of_a_tag_is_refused: ["--emit", "ast"],
        // Three hundred `->` in a row are one thing not checked yet, and are
        // reported once, at the first. A report each, with a span growing by
        // one access each, was output and time quadratic in the chain.
        // Mutation: report a member access whose base is one; three hundred
        // reports.
        a_chain_of_member_accesses_is_refused_once: ["--emit", "ast"],
        // C17 6.7.2.3 p1: `V` given its content twice in one scope is
        // `SC0312`, and `W` given it again in a block is a new tag, which is
        // not. The struct gate refuses every one of them as well. Mutation:
        // look a definition up in every visible scope; `W` is reported.
        // Mutation: drop the report; `V`'s goes.
        a_tag_defined_twice_in_one_scope_is_a_redefinition: ["--emit", "ast"],
        // C17 6.7.2.1 p3 and 6.7 p3: a member that is the struct being
        // defined, `void`, or a function is `SC0314`, two members of one name
        // are `SC0312`, and a flexible array member after a named one is
        // neither. The struct gate refuses each struct as well, until #419.
        // Mutation: mark a definition complete when it is bound; `s` goes
        // silent. Mutation: swap the redeclaration's labels; the labels
        // trade places.
        a_struct_member_a_struct_cannot_have_is_reported: ["--emit", "ast"],
        // C17 6.2.1 p4: a parameter list nested in a type is a prototype
        // scope of its own, so a name declared twice there is `SC0312` and an
        // undeclared length is `SC0301`, each with its caret. Mutation:
        // declare no nested parameter; the redefinition goes silent.
        // Mutation: skip the nested lengths; `m` goes silent.
        a_nested_parameter_list_is_held_to_its_own_scope: ["--emit", "ast"],
        // C17 6.9 p5: the second body of `f` is `SC0312` from the frontend,
        // and the prototypes before and after are not, so the IR is never
        // built. Mutation: drop the call to `define` in `Resolver::item`;
        // nothing reports it here and the lowering's `fill_function` panics.
        // Mutation: swap the two labels' spans; the labels trade places and
        // the header points at the first.
        a_function_defined_twice_is_a_redefinition: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C17 6.7 p3: `x` declared three times in one block is `SC0312`
        // twice, each pointing at the declaration before it, and the `x` in
        // the inner block hides it rather than repeating it, so is not.
        // Mutation: swap the two labels' spans; the labels trade places and
        // the header points at the first. Mutation: drop the report in
        // `Resolver::declare`; nothing is said here and the lowering builds.
        // Mutation: keep the first binding of a spelling in `spelled` rather
        // than the latest; the second report points at line 2.
        a_name_declared_twice_in_one_block_is_a_redefinition: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C17 6.7 p4: `f` declared at file scope with a type that does not
        // agree with its earlier prototype is `SC0313`, both types spelled,
        // and the declaration with no prototype between them is not, being
        // compatible with both. Mutation: swap the two labels; the types
        // trade places. Mutation: compare with the latest declaration; the
        // third is compatible with it and nothing is said.
        declarations_of_one_name_with_conflicting_types: ["--emit", "ast"],
        // Valid C that is not read yet, each told so with the paragraph that
        // makes it valid: an anonymous member (C17 6.7.2.1 p13) and a
        // bit-field, named or not (p9). Mutation: send a member with no
        // declarator to the `int;` check again; the anonymous member is
        // `SC0206`, a claim that valid C is invalid. Mutation: drop either
        // `:` check; that bit-field is `expected` something instead.
        a_struct_with_an_anonymous_member_is_not_read_yet: ["--emit", "ast"],
        a_bit_field_is_not_read_yet: ["--emit", "ast"],
        a_bit_field_with_no_name_is_not_read_yet: ["--emit", "ast"],
        // A member list has nowhere to put a tag (6.7.2.1 p2), so `struct T;`
        // in one declares nothing, which `clang -pedantic-errors` refuses
        // too. Mutation: call it an anonymous member; this moves.
        a_member_that_declares_only_a_tag_declares_nothing: ["--emit", "ast"],
        // A base with a type is not a struct, since a name holding one has
        // none, so `.` on an `int` and `->` on an `int *` break 6.5.2.3 p1
        // and are type errors rather than structs not checked yet. Mutation:
        // refuse every member access as `SC0304`; both move.
        a_member_access_on_what_is_not_a_struct_is_a_type_error: ["--emit", "ast"],
        // C17 6.7 p2: a declaration has to declare something, and specifiers
        // followed by their `;` declare nothing, at file scope, in a block and
        // as a `for`'s first clause alike, since all three are read by
        // `Parser::declared`. One case each, because only the first syntax
        // error in an input is reported. Mutation: drop the `;` check in
        // `declared`; each is `SC0201` "expected a name" again. Mutation: ask
        // it only at file scope; the block and `for` cases move. Mutation:
        // report it without `report_built`; the `;` left unread is refused as
        // well, a second report about one mistake, or in a block is read as
        // a statement of its own, and every case moves.
        a_declaration_that_declares_nothing_at_file_scope: ["--emit", "ast"],
        a_declaration_that_declares_nothing_in_a_block: ["--emit", "ast"],
        a_for_that_declares_nothing: ["--emit", "ast"],
        // `int *;` has a declarator, with no name, which is a syntax error and
        // not 6.7 p2's; `clang -pedantic-errors` calls it "expected identifier
        // or '('". Mutation: report `SC0206` for any declarator with no name;
        // this moves.
        a_declarator_with_no_name_is_still_a_syntax_error: ["--emit", "ast"],
        // C17 6.8.5 p1's second form: a `for` whose first clause is a
        // declaration, of one declarator and of two, with the name read in
        // the condition, the step and the body. Mutation: read a declaration
        // start as an expression clause again; both functions are `SC0201`.
        // Mutation: have `Parser::init_declarator_list` return after one
        // declarator, which a block shares; `two` is refused at the `,`. Mutation: close the scope before the body; `i` is undeclared
        // there.
        a_for_may_begin_with_a_declaration: ["--emit", "ast"],
        // 6.8.5 p5 scopes a `for`'s declaration to the loop, so `i` is
        // undeclared after it. Mutation: declare it in the block around the
        // `for`, by neither pushing nor popping a scope for it, or by pushing
        // it only after the declaration is resolved; the `SC0301` goes.
        // Removing only the push is not that: the pop then closes the block's
        // scope, and other cases fail instead.
        a_name_a_for_declares_is_out_of_scope_after_it: ["--emit", "ast"],
        // A `for`'s declaration is the statement a block would hold, so every
        // constraint on one reaches it: 6.7.6.2 p1 (`SC0309`), 6.7.9 p3
        // (`SC0310`) and p11 (`SC0302`); and 6.8.5 p3, the `for`'s own, refuses
        // a function under `SC0311`, `g` and `h` and `k`, the last declared
        // second. Mutation: stop asking a `for`'s declarators whether they
        // declare a function; the `SC0311`s go. Mutation: ask only the first
        // declarator; `k`'s goes. `for (struct S; 0;)` declares a tag, which
        // is not an object either, and is `SC0311` as well as the struct
        // gate's. Mutation: ask only a declaration's declarators; its
        // `SC0311` goes. Mutation: stop receiving a `for` declaration's
        // initializers; the `SC0302` goes.
        a_for_declaration_is_held_to_a_blocks_constraints: ["--emit", "ast"],
        // A `for` reads a declaration where a block would, and that includes
        // one an `__attribute__` begins, which is then refused as a block's is.
        // Mutation: drop the attribute half of the test in `for_statement`;
        // this becomes `SC0201` at the attribute.
        a_for_declaration_refuses_an_attribute_as_a_block_does: ["--emit", "ast"],
        // A constant expression has the value C gives it (C17 6.6 p6): `-1`
        // and `1 - 1` are lengths 6.7.6.2 p1 refuses, and `1 - 1` and `-0`
        // are null pointer constants (6.3.2.3 p3) wherever a pointer meets
        // one. `ok` is the control. Mutation: give only a literal a value;
        // the three declarations build and every line of `f` is refused.
        a_constant_expression_is_a_constant: ["--emit", "ast"],
        // At file scope an array's length is a constant with a value, C17
        // 6.7.6.2 p2, whether it is an object's or a function's return type's:
        // one that overflows or divides by zero has no defined value (6.6 p4),
        // and so has one built on it by a binary operator, each of the four
        // unary ones or a `?:` that evaluates it; one that names a variable is
        // not a constant, through a pointer as well, and so is one that mixes
        // the two. A division by zero that is never evaluated, under `||` or
        // in the arm a `?:` does not take, leaves a value (6.6 p3), and `v1`
        // and `v2` are controls with `ok`, a block's `w[k]` and a prototype's
        // `x[m]`, which are variable-length arrays C allows. `q1`'s undeclared
        // name is reported once, as `SC0301`, and not again as a length. The
        // carets of the `u` lines leave out their `(`, which is the span
        // `parser.rs::primary` gives a parenthesised operand, which is not
        // this check's. Mutation: stop
        // asking a file-scope length without a value; every `SC0309` goes.
        // Mutation: ask it at block scope or of a parameter, ask an operand C
        // does not evaluate for a value, or ask an untyped length; a control
        // reports. Mutation: leave a function definition's type unasked; the
        // `fr` report goes. Mutation: word every one alike, stop marking an
        // expression built on an undefined one through any of the three kinds
        // of operator or any unary one, or mark a mix of the two; a wording
        // moves.
        an_array_length_at_file_scope_has_a_value: ["--emit", "ast"],
        a_declaration_is_not_a_body: ["--emit", "ast"],
        a_failed_parse_reports_no_names: ["--emit", "ast"],
        a_file_scope_declaration_carries_its_initializer: ["--emit", "ast"],
        a_later_declarator_is_not_in_scope_in_an_earlier_initializer: ["--emit", "ast"],
        a_lexical_error_leaves_no_ir: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parse_error_leaves_no_ir: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_directive_stops_the_input_it_is_in: ["--emit", "ast"],
        a_tree_deeper_than_the_indent_shows: ["--emit", "ast"],
        abstract_function_parameter: ["--emit", "ast"],
        abstract_function_type_parameter: ["--emit", "ast"],
        add: ["--emit", "tokens"],
        an_array_length_stops_at_a_comma: ["--emit", "ast"],
        // A length is spelled as the tokens between its brackets, so one that
        // starts or ends with a parenthesised operand keeps both parentheses,
        // and whitespace inside the brackets is not part of it. Mutation:
        // spell the length expression's span again; `h` keeps a `)` with no
        // `(`, `f` a `(` with no `)`, and `g` loses both. Mutation: take the span from the `[` to the
        // `]`; every spelling doubles its brackets. Mutation: end the span at
        // the length expression's end; `g` and `f`, whose lengths end with a
        // parenthesis, lose it.
        an_array_length_is_spelled_as_written: ["--emit", "ast"],
        an_initializer_can_name_what_it_initializes: ["--emit", "ast"],
        an_initializer_stops_at_the_comma: ["--emit", "ast"],
        array_declaration: ["--emit", "ast"],
        array_length_is_not_evaluated: ["--emit", "ast"],
        assigning_the_wrong_type: ["--emit", "ast"],
        // C17 6.7.9 p11 gives an initializer the constraints of simple
        // assignment, so the spelling a declaration uses is the same rule.
        //
        // Mutation: have `types.rs::Checker::receivers_in` insert nothing for a
        // `Stmt::Declaration`. The `SC0302` goes from both of these and both
        // fail.
        initializing_with_the_wrong_type: ["--emit", "ast"],
        // C17 6.8.6.4 p1, both halves, the `void` expression included, which
        // `clang` accepts without `-pedantic-errors`. Mutation: accept a
        // `return` without a value in `Checker::receivers_in`; the first
        // builds. Mutation: accept one with an expression where the function
        // returns `void`; the second and third build. And `return;` in a
        // `void` function stays legal. Mutation: report a `return` without a
        // value whatever the function returns; the fourth is refused.
        a_return_without_a_value_in_a_function_returning_int: ["--emit", "ast"],
        a_return_with_an_expression_in_a_function_returning_void: ["--emit", "ast"],
        a_return_of_a_void_expression_in_a_function_returning_void: ["--emit", "ast"],
        a_return_without_a_value_in_a_function_returning_void_builds: ["--emit", "ast"],
        // One with an expression is reported beside it, in the order the
        // expressions are, after the type errors on lines above it. Mutation:
        // report it while the receivers are collected; it comes first.
        a_return_with_an_expression_in_void_is_reported_in_source_order: ["--emit", "ast"],
        // A `void` value where an assignment needs one, C17 6.5.16.1 p1 as
        // 6.8.6.4 p3 and 6.7.9 p11 apply it. Mutation: have
        // `Checker::assignable` answer `None` for a `void` source again; all
        // three build. The second holds each target that arm names, `int`,
        // `char` and a pointer: answering `None` for any one of them drops
        // that line's report.
        returning_a_void_value_from_a_function_returning_int_is_a_type_error: ["--emit", "ast"],
        initializing_with_a_void_value_is_a_type_error: ["--emit", "ast"],
        assigning_a_void_value_is_a_type_error: ["--emit", "ast"],
        // An argument is checked against its parameter, C17 6.5.2.2 p2, as
        // an initializer is. Mutation: skip the per-argument check in
        // `Checker::call`; the first three build. A null pointer constant
        // still passes for a pointer. Mutation: have
        // `Checker::check_argument` pass `false` for it; the fourth is
        // refused.
        passing_a_pointer_to_an_int_parameter_is_a_type_error: ["--emit", "ast"],
        passing_a_void_value_as_an_argument_is_a_type_error: ["--emit", "ast"],
        passing_an_int_to_a_pointer_parameter_is_a_type_error: ["--emit", "ast"],
        passing_a_null_pointer_constant_to_a_pointer_parameter_builds: ["--emit", "ast"],
        // Every argument, not only the first. Mutation: check only the first
        // pair in `Checker::call`; this builds.
        a_mismatch_in_a_later_argument_is_a_type_error: ["--emit", "ast"],
        // A parameter with no name is pointed at by its declaration.
        // Mutation: point at the argument instead of `parameter.span` in
        // `Checker::call`; the secondary label moves.
        passing_an_int_to_an_unnamed_pointer_parameter_is_a_type_error: ["--emit", "ast"],
        // A callee of pointer-to-function type is held to its function's
        // prototype, C17 6.5.2.2 p1, and its call has the function's return
        // type. Mutation: stop seeing through the pointer in
        // `Checker::call`; the first four build.
        too_many_arguments_through_a_function_pointer_is_a_type_error: ["--emit", "ast"],
        passing_a_pointer_through_a_function_pointer_is_a_type_error: ["--emit", "ast"],
        passing_an_int_through_an_address_of_a_function_is_a_type_error: ["--emit", "ast"],
        initializing_a_pointer_with_a_call_through_a_function_pointer_is_a_type_error: ["--emit", "ast"],
        // `*g` is typed as `g`, which the mutation above does not reach,
        // because `*g` is a function rather than a pointer to one. Mutation:
        // answer `None` for `*` of a function in `Checker::unary`; this
        // builds.
        passing_an_int_through_a_dereferenced_function_is_a_type_error: ["--emit", "ast"],
        // `*fp` is the function `fp` points at, through `*`'s pointer arm.
        // Mutation: answer `None` there for a pointer to a function; this
        // builds.
        too_many_arguments_through_a_dereferenced_function_pointer_is_a_type_error: ["--emit", "ast"],
        // Seeing through the pointer does not make `()` a prototype, C17
        // 6.7.6.3 p14. Mutation: check a pointer to a function declared
        // `()` as if it took no parameters; this is refused.
        a_call_through_a_pointer_to_a_function_without_a_prototype_builds: ["--emit", "ast"],
        // The null pointer constant still passes on the new path. It shares
        // its mutation with `passing_a_null_pointer_constant_to_a_pointer_parameter_builds`:
        // have `Checker::check_argument` pass `false` for it; this is refused.
        passing_a_null_pointer_constant_through_a_function_pointer_builds: ["--emit", "ast"],
        // The operand of `*` shall have pointer type, C17 6.5.3.2 p2, and
        // breaking it is the program's fault rather than this compiler's
        // gap. Mutation: answer `None` without reporting for a non-pointer
        // operand in `Checker::unary`; this builds.
        indirection_through_an_int_is_a_type_error: ["--emit", "ast"],
        // An array is the one non-pointer operand of `*` that is valid C,
        // through a decay this compiler does not model. Mutation: report it
        // with the others; this is refused.
        indirection_through_an_array_is_not_reported_as_one: ["--emit", "ast"],
        // What is called shall be a pointer to a function, C17 6.5.2.2 p1.
        // Mutation: answer `None` without reporting for any other callee in
        // `Checker::call`; this builds.
        calling_something_that_is_not_a_function_is_a_type_error: ["--emit", "ast"],
        // Nothing after names and types runs on a tree they refused, so each
        // of these is its type checker's report and nothing else, on a run
        // asked to go past it. Mutation: drop the gate after names and types
        // in `driver.rs::analysed`; the first gains the backend's `SC0801`
        // and the second the lowering's `SC0304`, which calls the program a
        // gap in this compiler.
        a_void_function_returning_a_value_stops_before_the_backend: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        a_constraint_the_type_checker_reported_stops_before_the_lowering: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Unary `+` and `-` take an arithmetic operand, `~` an integer one
        // and `!` a scalar one, C17 6.5.3.3 p1. Mutation: answer `int` for
        // any typed operand in `Checker::unary` again; this builds.
        a_unary_operator_on_a_pointer_is_a_type_error: ["--emit", "ast"],
        // Every operator against `void`, an array and a function, which
        // only `!` may take. Mutation: let any one operator take any one of
        // them; the report on that line goes.
        a_unary_operator_on_void_or_a_function_is_a_type_error: ["--emit", "ast"],
        // Every operator against `int` and `char`, and `!` against a
        // pointer, an array and a function. Mutation: refuse any one of
        // them; this is refused.
        a_unary_operator_on_an_operand_c_allows_builds: ["--emit", "ast"],
        // A refused unary operator has no type, so the `*` around it is not
        // reported again. Mutation: answer `int` after the refusal, for any
        // one operator; a second report appears at that `*`.
        indirection_through_a_refused_unary_operator_is_reported_once: ["--emit", "ast"],
        // A refused binary operation has no type, so the assignment around
        // it is not reported again. Mutation: answer `int` after the refusal
        // in `Checker::binary`; this gains an `SC0302`.
        a_refused_binary_operation_is_reported_once: ["--emit", "ast"],
        // A condition is a scalar, C17 6.8.4.1 p1, 6.8.5 p2 and 6.5.15 p2.
        // Mutation: have `check_received` accept any statement's condition;
        // `a_void_condition_is_a_type_error` builds. Mutation: skip the check
        // in `type_of`'s `Conditional` arm;
        // `a_void_condition_of_a_conditional_is_a_type_error` builds.
        // Mutation: let a refused conditional keep its arms' type; that case
        // gains an `SC0302` on the pointer it initializes. Mutation: refuse
        // any scalar type at any of the four places;
        // `a_scalar_condition_builds` is refused. Mutation: unwrap the
        // conditional's condition type rather than passing over an untyped
        // one; `an_untyped_condition_of_a_conditional_is_reported_once`
        // panics.
        a_void_condition_is_a_type_error: ["--emit", "ast"],
        a_void_condition_of_a_conditional_is_a_type_error: ["--emit", "ast"],
        // C17 6.5.2.1 p1 wants an integer beside the pointer, and a `void`
        // operand is refused whichever side it is on, with the caret on the
        // `void` one. Mutation: answer `None` for a pairing without a report;
        // this builds, and a run past `--emit ast` panics in the lowering.
        // Mutation: label the base wherever neither operand is a pointer; the
        // caret of `1[g()]` moves to the `1`.
        a_void_subscript_operand_is_a_type_error: ["--emit", "ast"],
        // And every other pairing that is not one pointer and one integer:
        // an integer base, two pointers, no pointer at all. Each was untyped
        // with nothing said, and reached the lowering to be called a gap in
        // this compiler. Mutation: answer `None` for these pairings without
        // a report; the three `SC0306`s go.
        a_subscript_without_one_pointer_and_one_integer_is_a_type_error: ["--emit", "ast"],
        a_scalar_condition_builds: ["--emit", "ast"],
        an_untyped_condition_of_a_conditional_is_reported_once: ["--emit", "ast"],
        // A conditional's arms are a pair C17 6.5.15 p3 allows, and have the
        // type p5 and p6 give them. Mutation: answer the first arm's type for
        // any pair in `Checker::conditional`;
        // `a_conditional_whose_arms_c_does_not_pair_is_a_type_error` builds.
        // Mutation: refuse any one pair the table allows;
        // `a_conditional_whose_arms_c_pairs_builds` is refused. Mutation:
        // answer no type for a pointer beside a null pointer constant;
        // `a_conditional_has_the_type_c_gives_it` loses its `SC0403` to the
        // lowering's `SC0304`.
        a_conditional_whose_arms_c_does_not_pair_is_a_type_error: ["--emit", "ast"],
        a_conditional_whose_arms_c_pairs_builds: ["--emit", "ast"],
        a_conditional_has_the_type_c_gives_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `p - q` is valid C and untyped here, with nothing reported, so the
        // `*` beside it is still `int` and the initializer is still checked.
        // Mutation: answer no type beside an untyped operand; the `SC0302`
        // goes, and a full build calls the program a gap in this compiler.
        a_type_error_beside_a_difference_of_pointers_is_still_reported: ["--emit", "ast"],
        // C17 6.5.16.2's two constraints, which are not the rule for a plain
        // `=`: `p += 1` is allowed and holds
        // `a_compound_assignment_on_a_pointer_ computes_into_a_pointer` silent.
        // The first and the last put the primary caret on the value, the middle
        // two on the place, because that is the operand the rule refuses.
        //
        // Mutation: have `types.rs::Checker::type_of` stop calling
        // `compound_assignment`. All four go silent and exit 0. Mutation: put
        // the primary label on the value always. The middle two move their
        // caret. Mutation: put it on the place always. The outer two move
        // theirs.
        adding_a_pointer_into_an_integer: ["--emit", "ast"],
        multiplying_a_pointer_in_place: ["--emit", "ast"],
        shifting_a_pointer_in_place: ["--emit", "ast"],
        subtracting_a_pointer_from_a_pointer_in_place: ["--emit", "ast"],
        // C17 6.5.5 to 6.5.14, one clause per operator: the same `SC0306`, for
        // a binary operator. Every row but `p == 1 - 1` is an error under
        // `clang --target=x86_64-unknown-linux-gnu -std=c17 -pedantic-errors`,
        // and that one is a null pointer constant compared, which C allows and
        // this accepts since constant expressions are evaluated (#384). The
        // control holds every pairing the clauses allow, so a clause answered
        // too strictly fails as surely as one answered too loosely; the unit
        // test `a_binary_operator_answers_for_every_operator_and_operand` holds
        // the same rows one at a time.
        //
        // Mutation: have `types.rs::Checker::binary` stop calling
        // `binary_operable`. The first four go silent and exit 0. Mutation:
        // have the `==` arm refuse a null pointer constant, or the relational
        // arm two pointers. The control reports.
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
        // Mutation: have `unsteppable` answer `None` for `void`. Lines 3 and 4
        // go silent. For a function, line 6 does, and for an array of unknown
        // length, lines 7 and 8. Mutation: drop the note from either report.
        // Its lines lose it. Mutation: have `unsteppable` refuse `int`. The
        // control reports.
        arithmetic_on_a_pointer_to_something_that_is_not_a_complete_object: ["--emit", "ast"],
        // The same rule in its other two spellings: C17 6.5.2.4 p2 and 6.5.3.1
        // p2 define an increment as `+= 1`, and 6.5.2.1 p1 gives a subscript
        // the same constraint. Lines 4 to 12 are refused, and `clang
        // -pedantic-errors` refuses the same nine. `1[v]` and `1[g]` are there
        // because the pointer can be either operand of `[]`, and `g[1]` and
        // `1[g]` because a function is refused only as the pointer 6.3.2.1 p4
        // makes of it. Lines 13 to 17 are the control, in the same run so that
        // their silence is asserted beside reports.
        //
        // Mutation: have `increment` stop asking `unsteppable`. Lines 4 to 7 go
        // silent. Mutation: have `subscript` stop asking it. Lines 8 to 12 do.
        // Mutation: have `subscript` ask only when the base is the pointer.
        // Lines 10 and 12 go silent. Mutation: drop `decayed` from the base in
        // `subscript`. Line 11 goes silent, and from the index, line 12.
        // Mutation: swap the two increments' clauses. Lines 4 to 7 change note.
        // Mutation: have `unsteppable` refuse `int`. The control reports.
        a_step_by_increment_or_subscript_on_a_pointer_to_something_that_is_not_a_complete_object: ["--emit", "ast"],
        // Why the rule matters past the message, as for the initializer below:
        // `i = p * 1` writes a multiplication over an `int *` into a local
        // declared `int`, which is the shape `docs/c-family.md`'s fourth
        // requirement forbids. `--emit safety-ir` so that a run with the
        // check mutated away reaches the lowering.
        //
        // Mutation: have `binary` stop calling `binary_operable`. The `.stderr`
        // goes empty and the `.exit` goes to 0.
        an_allocation_multiplied_into_an_integer_is_a_type_error: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Why the rule matters past the message. ADR-0030 has the memory check
        // drop the operand of an addition that is declared `int`, so an
        // allocation reaching `i` through this initializer is one that check is
        // handed and cannot see. `--emit safety-ir` so that a run with the
        // check mutated away reaches it. Under the mutation above the
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
        several_items_and_statements: ["--emit", "ast"],
        returning_the_wrong_type: ["--emit", "ast"],
        subscript: ["--emit", "ast"],
        too_few_arguments: ["--emit", "ast"],
        too_many_arguments: ["--emit", "ast"],
        // A name is typed by its standing declaration, the prototype, rather
        // than the `int g();` the use resolves to, so a pointer for its `int`
        // parameter and a call with none are both refused, and the label is
        // on the prototype. A call written before the prototype is held to
        // it too. Mutation: type a name by the binding it resolves to in
        // `types.rs`; all three are silent.
        a_call_is_checked_against_the_prototype_not_a_later_declaration_without_one: ["--emit", "ast"],
        a_call_written_before_the_prototype_is_checked_against_it: ["--emit", "ast"],
        // An object of pointer-to-function type is typed by the declaration
        // in scope, as C types it: `fp` holds `two`, and `fp(1, 2)` is a
        // defined call whatever `fp` is declared as later. Mutation: type
        // every name by its standing declaration in `types.rs`; this is
        // refused as too many arguments.
        a_pointer_to_a_function_is_called_as_it_is_declared_where_it_is_called: ["--emit", "ast"],
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

    "codegen" => {
        // The `--emit llvm-ir` cases, kept together because what each is for is
        // only visible beside the others. `every_operator` is the one that
        // stops the operator table being a table nothing checks: without it,
        // spelling `BitAnd` as `or`, `Mul` as `add`, `Le` as `lt`, `Neg` as
        // `add` and `BitNot` as `xor 0` all passed the whole suite, because
        // nothing checked the table against anything but itself.
        // `conversions_and_a_constant_condition` is the same for C17 6.3.1.3
        // and 6.5.2.2 p7: `c = 300` and `narrow(300)` both answer 44, and a
        // constant that ignored its destination's type passed everything before
        // it. `mix` is there because every other call in the suite has one
        // parameter or two of one type, so pairing each argument with the wrong
        // parameter passed everything too.
        //
        // The two `extended` cases are one program on two machines, because
        // what differs is the machine: `x86_64-unknown-linux-gnu` asks for
        // `signext` and `armv7-unknown-linux-gnueabihf` for `zeroext`, and the
        // `int` beside the `char` is what says the rule reads a width rather
        // than a type. Every other `--emit llvm-ir` case is on a target that
        // asks for nothing, so these two are the only place in the tree an
        // attribute appears at all.
        //
        // Every one of these is also in `llvm.rs`, which hands it to `clang`.
        // The text and whether the text is LLVM are two claims.
        a_narrow_unsigned_value_is_extended_without_a_sign: ["--emit", "llvm-ir", "--target", "armv7-unknown-linux-gnueabihf"],
        a_narrow_value_is_extended_where_the_target_asks: ["--emit", "llvm-ir", "--target", "x86_64-unknown-linux-gnu"],
        an_ir_shape_the_backend_cannot_write: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        llvm_ir_follows_the_target: ["--emit", "llvm-ir", "--target", "aarch64-unknown-linux-gnu"],
        llvm_ir_of_conversions_and_a_constant_condition: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        llvm_ir_of_every_operator: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        // A block ending in a call that does not return ends in `unreachable`,
        // and `tests/llvm.rs` hands it to `clang`. Mutation: have `emit.rs`'s
        // `call` write nothing for no continuation; this fails, and so does
        // `tests/llvm.rs`, since the block is left with no terminator.
        llvm_ir_of_a_call_that_does_not_return: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        llvm_ir_of_pointers_branches_and_a_loop: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        the_mvp_becomes_llvm_ir: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    }

    "lowering" => {
        // The shape of the Safety IR a program lowers to, where no check and no
        // frontend report is what the case is about.
        an_initializer_becomes_a_store: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        edges_of_a_branch_and_a_loop: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        every_shape_the_artifact_spells: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `i` and `j`, declared by the `for`, begin their storage once before
        // the first turn and end it once, in reverse, in the block the loop is
        // left to, and `t`, declared in the body, begins and ends it each turn
        // (ADR-0012). `g`'s loop has no condition and so no edge out: `x`'s
        // storage ends at the `return`, and again in the block after the
        // loop, which is built and which nothing reaches, so no check walks
        // it.
        // Mutation: do not end the `for`'s scope's storage; `i`'s
        // `StorageDead` goes.
        a_for_declaration_lives_across_the_whole_loop: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `f`'s `for` cannot be lowered, since a variable length array has no
        // IR yet, and its scope is closed all the same, so `g`, lowered by the
        // same `Lowering`, opens with only its body's scope and `x` gets no
        // storage marker. The array is chosen for what the lowering cannot do
        // and stops testing this the day it can. Mutation: close the `for`'s
        // scope only when the loop was lowered; `g` gains a `StorageLive`.
        a_for_that_cannot_be_lowered_leaves_no_scope_open: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `void` value is nothing: a call to a `void` function has no
        // `Destination`, a `void` `?:` has no answer and its arms write
        // none, a comma discards no left operand that is `void`, and neither
        // does a `for`'s first or third clause. No
        // `void` local appears but the return place of a declared `void`
        // function. Mutation: make the temporary for a `void` call again, or
        // give a `void` `?:` its answer back; this moves. Mutation: pop a
        // comma's left operand whether or not it is `void`, or lower a
        // `for`'s first or third clause with `value`; this panics on the
        // empty stack.
        a_call_that_returns_nothing_writes_nowhere: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C17 6.5.2.1 p2 makes `E1[E2]` mean `*((E1)+(E2))`, so `1[p]` is
        // `p[1]`, and both lower to `p + 1` and a `Deref`. Both are doubted
        // alike, because the nullability check answers `Unknown` for the
        // result of any arithmetic, `_Nonnull` operand or not. Mutation: read the
        // subscript's type off the base only; `1[p]` is `SC0304`. Mutation:
        // take the base as the pointer in the lowering; the constant `1` is
        // refused as pointing at nothing.
        a_subscript_whose_pointer_is_the_index_is_built_as_the_other_way_round: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same two operators, at the one type whose operation does not
        // happen at `int`. C17 6.5.6 p8 makes `p + 1` a pointer, and ADR-0030
        // has the memory check read a local's declared type to tell the pointer
        // operand of an addition from the integer beside it and drop the
        // integer, so a temporary declared `int` holding an allocation is one
        // that check can be handed and not see. `--emit safety-ir` because the
        // declaration being pinned is the IR's: `compound_assignment` and
        // `increment` pin the tree these are read from and stop there, and the
        // tree is where this is right.
        //
        // Mutation: have `lowering.rs::promoted` answer `Ty::Int` for a pointer
        // place again. These two fail and nothing else in the suite does, which
        // is why they are here: the fix is invisible to every other case.
        a_compound_assignment_on_a_pointer_computes_into_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_increment_of_a_pointer_computes_into_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the same through a projection, because `promoted` is handed the
        // whole place rather than its base local: `*pp` is an `int *` where
        // `pp` is an `int **`, and the two cases above cannot tell the
        // difference because their places have no projection at all.
        //
        // Mutation: have `promoted` ask about `Place::local(place.local)`
        // instead of `place`. Only this case fails.
        a_compound_assignment_through_a_dereferenced_pointer_computes_into_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_scope_that_opens_and_closes: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `wasm32` on purpose, and not the triple every other IR case names: no
        // CI runner and no developer machine hosts it, so this is the case that
        // fails wherever `--target` stops being honoured. On a machine that
        // hosts the triple the others name, they cannot tell the two apart.
        a_target_the_host_is_not: ["--emit", "safety-ir", "--target", "wasm32-unknown-unknown"],
        places_a_pointer_reaches: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        the_mvp_lowers_to_blocks_and_edges: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        several_declarators_each_become_a_function: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        several_declarators_each_become_a_local: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    }
}

/// Everything in `cases/` belongs to a case the table names, in the group the
/// table names it under.
///
/// The table is the definition and the directory follows it, which is the same
/// rule read from the other end. These are the ways to be in there and be
/// dead, and one guard answers for all of them rather than for the first:
///
/// * a `.c` nobody listed is never run;
/// * a `.stdout` left behind by a renamed case is compared against nothing, and
///   then waits for the next case to reuse the name, which starts life failing
///   against content from a case it never heard of;
/// * a case's files in a group other than its own are never read, since the
///   case runs from its own group's directory;
/// * a directory no group names, or one inside a group, hides any of those
///   from a walk that only looks where the table says to;
/// * a file directly in `cases/` belongs to no group.
///
/// Dead weight wearing the appearance of coverage is the failure this corpus
/// exists to avoid, so the directory answers for every entry it has, one level
/// down and no further. See ADR-0007.
///
/// Mutation: put anything in `cases/` that the table does not name: a file at
/// the top level, a directory no group names, a directory inside a group, or a
/// case's files in another group. This test fails each time. What `strays`
/// itself refuses is held by the test after it, since on a corpus with nothing
/// stray in it a guard that refused nothing would pass here too.
#[test]
fn every_file_in_the_corpus_belongs_to_a_case_in_the_table() {
    let strays = strays(&cases_dir(), CASES);

    assert!(
        strays.is_empty(),
        "nothing in the table names these, so nothing runs them and nothing \
         says so: {strays:?}"
    );
}

/// Each way to be stray, built in a directory of its own and named by `strays`,
/// beside two cases that are where they belong.
///
/// Mutations, each of which the guard above survives on a correct corpus and
/// this does not: compare a file's stem without its group; skip the test that
/// a group is a directory; skip the test that what is inside one is a file;
/// report nothing found inside a group.
#[test]
fn every_way_to_be_stray_is_reported() {
    let root = std::env::temp_dir().join(format!("safec_strays_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["g", "h", "g/nested", "g/one.exit", "unknown"] {
        std::fs::create_dir_all(root.join(dir)).expect("a temporary directory can be made");
    }
    for file in [
        "g/one.c",
        "g/one.stdout",
        "h/two.c",
        "top.c",
        "k",
        "h/one.c",
        "g/one.txt",
        "g/three.c",
    ] {
        std::fs::write(root.join(file), b"").expect("a temporary file can be written");
    }

    let found = strays(&root, &[("g", "one"), ("h", "two"), ("k", "four")]);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        found,
        [
            "g/nested",   // a directory inside a group
            "g/one.exit", // a directory, though a case's file would have its name
            "g/one.txt",  // an extension no stream has
            "g/three.c",  // a case nobody listed
            "h/one.c",    // a case's file in another group
            "k",          // a file, though a group has its name
            "top.c",      // a file directly in the corpus
            "unknown",    // a directory no group names
        ]
    );
}

/// What under `root` belongs to no case of `cases` in the group it is in,
/// sorted.
fn strays(root: &Path, cases: &[(&str, &str)]) -> Vec<String> {
    const EXPECTED: [&str; 4] = ["c", "stdout", "stderr", "exit"];

    let mut strays: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root).expect("the cases directory is in the repository") {
        let group_path = entry.expect("a directory entry can be read").path();
        let group = group_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        if !group_path.is_dir() || !cases.iter().any(|(named, _)| *named == group) {
            strays.push(group);
            continue;
        }

        for entry in std::fs::read_dir(&group_path).expect("a group's directory can be read") {
            let path = entry.expect("a directory entry can be read").path();
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();

            let belongs = path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| EXPECTED.iter().any(|known| ext == *known))
                && cases.contains(&(group.as_str(), stem.as_str()));

            if !belongs {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                strays.push(format!("{group}/{name}"));
            }
        }
    }
    strays.sort();
    strays
}

/// Where the cases live.
fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases")
}

/// Run one case and compare all three of its outputs.
///
/// The compiler is run **from** the case's group directory and handed a bare
/// file name, because it echoes back the path it was given: `safec --emit
/// tokens crates/safec/tests/cases/frontend/add.c` prints
/// `crates/safec/tests/cases/frontend/add.c:3:1 keyword "int"`, while the same
/// run from inside the directory prints `add.c:3:1 keyword "int"`. That is
/// what makes an expected file mean the same thing on every machine, and it is
/// why passing a path here would silently break every case that reports a
/// position.
///
/// `--color never` is passed rather than relied on. `ColorMode::Auto` resolves
/// against whether the stream is a terminal, and a test whose meaning depends
/// on not being one changes meaning when somebody runs it differently.
fn run_case(group: &str, name: &str, args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .current_dir(group_dir(group))
        .args(["--color", "never"])
        .args(args)
        .arg(format!("{name}.c"))
        .output()
        .expect("the compiler binary was built for this test");

    let code = output
        .status
        .code()
        .unwrap_or_else(|| panic!("case `{name}`: the compiler was killed by a signal"));

    check(group, name, "stdout", &output.stdout);
    check(group, name, "stderr", &output.stderr);
    check(group, name, "exit", &expected_exit(code));
}

/// Where one group's cases live, and the directory the compiler is run from
/// for them.
fn group_dir(group: &str) -> PathBuf {
    cases_dir().join(group)
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
/// when this was written and is many times that now.
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
fn check(group: &str, name: &str, ext: &str, actual: &[u8]) {
    let path = group_dir(group).join(format!("{name}.{ext}"));

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
