//! C programs, and the output the compiler is expected to produce for them.
//!
//! A case is a `.c` file in a group's directory under `cases/` and up to three
//! expected-output files beside it, and a `.h` of the same name where the `.c`
//! includes one. Adding one is those files and a line in the table below; no Rust
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
        // A pointer local nothing assigned, read by a null test and written
        // through on the arm it passes: a proof at each read (C17 6.3.2.1 p2),
        // since no path assigns it.
        // Mutation: treat every local as assigned where the function starts.
        // It builds.
        a_pointer_never_assigned_is_reported_where_a_null_test_reads_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Assigned on one arm only and freed after, which the other arm
        // reaches unassigned: doubted, not proved, since the edge may be one
        // no execution takes. Mutation: join with `&&`. It is proved.
        a_pointer_assigned_on_one_arm_and_freed_after_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Assigned on both arms: nothing. Mutation: have an assignment leave
        // the bit as it was. It is proved.
        a_pointer_assigned_on_every_arm_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The body's program: `c`'s false edge leads to the read on every path
        // and assigns `p` on none. Mutation: drop the pass over the edges. It
        // builds.
        a_pointer_assigned_on_one_arm_is_doubted_where_the_other_arm_leads_to_its_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The edge is the one a non-zero condition takes, and the label says so.
        // Mutation: swap the edge's words. The label moves.
        a_pointer_assigned_only_on_the_arm_a_true_test_skips_is_doubted_at_its_true_edge: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Every edge before the body either carries a path that assigned `p` or
        // leads past the body. Mutation: drop (b) or (c) in
        // `memory::assigned`. It is refused.
        a_pointer_assigned_inside_a_logical_and_is_read_in_its_body_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The loop test's exit edge carries the later turns, which assigned `p`.
        // Mutation: drop (a). It is refused.
        a_pointer_assigned_in_a_loop_that_runs_once_is_read_after_it_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read under `i > 0`, after the turn that assigned it. No one clause
        // holds it: drop (a) together with (b) or (c), or join with `&&`, and
        // it is refused.
        a_pointer_read_in_a_loop_only_after_its_first_turn_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The read is under `saved`, so `c`'s false edge does not lead to it
        // on every path. Mutation: drop (c). It is refused.
        a_pointer_assigned_beside_a_flag_is_read_under_it_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same with a parameter tested twice. Mutation: drop (c). It is
        // refused.
        a_pointer_assigned_and_read_under_one_parameter_tested_twice_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The unassigned path ends in a call that does not return. Mutation:
        // count such a call as going on. It is refused.
        a_pointer_whose_unassigned_path_ends_in_abort_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Assigned under `c` and under `!c`: a path from `c`'s false edge
        // assigns it. Mutation: drop (b). It is refused.
        a_pointer_assigned_on_both_arms_of_a_test_and_its_negation_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // **The cost**: `n < 0` cannot be false where `n >= 0` was, and C
        // defines this, but its false edge leads to the read unassigned. It is
        // refused, as clang's `-Wall` warns. It is here so the cost is seen,
        // and fails if the pass over the edges is dropped.
        a_pointer_assigned_on_both_arms_of_a_test_that_cannot_fail_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The arm that does not assign it never ends, so its edge does not
        // lead to the read. Mutation: count a loop as reaching the read. It
        // is refused.
        a_pointer_read_after_an_arm_that_never_ends_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The unassigned path ends in `exit` inside a larger expression,
        // where the lowering keeps the call's edge. Mutation: read only
        // `then` in `reaches_certainly`. It is refused.
        a_pointer_whose_unassigned_path_ends_in_exit_inside_an_expression_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // **A cost**: `c || 1` is never false, but `||` is lowered as a
        // second branch on its result, whose false edge leads to the read
        // unassigned. It is refused, which clang does not do. It is here so
        // the cost is seen, and fails if the pass over the edges is dropped.
        a_pointer_assigned_under_a_test_with_a_constant_operand_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `abort` inside a larger expression keeps its edge, and touches
        // nothing on it. Mutation: read the four names as `Callee::Opaque`.
        // `SC0407` at `use(p)`.
        a_pointer_handed_on_after_abort_inside_an_expression_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What `exit` is handed is still read, and the replay in
        // `memory::findings_within` is what reports it, not the transfer.
        // Mutation: skip the replay's `used` at a terminator calling one of
        // the four names. It is silent.
        a_freed_pointer_read_as_the_argument_of_exit_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Neither check doubts `*p`: the memory check reads `abort` as touching
        // nothing, and the nullability check ends its path and reads `p != 0`
        // through the `||`'s normalisation (#592). Mutation: read the four
        // names as `Callee::Opaque`. `SC0402`. Mutation: stop reading through
        // the normalisation in `tested_against_null`. `SC0403`.
        a_pointer_tested_before_abort_inside_an_expression_is_not_doubted_as_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A read unordered against `exit`, of memory a store in the same
        // expression exposed: it ran before the call or never ran. Mutation:
        // answer `Callee::Ends` in `used_before` as `Callee::Opaque` is.
        // `SC0402` at `a[0]`.
        a_read_unordered_against_exit_after_a_store_exposed_it_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `free(p)` with `p` null frees nothing (C17 7.22.3.3 p2), and on an
        // arm that has passed `(abort(), 0);` the nullability check still
        // answers what it knew there. Mutation: answer nothing established
        // on an ended path in `null_at_terminators`. `SC0401`.
        a_free_of_a_null_pointer_after_abort_on_its_arm_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Stored into fresh memory and read back, and reported only where it
        // was first read: the store writes the slot, as any store does, so
        // neither the read back nor the copy the lowering takes of the
        // assignment's value is a read of memory nothing wrote. Mutation: keep
        // the slot fresh for a store of an indeterminate pointer. `SC0411`
        // joins at the store and at the read back. See ADR-0057.
        a_pointer_read_out_of_fresh_memory_stored_and_read_back_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A copy of it, freed after a call may have replaced it: the doubt at
        // the free stays, since what is freed may not be what was reported.
        // Mutation: drop the `escaped` clause in `Known::holds_fresh_read`.
        // The doubt goes.
        a_pointer_read_out_of_fresh_memory_a_call_replaced_is_still_doubted_where_a_copy_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read out of what `malloc` returned before anything wrote
        // it, and written through on the arm a null test passes: a proof
        // (C17 7.22.3.4 p2 and Annex J.2), at the read. Mutation: leave a
        // `malloc` site not fresh. It builds.
        a_pointer_read_out_of_fresh_memory_is_reported_where_it_is_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Freed, which frees no allocation, and reported once, at the read,
        // rather than beside a doubt at the free. Mutation: drop the
        // suppression after `freed_after_its_read_was_reported` in the
        // replay. The doubt joins. See ADR-0057.
        a_pointer_read_out_of_fresh_memory_and_freed_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read through and then freed: reported at the read, not again at
        // either use. Mutation: have `loaded_indeterminate` report a read
        // through a local `Known::holds_fresh_read` marks as well. `SC0411`
        // joins at the read through.
        a_pointer_read_out_of_fresh_memory_read_through_and_freed_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Copied first, and the copy freed. Mutation: clear the mark on a
        // plain copy. The doubt at the free joins.
        a_copy_of_a_pointer_read_out_of_fresh_memory_freed_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read through a second pointer to the same allocation. Mutation:
        // leave a `malloc` site not fresh. It builds.
        a_pointer_read_out_of_fresh_memory_through_an_alias_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `calloc`'s space is zero bits (C17 7.22.3.2 p2), a null pointer on
        // this target, so the null test is right. Mutation: let `calloc`
        // leave its site fresh. It is reported.
        a_pointer_read_out_of_memory_calloc_made_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written by a store first. Mutation: have a store through one
        // dereference leave its sites fresh. It is reported.
        a_pointer_read_out_of_memory_a_store_wrote_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Handed to a call this check cannot read first, which may have
        // written it; the doubts are the ones that call already raises.
        // Mutation: have the exposure of what a call is handed leave its
        // sites fresh, in both of `Known::expose`'s loops; either alone is
        // held by the other. `SC0411` joins them.
        a_pointer_read_out_of_memory_a_call_was_handed_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written by `memcpy` first, which exposes what it was handed.
        // Mutation: have an exposure leave its sites fresh, in both of
        // `Known::expose`'s loops. It is reported.
        a_pointer_read_out_of_memory_memcpy_wrote_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written on one arm only, which is undefined on the other and not
        // reported: a slot is asked only for a proof. Mutation: join the mark
        // with `||`.
        // It is reported.
        a_pointer_read_out_of_memory_written_on_one_arm_is_not_asked_yet: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written through a pointer read out of memory, which this check
        // does not place. Mutation: have such a store clear no site. It is
        // reported.
        a_pointer_read_out_of_memory_a_loaded_pointer_wrote_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read through a pointer that may be `p`'s address, which holds a
        // value. Mutation: drop the address clause in `reads_fresh`. It is
        // reported.
        a_pointer_read_through_what_may_be_a_locals_address_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read out of fresh memory, and then handed by address to a call
        // that may have written another pointer into it, so the doubt at its
        // free stays. Mutation: drop the `escaped` clause in
        // `Known::holds_fresh_read`. The doubt goes.
        a_pointer_read_out_of_fresh_memory_a_call_may_have_replaced_is_still_doubted_where_it_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read through a pointer to either of two allocations, one written,
        // and used only where it is the written one. Mutation: weaken `all`
        // to `any` over the sites in `reads_fresh`. It is reported.
        a_pointer_read_through_either_of_two_allocations_one_written_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read out of fresh memory on one arm and freed only on the other,
        // where it holds what `malloc` returned, so the doubt at the free
        // stays. Mutation: join `Held::fresh_read` with `||`. The doubt goes.
        a_pointer_read_out_of_fresh_memory_on_one_arm_only_is_still_doubted_where_it_is_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Stored in a holder handed to a call this check cannot read, which
        // may write through it. Mutation: delete `written` from the loop
        // over `reachable_now` in `Known::expose`. `SC0411` joins the
        // doubts.
        a_pointer_read_out_of_memory_a_call_reached_through_a_holder_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written through two dereferences, `**pp = &q;` with `*pp == box`.
        // Mutation: drop `one_step` from the store's clause. It is reported.
        a_pointer_read_out_of_memory_written_through_two_dereferences_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read as a condition, which a terminator holds. Mutation: drop the
        // call to `loaded_indeterminate` at the terminator. It builds.
        a_pointer_read_out_of_fresh_memory_by_a_condition_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Handed to `realloc`, and reported once, at the read. Mutation: ask
        // `freed_after_its_read_was_reported` about `free` alone. The doubt
        // joins.
        a_pointer_read_out_of_fresh_memory_and_reallocated_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read out of the old block on the arm where `realloc` failed, which
        // left it live and unwritten (C17 7.22.3.5 p4). Mutation: clear
        // `fresh` for what a `realloc` is handed. It builds.
        a_pointer_read_out_of_the_old_block_after_a_failed_realloc_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read out of an allocation a free of one of two may have freed:
        // freed or never written, it is undefined on every path, and the read
        // stays a proof beside the doubt. Mutation: clear `fresh` for every
        // site a free may free. Only the doubt is left.
        a_pointer_read_out_of_fresh_memory_a_free_may_have_freed_is_still_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local reached by its address is memory too: a pointer read out of
        // one nothing wrote is reported where it is read, through a pointer
        // and by name (#640). See ADR-0057.
        //
        // Mutation: `reads_fresh` asks the sites alone; the first two go
        // silent.
        a_pointer_read_out_of_a_local_array_nothing_wrote_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_through_the_address_of_a_local_nothing_wrote_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `unwritten_read` reports nothing; this goes silent.
        a_local_whose_address_is_taken_read_by_name_before_anything_wrote_it_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `unwritten_read` is not asked at a terminator; both
        // reads, a call's argument and a branch's condition, go silent.
        a_local_whose_address_is_taken_read_by_name_in_a_call_or_a_condition_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: say only one storage where the pointer may reach both;
        // one of the two notes goes.
        a_pointer_read_out_of_an_allocation_or_a_local_nothing_wrote_cites_both: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a store through a pointer clears no local; this is
        // reported.
        a_pointer_read_out_of_a_local_array_after_it_was_written_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `Known::replaced` clears no local; this is reported.
        a_local_a_call_was_handed_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written on one arm only, `m[0]` may be read unwritten on the
        // other: an `SC0411` doubt (#668). See ADR-0057. Mutation: the
        // proofs per element joined by union; this is certain. The silent
        // half is the next case's, since this one's unwritten side is the
        // value joined into.
        a_local_array_written_on_one_arm_only_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same written on the other arm, which reaches the join as the
        // value joined in rather than the one joined into. Mutation: a
        // forward join keeps one side's `Known::maybe_unwritten_elements`;
        // this goes silent.
        a_local_array_written_on_the_other_arm_only_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Each turn's `m` is a new object (C17 6.2.4 p2), unwritten on the
        // arm that skips the write whatever the turn before wrote at its
        // end. Mutation: `StorageLive` leaves
        // `Known::maybe_unwritten_elements` alone; this goes silent.
        a_local_array_made_again_each_turn_and_written_on_one_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written on every path into the join, it is written. Mutation: a
        // forward join doubts whatever the two sides hold; this gains an
        // `SC0411` doubt.
        a_local_array_written_on_both_arms_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m[1]` was written on neither path, so it is still a certain
        // `SC0411` where `m[0]` would be a doubt. Mutation: the doubt is
        // kept for the whole array rather than per element; this is a
        // doubt.
        an_element_neither_arm_wrote_is_reported_beside_one_written_on_one_arm: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The doubt made where the arms meet crosses the loop's back edge,
        // which keeps what either side doubted. Mutation: a back edge keeps
        // only what both sides prove; this goes silent.
        a_local_array_doubted_before_a_loop_is_still_doubted_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer local whose address is taken, written on one arm, is an
        // `SC0411` doubt read through that address (#668). Mutation: a
        // forward join sets no doubt where `Known::unwritten` differs; this
        // goes silent.
        a_pointer_local_written_on_one_arm_only_is_doubted_where_read_through_its_address: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same read by name, in its own words. Mutation: as above; this
        // goes silent.
        a_pointer_local_written_on_one_arm_only_is_doubted_where_read_by_name: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a back edge does not keep `Known::partly_written`; this
        // goes silent.
        a_pointer_local_doubted_before_a_loop_is_still_doubted_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A doubt the head already holds survives the back edge even where
        // the loop writes the element, which leaves it nothing to carry
        // back. Mutation: a back edge keeps only what arrives along it and
        // what both sides prove; this goes silent.
        a_local_array_doubted_before_a_loop_that_writes_it_is_still_doubted_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same of a pointer local. Mutation: a back edge keeps only the
        // `Known::partly_written` it brings; this goes silent.
        a_pointer_local_doubted_before_a_loop_that_writes_it_is_still_doubted_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The doubt made inside the body reaches the head along its back
        // edge. Mutation: a back edge drops the `Known::partly_written` it
        // brings; this goes silent.
        a_pointer_local_written_on_one_arm_inside_a_loop_is_doubted_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same of an array too long to keep per element. Mutation: as
        // above; this goes silent.
        a_long_local_array_written_on_one_arm_inside_a_loop_is_doubted_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The loop-fill exemption for a local with no per-element record.
        // Mutation: a back edge sets a doubt where `Known::unwritten`
        // differs; this gains an `SC0411` doubt.
        a_pointer_local_assigned_by_a_loop_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same of an array too long to keep per element. Mutation: as
        // above; this gains an `SC0411` doubt.
        a_long_local_array_filled_by_a_loop_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A doubt on one side of a join and a write on the other, where
        // `Known::unwritten` differs on neither. Mutation: a forward join
        // keeps only the side joined into; this goes silent.
        a_long_local_array_written_in_part_on_one_arm_and_at_its_start_on_the_other_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same with the arms the other way round. Mutation: a forward
        // join keeps only the side joined in; this goes silent.
        a_long_local_array_written_at_its_start_on_one_arm_and_in_part_on_the_other_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A write that may be of any element of an array too long to keep
        // per element leaves it in doubt (#639). Mutation: such a write
        // sets no `Known::partly_written`; this goes silent.
        a_long_local_array_copied_into_through_a_moved_pointer_is_doubted_where_read_at_its_start: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An array too long to keep per element is answered as a pointer
        // local is. Mutation: as above; this goes silent.
        a_long_local_array_written_on_one_arm_only_is_doubted_where_read_at_its_start: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `StorageLive` leaves `unwritten` alone; this goes silent.
        a_local_array_made_again_each_turn_of_a_loop_is_reported_where_read_before_this_turn_wrote_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `Known::replaced` clears no local, which is the door a
        // copy goes through; this is reported.
        a_local_filled_by_memcpy_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `holds_a_pointer` answers `true` for every type; this is
        // reported where `v` is read by name.
        an_integer_local_whose_address_is_taken_read_before_anything_wrote_it_is_not_followed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `StorageLive` makes every local unwritten again,
        // whatever its type; this gains an `SC0411`.
        an_integer_local_made_again_in_a_loop_and_read_as_a_pointer_is_not_followed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `holds_a_pointer` answers `true` for `char`; this is
        // reported.
        a_character_local_whose_address_is_taken_read_before_anything_wrote_it_is_not_followed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `Known::replaced` clears no local, which is the door a
        // store this check cannot place goes through; this is reported.
        a_local_written_through_two_dereferences_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An array holds its elements, so a store this check cannot place
        // writes it where the type stored is its element's. Mutation:
        // `replaced_by` compares the type stored with the local's own type;
        // this is reported.
        a_local_array_written_through_a_pointer_read_out_of_memory_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `e` walks back onto `m[1]` and `m[0]` and keeps its edge to the
        // array as it moves (#639), so the store writes `m` in part and the
        // read of `m[0]` is an `SC0411` doubt, which `--allow-unknown`
        // lowers. See ADR-0057. Mutation: the edge is dropped for every
        // local again; the `SC0411` is certain. Mutation: a store marks
        // nothing `Held::stored_through`; this gains a doubt at `*e = &x`,
        // the lowering's own read of the assignment's value (#667).
        a_local_array_filled_backwards_through_a_moved_pointer_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A write this check cannot pin down writes only the locals its
        // place may land in (#645). See ADR-0057.
        //
        // Mutation: `replaced_by` passes `may_hold` as `may_write`; this
        // goes silent.
        a_pointer_read_out_of_a_local_array_a_store_through_caller_memory_did_not_write_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read out of memory before a local array's address left
        // cannot be that array, so a write through it does not write the
        // array (#648). See ADR-0057. Mutation: a pointer read out of
        // memory lands in every local code this check cannot read names at
        // the write again; this goes silent.
        a_pointer_read_out_of_memory_before_a_local_array_left_does_not_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same written by a library copy. Mutation: as above; this goes
        // silent.
        a_copy_through_a_pointer_read_out_of_memory_before_a_local_array_left_does_not_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read after the address left, it may be the array, and the write may
        // be its assignment. Its edges and what was named as it was read
        // each say so. Mutation: a pointer read out of memory lands in
        // neither; this gains `SC0411`. Either alone leaves it silent.
        a_pointer_read_out_of_memory_after_a_local_array_left_may_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read before on one path and after on the other, it may be the
        // array where the paths meet. Mutation: as above, both together;
        // this gains `SC0411`.
        a_pointer_read_out_of_memory_before_a_local_array_left_on_one_path_only_may_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read out of memory a call wrote, after the array's address left
        // through a local the call kept, it may be that address, and no edge
        // says so: a call records nothing of what it wrote. A correct
        // program. Mutation: a load records nothing in `named_at_read`, or a
        // write through it ignores what it recorded; this gains `SC0411`.
        a_pointer_read_out_of_memory_a_call_wrote_after_a_local_array_left_may_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same read after the address left on one path and before it on
        // the other: where the paths meet it may be that address.
        // Mutation: the join keeps one path's `named_at_read` rather than
        // the union; this gains `SC0411`.
        a_pointer_read_out_of_memory_a_call_wrote_after_a_local_array_left_on_one_path_only_may_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same moved as it is read, `*x + 1`, which reaches the value as
        // one operand rather than through a load. Mutation: such an operand
        // records nothing in `named_at_read`; this gains `SC0411`.
        a_pointer_moved_as_it_is_read_out_of_memory_a_call_wrote_after_a_local_array_left_may_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer a call wrote into a local, before a local array's
        // address left, cannot be that array, so a write through it does
        // not write the array (#661). See ADR-0057. Mutation: a pointer a
        // call wrote lands in what is named at the write again; this loses
        // its `SC0411`.
        a_pointer_a_call_wrote_before_a_local_array_left_does_not_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written after the address left, it may be the array. Two things
        // say so: the call is taken to write every local it may name, and
        // what it could name is recorded on the pointer. Mutation: both
        // dropped, the second by passing no local as the call's `may_write`;
        // this gains `SC0411`. Either alone leaves it silent.
        a_pointer_a_call_wrote_after_a_local_array_left_may_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A store this check cannot place, through a pointer a call may have
        // aimed at `t`, may put there the array's address it makes nameable
        // itself, so a write through `t` after it may write the array. A
        // correct program. Mutation: a store adds nothing to what such a
        // local records after it exposes what it carries; this gains
        // `SC0411`.
        a_pointer_a_call_kept_written_by_a_store_that_made_a_local_array_nameable_may_write_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `t`'s address reaches no call, so no store through a pointer read
        // from the caller can land in it, and `*t = 0` writes `x`, not the
        // array. Mutation: every escaped local `replaced` marks records
        // what is named, written or not; this loses its `SC0411`.
        a_pointer_no_call_can_reach_is_not_written_by_a_store_this_check_cannot_place: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `t` was written by a call in an earlier turn, which named that
        // turn's `m`; this turn's `m` is a new object (C17 6.2.4 p2), which
        // the write through `t` does not reach. Mutation: `StorageLive`
        // leaves the recorded sets alone; this loses its `SC0411`.
        a_pointer_a_call_wrote_in_an_earlier_turn_does_not_write_the_next_turns_local_array: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer copied in an earlier turn, whose edge names that turn's
        // `m`, is no write of this turn's `m` (C17 6.2.4 p2, #664). See
        // ADR-0057. Mutation: `StorageLive` leaves pointers' edges to the
        // local alone; this loses its `SC0411`.
        a_pointer_copied_in_an_earlier_turn_does_not_write_the_next_turns_local_array: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Copied in this turn, after `*b = m;`, it may be this turn's `m`,
        // and no `SC0411`. No mutation measured gives it one. Mutation: the
        // pointer that loses its edge is marked `lost`; this fails on its
        // doubts about `t`, which change.
        a_pointer_copied_in_this_turn_may_write_this_turns_local_array: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer stored into `m[1]`, freed and read back is used after it
        // was freed (#639): `&m[0] + 1` still points into `m` (C17 6.5.6
        // p8). Mutation: the edge is dropped for every local again; this
        // loses its `SC0402`.
        a_pointer_stored_into_a_local_arrays_second_element_and_freed_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // One record stands for the array, so what `m[1]` holds is what `m`
        // may hold, and `m[0]` read back after `m[1]`'s pointer was freed is
        // doubted. No mutation of #639 moves it, since a store at `m[1]`
        // reached the array as a whole before #639 too. Mutation: a write
        // through a known element clears none (#667); this gains `SC0411`.
        a_pointer_stored_into_one_element_and_another_freed_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A write through `&m[0] + 1` writes `m[1]` alone, so `m[0]` read
        // after it was written by nothing is a certain `SC0411` (#639,
        // #667). See ADR-0057. Mutation: a write through a known element
        // clears every element; this goes silent.
        a_local_array_written_only_through_a_moved_pointer_is_reported_where_read_at_its_start: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same written a level down, `**pe` with `pe = &e`, which knows
        // the element `e` points at too. Mutation: as above; this goes
        // silent.
        a_local_array_written_through_a_moved_pointer_a_level_down_is_reported_where_read_at_its_start: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same written by a library copy through `m + 1`. Mutation: no
        // write marks an array partly written; this goes silent.
        a_local_array_copied_into_through_a_moved_pointer_is_doubted_where_read_at_its_start: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same written by a call handed `m + 1`. Mutation: a call marks
        // nothing partly written; this goes silent.
        a_local_array_handed_to_a_call_through_a_moved_pointer_is_doubted_where_read_at_its_start: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m[1]` read after `m[1] = &x` is the element written, and is not
        // reported. Mutation: a write through a known element clears none;
        // this gains `SC0411`.
        a_local_array_element_written_and_read_through_a_moved_pointer_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written in part and then at its start, it is written, as on
        // `main`. Mutation: a write through a known element clears none;
        // this gains an `SC0411`.
        a_local_array_written_in_part_and_then_at_its_start_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m[0] = &x;` writes `m[0]` alone, so `m[1]` read after it was
        // written by nothing: a certain `SC0411` (#667). See ADR-0057.
        // Mutation: a write through a known element clears every element;
        // this goes silent.
        an_element_of_a_local_array_nothing_wrote_beside_one_written_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Every element written one at a time is the whole array written.
        // Mutation: a write through a known element clears none; this gains
        // `SC0411`.
        every_element_of_a_local_array_written_one_by_one_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m[0][1]` and `m[1][0]` are two elements, counted across both
        // dimensions. Mutation: a pointer moves by its constant alone, not
        // times the elements one pointee holds; this goes silent.
        an_element_of_a_two_dimensional_local_array_nothing_wrote_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An array of more than 64 elements is kept whole, as before #667,
        // so a write to `m[0]` writes it. Mutation: no cap; this gains
        // `SC0411`.
        an_element_of_a_local_array_too_long_to_keep_per_element_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m + 1` stored and read back keeps no element, so the write
        // through it may be of any element: `m[0]` is an `SC0411` doubt
        // (#667). Mutation: a stored pointer marks nothing
        // `Known::stored_in_part`; this goes silent.
        a_local_array_written_through_a_moved_pointer_read_back_out_of_memory_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `fill(&e, &x)` writes `m` through `e = m + 1`, a level below what
        // it is handed: an `SC0411` doubt at `m[0]`. Mutation: a call asks
        // only the pointer it is handed; this goes silent.
        a_local_array_written_by_a_call_through_a_moved_pointer_a_level_down_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call handed `m + 1` may keep it, and a later call may write
        // through it: still in part. Mutation: `Known::exposed_in_part`
        // adds nothing at a later call; the doubt goes.
        a_local_array_handed_in_part_to_a_call_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m[i]` in a loop is an offset this check cannot read, which
        // writes the whole array, as on `main`. Mutation: such a write
        // leaves the array partly written; this gains an `SC0411` doubt.
        a_local_array_filled_by_a_loop_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same fill inside an outer loop that makes `m` again each
        // turn: the inner head holds what its back edge brought, and the
        // outer loop's next turn arrives at it from before the loop.
        // Mutation: a head joins what arrives from before it straight into
        // what it had; this gains an `SC0411` doubt, which `main` does not
        // give. Found by review of #668.
        a_local_array_made_in_an_outer_loop_and_filled_by_an_inner_loop_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same of a pointer local whose address is taken. Mutation: as
        // above; this gains an `SC0411` doubt.
        a_pointer_local_made_in_an_outer_loop_and_assigned_in_an_inner_loop_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `fill(m)` may write all of `m`, as on `main`. Mutation: a call
        // handed an array at its start writes it in part; this gains an
        // `SC0411` doubt.
        a_local_array_handed_to_a_call_at_its_start_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `e = m + 1` is element one on both paths into the join, so the
        // write after it is of `m[1]` alone and `m[0]` stays unwritten.
        // Mutation: a join forgets an element both paths agree on; the
        // `SC0411` is a doubt.
        an_element_left_unwritten_beside_one_written_after_two_paths_meet_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `e` walked back by a loop has lost its element, and `m` was
        // written in part, so `*e` may read the element nothing wrote: an
        // `SC0411` doubt. Mutation: a read at an unknown element is never
        // doubted; this goes silent.
        a_read_through_a_pointer_that_lost_its_element_is_doubted_where_an_element_may_be_unwritten: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `box[0] - 1` reads the stored `m + 1` as an operand rather than
        // through a load, and the write through it may be of any element:
        // `m[1]` is an `SC0411` doubt. Mutation: such an operand takes no
        // mark from `Known::stored_in_part`; this goes silent.
        a_local_array_written_through_a_pointer_moved_as_it_is_read_out_of_memory_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The moved pointer stored in the first turn named that turn's `m`,
        // so this turn's `box[0] = m` writes it whole (C17 6.2.4 p2).
        // Mutation: `StorageLive` leaves `Known::stored_in_part` alone; this
        // gains an `SC0411` doubt.
        a_local_array_written_through_memory_after_an_earlier_turn_stored_a_moved_pointer_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The first turn handed that turn's `m` in part; this turn's `m` is
        // a new object, handed whole. Mutation: `StorageLive` leaves
        // `Known::exposed_in_part` alone; this gains an `SC0411` doubt.
        a_local_array_handed_whole_after_an_earlier_turn_handed_it_in_part_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `c - 8` with `c` a `char *` moves one pointer, not eight, so the
        // write is of an element this check cannot count: `m[0]` is an
        // `SC0411` doubt. Mutation: a byte counts as an element; this goes
        // silent.
        a_local_array_written_through_a_byte_pointer_moved_back_by_a_pointer_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `c + 8` is `m[1]`, written; `m[8]` is not. Neither is a certain
        // `SC0411`, the element written being one this check cannot count.
        // Mutation: a byte counts as an element; `m[1]` is a certain
        // `SC0411` and `m[8]` goes silent.
        a_local_array_written_through_a_byte_pointer_is_not_proved_unwritten_at_the_element_written: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `f = *pe` reads `e = m + 1` back out of `e` itself, which no store
        // marked, and the write through it may be of any element: `m[0]` is
        // an `SC0411` doubt. Mutation: a load takes no edge from the local
        // it reads out of; this goes silent.
        a_local_array_written_through_a_moved_pointer_read_back_through_its_address_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same read as an operand, `*pe - 1`, which moves it back onto
        // `m[0]`; `m[1]` is an `SC0411` doubt. Mutation: such an operand
        // takes no edge from the local it reads out of; this goes silent.
        a_local_array_written_through_a_moved_pointer_read_back_through_its_address_as_an_operand_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `fill(h)` reaches `m` only through `*h = m + 1`, stored in an
        // allocation, and may write any element: `m[0]` is an `SC0411`
        // doubt. Mutation: a call takes no mark from
        // `Known::stored_in_part`; this goes silent.
        a_local_array_reached_by_a_call_through_a_moved_pointer_stored_in_an_allocation_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `**q` writes `m[1]` through `e`, or `m[0]` through what the
        // allocation holds, so `m[0]` is an `SC0411` doubt. Mutation: a
        // pointer read out of memory keeps the element of the one beside
        // it; this is a certain `SC0411`.
        a_local_array_written_through_a_pointer_that_may_be_read_out_of_memory_is_not_proved_unwritten: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m[c]` may be `m[0]` or `m[2]`, which nothing wrote, or `m[1]`:
        // an `SC0411` doubt. Mutation: an element nobody can name is asked
        // only of an array written in part; this goes silent.
        an_element_of_a_local_array_read_at_an_offset_this_check_cannot_read_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `f` is `m` or `m + 1`, so its element is lost at the join and
        // `*f` may read either: an `SC0411` doubt. Mutation: as above; this
        // goes silent.
        a_read_through_a_pointer_to_one_of_two_unwritten_elements_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*w` reads `m[0]`, which may be unwritten, or a fresh allocation:
        // an `SC0411` doubt. Mutation: a read that may reach an allocation
        // is never doubted; this goes silent.
        a_read_through_a_pointer_to_an_array_written_in_part_or_to_fresh_memory_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*w` reads `m[0]`, which may be unwritten, or `slot`, which is:
        // an `SC0411` doubt.
        a_read_through_a_pointer_to_an_array_written_in_part_or_to_an_unwritten_local_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `fill(m)` may write all of `m`, whatever `fill(&m[1])` kept.
        // Mutation: a call handed an array whole still writes in part what
        // an earlier call was handed in part; this gains an `SC0411`
        // doubt.
        a_local_array_handed_whole_to_a_call_after_one_handed_it_in_part_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `e - 1` with `e = m + 2` is `m[1]`, so `m[3]` read after the write
        // was written by nothing: a certain `SC0411`. Mutation: a
        // subtraction moves forwards; this goes silent.
        an_element_of_a_local_array_past_one_written_through_a_pointer_moved_back_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Sixty-four elements are kept one by one, the most that are. See
        // `ELEMENTS_KEPT`. Mutation: keep fewer than sixty-four; this goes
        // silent.
        an_element_of_a_local_array_of_sixty_four_pointers_nothing_wrote_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m[1] = &x` after a write of an element this check cannot count
        // leaves `m` written in part: `m[0]` is an `SC0411` doubt. Mutation:
        // a write at a known element forgets the array was written in part;
        // this is a certain `SC0411`.
        a_local_array_written_in_part_and_then_at_a_known_element_is_still_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `**pp` writes `m[1]` through `e1` or `m[0]` through `e2`: `m[0]`
        // is an `SC0411` doubt. Mutation: a write takes the element of the
        // first pointer it goes through whatever the others name; this is
        // a certain `SC0411`.
        a_local_array_written_through_a_pointer_that_may_name_either_of_two_elements_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An array too long to keep per element, written in part and then
        // at its start, is written, as on `main`. Mutation: a write of a
        // whole local leaves its partial mark; this gains an `SC0411`
        // doubt.
        a_long_local_array_written_in_part_and_then_at_its_start_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Each turn's `m` is a new object (C17 6.2.4 p2), so `m[0]` read
        // first is a certain `SC0411`, whatever the turn before wrote.
        // Mutation: `StorageLive` leaves `Known::partly_written` alone; this
        // is a doubt.
        a_local_array_written_in_part_in_an_earlier_turn_is_reported_unwritten_in_the_next: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `e` is `m + 1` on one path and `m + i` on the other, an edge
        // only #639's rule keeps on the first. Mutation: a join keeps that
        // mark only where both paths have it; this goes silent.
        a_local_array_written_through_a_pointer_moved_off_it_on_one_path_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a join keeps `Known::exposed_in_part` only where both
        // paths have it; this goes silent.
        a_local_array_handed_in_part_to_a_call_on_one_path_is_doubted_after_a_later_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a join keeps `Known::stored_in_part` only where both
        // paths have it; this goes silent.
        a_local_array_written_through_memory_that_may_hold_a_moved_pointer_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*e` is written on one path only, so the read of it is no read
        // of what was just written. Mutation: a join keeps
        // `Held::stored_through` where either path has it; this goes
        // silent.
        a_read_back_through_a_pointer_written_through_on_one_path_only_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `m[5]` is past `m`, which the bounds check reports; the record
        // of `m` is not indexed by it. Mutation: drop the length check
        // before the element is cleared; this panics.
        a_write_past_the_end_of_a_local_array_kept_per_element_is_read_as_no_element_of_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A write through a pointer kept from an earlier turn, `*p = 0`, is
        // no write of this turn's `m`, and `m[0]` is read unwritten: a
        // certain `SC0411`, as on `main`. Mutation: the pointer that loses
        // its edge is marked `lost`; this loses its `SC0411`, since a write
        // through a lost pointer lands in every local named (#664).
        a_write_through_a_pointer_kept_from_an_earlier_turn_does_not_hide_an_unwritten_local_array: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `p` may hold this turn's `x` or `a`, and is given `a` back before
        // the next turn: every read is defined. Mutation: the pointer that
        // loses its edge is marked `lost`; this gains `SC0402`.
        a_pointer_that_may_hold_an_earlier_turns_local_is_not_doubted_where_it_holds_another: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a library copy whose destination was read out of memory
        // writes every escaped local; this goes silent.
        a_pointer_read_out_of_a_local_array_a_copy_through_caller_memory_did_not_write_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: only a write deeper than one dereference, rather than
        // one through a pointer read out of memory on the way down, lands
        // in every local code this check cannot read may name; this goes
        // silent.
        a_pointer_read_out_of_a_local_array_a_copy_through_a_dereference_of_a_placed_pointer_did_not_write_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same through a pointer read out of the caller's memory, which
        // may be the array. **Held by two routes, measured**: the address
        // edges `t` carries out of that memory, and the level the walk
        // passes through being a load. Dropping either alone leaves this
        // silent; it marks the program the fix must not report.
        a_local_array_copied_into_through_a_dereference_of_a_pointer_read_out_of_memory_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An `int` stored through a pointer that may be the array cannot be
        // a write of its pointer elements (C17 6.5 p7). Mutation:
        // `replaced_by` drops the type from its write answer; this goes
        // silent.
        a_pointer_read_out_of_a_local_array_an_int_stored_where_it_may_point_did_not_write_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The loop writes `m` through `e`, which keeps its edge to the array
        // as it moves (#639), so the second free of `a` through `q` is
        // reported, and the read of `m[0]` is a doubt rather than a proof,
        // since a moved pointer may have written another element. See
        // ADR-0057. Mutation: the edge is dropped for every local again;
        // the `SC0401` goes and the `SC0411` is certain. Mutation: a store
        // marks nothing `Held::stored_through`; this gains a doubt at the
        // store (#667).
        a_double_free_through_an_array_filled_backwards_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer stored where code outside this function may read it is
        // asked at the store, as a return is (#641). See ADR-0041.
        //
        // Mutation: `stored_away` is not called; these two go silent.
        a_freed_pointer_a_callee_stores_through_its_parameter_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_freed_pointer_a_callee_stores_through_a_pointer_to_a_local_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a store of a pointer this check followed is asked as a
        // doubt whatever it holds; this is reported.
        a_live_allocation_a_callee_stores_through_its_parameter_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `leaves` does not ask `sites_named_by_unknown_code()`;
        // this loses its `SC0414`.
        a_freed_pointer_stored_in_memory_handed_to_a_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `leaves` does not ask `exposed_locals`; this loses its
        // `SC0414`.
        a_freed_pointer_stored_through_a_pointer_to_a_local_a_call_kept_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Stays in sight: the read back is what is asked. Mutation: every
        // store leaves; this gains an `SC0414`.
        a_freed_pointer_stored_in_memory_only_this_function_reads_is_not_reported_where_it_is_stored: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same store spelled without the pointer. Mutation: a write of a
        // local `exposed_locals` holds is not asked; this goes silent.
        a_freed_pointer_written_into_a_local_a_call_kept_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A box that left through a local a call kept, `slot = b;`, is
        // memory such code may read (#655). Mutation: `leaves` asks
        // `reachable_now()` in place of `sites_named_by_unknown_code()`;
        // this loses its `SC0414`. Its `SC0402` at the same store is not
        // about `x`; it is the false report #659, and fixing that removes it
        // from this case's `.stderr`.
        a_freed_pointer_stored_into_a_box_a_local_a_call_kept_points_at_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same two levels below the local, `*c = b; slot = c;`.
        // Mutation: the walk takes one step from the exposed locals and
        // stops; this loses its `SC0414`.
        a_freed_pointer_stored_into_a_box_two_levels_below_a_local_a_call_kept_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same with `x` live. Held by no mutation of `leaves`, measured:
        // a live pointer is not a freed one whichever memory it lands in, so
        // this marks what the store must not report.
        a_live_pointer_stored_into_a_box_two_levels_below_a_local_a_call_kept_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local whose address such a local holds, `slot = &t; t = x;`, is
        // a local code outside the function may read before any later call
        // marks it (#655). Mutation: `stored_away`'s direct write asks
        // `exposed_locals` in place of `locals_named_by_unknown_code()`;
        // this goes silent.
        a_freed_pointer_written_into_a_local_a_kept_local_points_at_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same store through a pointer, `*pt = x;`. Mutation: `leaves`
        // asks `exposed_locals` of the locals the store may land in; this
        // goes silent.
        a_freed_pointer_stored_through_a_pointer_to_a_local_a_kept_local_points_at_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A box holding a freed pointer stored into a box a kept local
        // points at is asked what it holds, a doubt as at the other doors
        // (#651). Its `SC0402` is #659. Mutation: `leaves` asks
        // `reachable_now()`; this loses its `SC0414`.
        a_box_holding_a_freed_pointer_stored_into_a_box_a_local_a_call_kept_points_at_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer moved off a local array still names it (#639), and the
        // array's memory may leave. Mutation: the edge is dropped for every
        // local again; this loses the `SC0402` at the read back.
        a_freed_pointer_stored_through_a_pointer_moved_off_a_local_array_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // **Held by two routes, measured**: `through_unnamed`, and a pointer
        // naming nothing taken to land anywhere. Dropping either alone keeps
        // their `SC0414`; dropping both loses it.
        a_freed_pointer_stored_two_levels_below_a_parameter_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_freed_pointer_stored_through_a_pointer_read_out_of_a_parameter_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a parameter's memory held is a pointer this check lost, and
        // the words say so. Mutation: word the lost row as a freed one; this
        // fails.
        a_pointer_read_out_of_a_parameter_freed_and_stored_back_is_reported_as_one_this_check_lost: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What memory that leaves holds is asked at the door it leaves by, as
        // a call asks it (#651), at every level (#654). See ADR-0041.
        //
        // Mutation: `returned` does not ask `freed_behind`; this goes silent.
        a_box_holding_a_freed_pointer_returned_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `stored_away` does not ask `freed_behind`; this goes
        // silent.
        a_box_holding_a_freed_pointer_stored_through_a_parameter_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `freed_behind` doubts every allocation the box may
        // contain, live or not; this is reported.
        a_box_holding_a_live_pointer_returned_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The box left first, so the store into it is the proof of #641,
        // not the doubt behind it. Mutation: `stored_away` skips its own
        // verdict and asks only what is behind its value; this loses its
        // `SC0414`.
        a_freed_pointer_put_into_a_box_already_stored_through_a_parameter_is_reported_at_the_put: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A box written into a local a call kept, the store door's direct
        // write. Mutation: a direct write does not ask `freed_behind`; this
        // goes silent.
        a_box_holding_a_freed_pointer_written_into_a_local_a_call_kept_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `returned` asks `freed_behind` only of a plain local;
        // this goes silent.
        a_box_holding_a_freed_pointer_returned_through_a_dereference_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `freed_behind` doubts every allocation the box may
        // contain, at the store door; this is reported.
        a_box_holding_a_live_pointer_stored_through_a_parameter_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a box held may be an allocation a loop has since made again
        // at the same site, freed on the turn after it was put there. The
        // three doors. Mutation: `handed_below` does not read `stale`; these
        // three go silent.
        a_box_filled_on_one_turn_and_freed_through_on_the_next_returned_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_box_filled_on_one_turn_and_freed_through_on_the_next_stored_through_a_parameter_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_box_filled_on_one_turn_and_freed_through_on_the_next_handed_to_a_call_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What leaves is asked at every level, not one (#654). See ADR-0042.
        //
        // Mutation: `handed_below` stops at the first level; these three go
        // silent.
        a_box_of_a_box_holding_a_freed_pointer_handed_to_a_call_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_box_of_a_box_holding_a_freed_pointer_returned_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_box_of_a_box_holding_a_freed_pointer_stored_through_a_parameter_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // **Held by no mutation, measured**: a live allocation is never a
        // doubt, so keeping a site below the first level whatever its state
        // leaves this silent. It marks the program the walk must not report.
        a_box_of_a_box_holding_a_live_pointer_returned_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer a call may have freed two boxes in: `Reachable`, counted
        // where an allocation on the way holds it, as the first level counts
        // it. Mutation: `Reachable` is not counted below the first level;
        // these two go silent.
        a_box_of_a_box_holding_a_pointer_a_call_may_have_freed_handed_to_a_call_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_box_of_a_box_holding_a_pointer_a_call_may_have_freed_returned_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the walk stops after its first pass below the first
        // level; this goes silent.
        a_box_three_levels_above_a_freed_pointer_handed_to_a_call_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `Unknown` is not counted below the first level; this goes
        // silent.
        a_box_of_a_box_holding_a_pointer_a_call_was_handed_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `stale` is read only at the first level's containers;
        // this goes silent.
        a_box_of_a_box_filled_on_one_turn_and_freed_through_on_the_next_handed_to_a_call_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // **A cost, recorded in ADR-0042.** The caller's pointer two boxes in,
        // after a call that may have freed the caller's memory, is doubted;
        // one box in, it already was. Two routes reach it: the caller's
        // allocation is `Reachable` and held in memory, and the call left
        // the box `stale`. Either mutation alone, `Reachable` not counted
        // below the first level or `stale` read only at the first level's
        // containers, leaves it doubted; both together silence it.
        a_box_of_a_box_holding_the_callers_pointer_handed_on_after_a_call_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call writes only the locals it may reach, `Known::exposed_locals`
        // (#643). See ADR-0057.
        //
        // Mutation: an opaque call passes `|_| true` as `may_write`; this
        // goes silent.
        a_pointer_read_out_of_a_local_array_an_unrelated_call_did_not_reach_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a library copy passes `|_| true` as `may_write`; this
        // goes silent.
        a_pointer_read_out_of_a_local_array_a_copy_elsewhere_did_not_write_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the opaque arm skips `written_through` for a plain
        // argument; this is reported.
        a_local_array_handed_to_a_call_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `expose_locals` is asked before `expose` at the call,
        // so what it was handed is not yet reachable; this is reported.
        a_local_array_stored_in_memory_a_call_was_handed_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the walk does not step through what a local holds;
        // this is reported.
        a_local_array_reached_through_a_local_a_call_was_handed_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the locals already exposed are not a seed of the walk;
        // this is reported.
        a_local_array_put_in_a_local_an_earlier_call_kept_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An address placed in memory that only memory already handed over
        // reaches, rather than in that memory itself. Mutation: the walk
        // does not start from `reachable_now()`; this is reported.
        a_local_array_stored_in_memory_reached_from_memory_already_handed_away_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `exposed_locals` joined by intersection; this is
        // reported.
        a_local_array_put_in_a_local_a_call_kept_on_one_arm_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An address stored through a pointer this check cannot place,
        // which records it in every allocation such code can reach.
        // Mutation: the walk does not start from `reachable_now()`; this is
        // reported.
        a_local_array_stored_where_this_check_cannot_place_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the opaque arm skips `levels_below` for an argument
        // read through a dereference; this is reported.
        a_local_array_handed_to_a_call_as_a_pointer_read_out_of_a_local_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `StorageLive` leaves `exposed_locals` alone; the second
        // turn's read goes silent.
        a_pointer_read_out_of_a_local_array_made_again_after_a_call_kept_the_old_one_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a library copy whose first argument is a dereference
        // writes every escaped local; this goes silent.
        a_pointer_read_out_of_a_local_array_a_copy_through_a_dereference_elsewhere_did_not_write_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a library copy whose first argument is a dereference
        // writes no local; this is reported.
        a_local_array_copied_into_through_a_dereference_is_not_reported_as_indeterminate: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `d` points at `m[0]` (C17 6.5.6 p8) and keeps its edge to the
        // array as it moves (#639), so the call may write `m` in part and
        // the read of `m[0]` is an `SC0411` doubt. See ADR-0057. Mutation:
        // the edge is dropped for every local again; the `SC0411` is
        // certain.
        a_local_array_handed_to_a_call_through_a_pointer_moved_back_onto_it_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `aligned_alloc`'s contents are indeterminate too (C17 7.22.3.1 p2).
        // Mutation: drop it from `leaves_indeterminate`. It builds.
        a_pointer_read_out_of_memory_aligned_alloc_made_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read through a null pointer, which names no allocation, so the
        // read is not said to come out of fresh memory. Mutation: drop the
        // clause in `reads_fresh` that asks for a site. `SC0411` joins the
        // null report.
        a_pointer_read_through_a_null_pointer_is_not_said_to_come_from_fresh_memory: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Only compared, which Annex J.2 lists as undefined as well: reported
        // at the read, whatever it is read for (#587). Mutation: drop the
        // call to `loaded_indeterminate` for an element. It builds. See
        // ADR-0057.
        a_pointer_read_out_of_fresh_memory_and_only_compared_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read and the value thrown away. Mutation: as above. It builds.
        a_pointer_read_out_of_fresh_memory_and_discarded_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read as an operand of a comparison, with no temporary. Mutation:
        // read only `Rvalue::Use` in `read_out_of_memory_in_element`. It
        // builds.
        a_pointer_read_out_of_fresh_memory_in_a_comparison_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same read on both sides, one caret, one report. Mutation:
        // drop the check of `said` in `loaded_indeterminate`. It panics, as
        // a second proof at one caret names no free.
        a_pointer_read_out_of_fresh_memory_twice_in_one_comparison_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // On the right. Mutation: drop the right operand in
        // `read_out_of_memory_in_element`. It builds.
        a_pointer_read_out_of_fresh_memory_on_the_right_of_a_comparison_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `**box` as an operand reads `*box` on the way. Mutation: answer
        // `None` for anything but `*l` in `read_as_value`. Only the null
        // doubt is left.
        a_pointer_read_out_of_fresh_memory_and_read_through_in_one_operand_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read as the operand of a unary operator, with no temporary.
        // Mutation: read nothing for `Rvalue::Unary` in
        // `read_out_of_memory_in_element`. It builds.
        a_pointer_read_out_of_fresh_memory_under_a_logical_not_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read as an argument, with no temporary. Mutation: drop the
        // arguments in `read_out_of_memory_in_terminator`. It builds.
        a_pointer_read_out_of_fresh_memory_and_handed_to_a_call_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read by an expression statement, which converts it (C17 6.3.2.1
        // p2). Mutation: answer nothing for `Element::Evaluate` in
        // `read_out_of_memory_in_element`. It builds.
        a_pointer_read_out_of_fresh_memory_by_an_expression_statement_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Read on the way through two dereferences. Mutation: answer `None`
        // in `first_of_two_derefs`. It builds.
        a_pointer_read_out_of_fresh_memory_through_two_dereferences_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Written, which reads nothing out of it. Mutation: answer a place
        // written through one dereference in `first_of_two_derefs`. `SC0411`.
        a_pointer_written_into_fresh_memory_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Its address is taken, so a write through it may assign it and this
        // check does not follow one. Mutation: drop the exclusion. It is
        // reported.
        a_pointer_a_call_fills_in_through_its_address_is_not_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Each turn begins a new `p` (C17 6.2.4 p6), so last turn's assignment
        // is no assignment of this one. Mutation: have `StorageLive` leave the
        // bit as it is. The turn before assigned it, and the free is believed.
        a_pointer_declared_in_a_loop_is_unassigned_on_every_turn: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `p = *p;` reads `p` before it writes it, in one element; `p = p + 1;`
        // goes through a temporary and cannot tell the order apart. Mutation:
        // take an element's write before its reads. It builds.
        a_pointer_read_and_written_by_one_statement_is_read_first: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same proof inside a hatch, which reports what it proved (ADR-0038).
        // Mutation: conclude it `Unknown` where no path assigns. It is listed and
        // not reported.
        a_pointer_never_assigned_in_a_hatch_is_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // A loop per write over 150 allocating locals: about 1.3 GB by the
        // check's estimate of what it would hold, past its bound, so it is one
        // `Unknown` at the name and not read (#572). `hatches` because the
        // function's IR is thousands of lines. Mutation: drop the guard. The
        // check runs for seconds and about 1.2 GB, and the expected `.stderr`
        // no longer matches.
        a_function_too_large_for_the_memory_check_is_unknown: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // The same under `--allow-unknown`: a warning, and it builds. Mutation:
        // conclude it `Unsafe`. It is still an error.
        a_function_too_large_for_the_memory_check_is_a_warning_under_allow_unknown: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // The same function as a hatch, with a double free the check proves
        // below the bound. A hatch lists what was not proved, and this one was
        // not read, so it is reported: listed only, the double free built in
        // silence. See ADR-0038. Mutation: route it as any other `Unknown`.
        // It exits 0.
        a_hatch_too_large_for_the_memory_check_is_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
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
        // `used here, where it may have been freed before`, and **no** `freed here` caret
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
        // A program with no `free` in it, whose pointer is doubted only
        // because calls this check cannot read were handed it, and whose
        // words assert no free: a use, a pass and a return, each certain, of
        // a pointer that may have been freed. Mutation: give the
        // `Disagreement` rows in `memory_finding` their old words back; every
        // line here says a free happened. The double free's row is held by
        // `a_memory_diagnostic_names_the_value_it_is_about`, which needs a
        // `free` to reach it.
        a_doubt_with_no_free_in_the_program_asserts_none: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer whose address was taken is doubted, and not told of a free
        // nothing made: an allocation and a parameter with no free, which get
        // one set of words, and a read after `free(*pp)`, which is a use after
        // free on every execution and has to stay a doubt rather than be told
        // this check merely stopped following. Saying the address was taken
        // needs the lattice to know why a site is reachable, which is #471.
        // Mutation: count a `SiteState::Reachable` site as lost in
        // `memory::report::verdict`; the third is told nothing says the
        // program is wrong. Mutation: give the `Disagreement` return row its
        // old label; the first two say a free happened.
        a_pointer_whose_address_was_taken_is_doubted_without_being_told_of_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // Each has a sibling that reads through a pointer, `*p` or `*w`,
        // because one that cannot act leaves the marker in (#178). Mutation:
        // answer that at most one operand acts everywhere, or count a
        // dereference as inert; these gain a proof.
        a_comma_inside_a_call_argument_orders_nothing_outside_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_logical_and_inside_an_unsequenced_operand_orders_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_conditional_inside_an_unsequenced_operand_orders_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same operators below a node whose other operands cannot act,
        // whose order C17 6.5.17 p2 and 6.5.13 p4 fix whatever encloses them:
        // an assignment to a local, a negation, and a call whose other
        // argument is a constant. Each is the proof `int x = (free(p), *p);`
        // is (#178). Mutation: drop the root at every node that does not
        // sequence, as before; all four are doubts again. And a call beside
        // one acts, so it keeps its doubt. Mutation: count a call as inert;
        // it is proved.
        an_assignment_of_a_comma_that_frees_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_assignment_of_a_logical_and_that_frees_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_negation_of_a_comma_that_frees_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_logical_and_in_an_argument_beside_a_constant_is_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_beside_a_comma_that_frees_keeps_it_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A sibling that reads through a pointer by any spelling acts, and so
        // does the place of a store through one: each read becomes an element
        // before the `?:`'s marker, which would clear it before the free. So
        // each of these keeps its doubt. Mutation: count a subscript, a
        // post-increment, or a dereferenced place whatever it dereferences,
        // as inert; each in turn goes silent. Found by review.
        a_subscript_beside_a_conditional_that_frees_keeps_it_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_increment_beside_a_conditional_that_frees_keeps_it_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_store_through_a_pointer_read_beside_a_conditional_that_frees_keeps_it_unproven: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // One argument writes a pointer another reads, which C17 6.5 p2 makes
        // undefined whatever it holds, so the conflict is the report, `SC0409`,
        // and a proof: through a `_Nonnull` parameter, a dereference, a
        // pointer freed before the call, and a write inside a comma, which
        // orders nothing outside its own argument (#247). Mutation: have the
        // lowering find no conflict, or the memory check's arm for the
        // element do nothing; all five below go silent. Two writes are the
        // same conflict in other words. Mutation: count only a read as the
        // other access, or report a local once per write rather than once
        // per call; the fifth goes silent, or reports twice.
        an_argument_read_while_another_argument_writes_it_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_dereference_in_an_argument_another_argument_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_freed_pointer_read_in_an_argument_another_argument_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_inside_a_comma_is_unsequenced_against_another_argument: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        two_arguments_writing_one_pointer_are_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A write by name is an increment as much as an assignment, and a
        // write through `q` reads `q`. Mutation: leave out an increment's
        // write, or walk nothing below an assignment's place that is not a
        // name; each goes silent.
        an_argument_incremented_while_another_reads_it_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_through_a_pointer_another_argument_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*&q = &x` is a write of `q` by name, because the lowering reads
        // `*&E` as `E`, so a walk that took it for a write through a pointer
        // left the read of a freed `q` silent. Mutation: do not see through
        // `*&`; it goes silent. Found by review.
        a_write_through_the_address_of_a_pointer_is_a_write_by_name: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The element goes before the call's `ArgumentsEvaluated`, which a
        // block has to end with for ADR-0027's null exemption to hold, so the
        // free `realloc` makes of a `p` that is null in every order is not
        // reported beside the conflict. Mutation: emit the element where the
        // call finishes; an `SC0401` appears. Found by review.
        a_conflict_leaves_a_null_argument_exempt_from_the_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What counting a write behind `&&`, `||` or `?:` costs: one under a
        // condition no execution satisfies is refused, though C defines the
        // program. Where the condition can hold, the call is undefined on that
        // execution, which is what `Unsafe` says. Found by review; see
        // ADR-0052.
        a_write_that_never_runs_is_still_a_conflict: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What is not a conflict. `&q` takes the address and reads nothing
        // (C17 6.3.2.1 p2), and a write and a read inside one argument are
        // ordered by the comma between them. Mutation: count `&q` as a read,
        // or compare an argument with itself; each reports `SC0409`. And what
        // is not asked about: `k(i, i++)` is undefined by the same paragraph,
        // and an integer is not what this check is believed about. Mutation:
        // drop the pointer filter; it reports. See ADR-0052.
        an_address_taken_while_another_argument_writes_it_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_and_a_read_in_one_argument_are_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_integer_another_argument_writes_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same conflict between the operands of an operator that does not
        // sequence (#525): an assignment whose place reads the pointer its
        // value writes, through `*` and through `[]`, a sum, and a subscript
        // whose index writes its base. C17 6.5 p2 makes each undefined, and
        // each is `SC0409` in an operator's words. Mutation: push no
        // `Task::Conflicts` for an assignment, a binary operator or a
        // subscript; the first three, the fourth, or the fifth goes silent.
        // Mutation: word `Between::Operands` as a call's arguments; all five
        // fail on their `.stderr`.
        a_store_through_a_pointer_the_value_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_store_through_a_freed_pointer_the_value_replaces_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_store_into_an_element_of_a_pointer_the_value_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_sum_reading_a_pointer_its_other_operand_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_subscript_whose_index_writes_its_base_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two parameters' lengths are unsequenced against each other (C17
        // 6.7.6 p3), so they are asked as a call's arguments are, and the
        // words name a length (#529). Mutation: `Lowering::lengths` does not
        // call `conflicts`; both exit 0. Mutation: pass `Between::Operands`
        // there; both fail on `.stdout` and `.stderr`. Mutation: word
        // `Between::Lengths` as operands; both fail on `.stderr`.
        a_pointer_one_length_reads_and_another_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        two_lengths_writing_one_pointer_are_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Every length against every other, not only the first two or the
        // adjacent ones: here the write is in the third and the reads in the
        // first two. Mutation: build `each` from `walked.iter().take(2)`;
        // this exits 0. Found by review.
        a_pointer_the_third_length_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A conflict inside one length keeps the words of what the two are
        // operands of, and is told to write in the body, since a declarator
        // holds no statement (#531). Mutation: answer the remedy from
        // `between` alone; both are told to use a statement of their own.
        // Mutation: give the body the length's remedy too;
        // `a_sum_reading_a_pointer_its_other_operand_writes_is_reported` and
        // `an_argument_read_while_another_argument_writes_it_is_reported`
        // fail beside the other body cases.
        a_conflict_between_operands_inside_a_length_is_told_to_write_in_the_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_conflict_between_arguments_inside_a_length_is_told_to_write_in_the_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An integer two lengths write is undefined by the same paragraph and
        // not what this check is believed about, as for a call's arguments.
        // Mutation: drop the pointer filter in `conflicts`; this reports.
        an_integer_two_lengths_write_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What is not one: the store is sequenced after the value computations
        // of both operands (C17 6.5.16 p3), and here the comma orders the
        // inner write before the value is computed (6.5.17 p2). Mutation:
        // walk the place as a value, or count a comma's left operand as
        // unordered; this reports.
        a_store_into_a_pointer_its_value_replaced_first_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A compound assignment reads its place, `E1 = E1 op (E2)` (C17
        // 6.5.16.2 p3), so the read is unsequenced against the write in the
        // value. Mutation: walk a compound assignment's place as a place; it
        // goes silent. Found by review.
        a_compound_assignment_reading_a_pointer_its_value_writes_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The store is a write too, unsequenced against a write in the value
        // that no sequence point puts first: `q = q++`, `q = (q = s)`, and a
        // write in an arm of `?:`, which is not ordered before the value. Each
        // is undefined by 6.5 p2. Mutation: drop the store's own check; all
        // three go silent. Found by review.
        a_pointer_assigned_its_own_increment_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_an_assignment_to_itself_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_from_an_arm_that_writes_it_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A write below a binary operator, a subscript or a comma's right
        // operand is not ordered before the value either. Mutation: push the
        // operands of a `Binary`, of a `Subscript`, or a comma's right operand
        // as ordered in `unordered_writes`; the matching case goes silent.
        // Found by review.
        a_pointer_assigned_a_sum_that_increments_it_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_an_element_whose_index_increments_it_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_a_comma_that_increments_it_last_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And where a sequence point does put the write first, C defines it: a
        // call's argument (6.5.2.2 p10), the condition of `?:` (6.5.15 p4) and
        // the left of `&&` (6.5.13 p4). Mutation: count any one of those as
        // unordered; its case reports.
        a_pointer_assigned_a_call_handed_its_increment_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_after_a_condition_increments_it_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_after_a_logical_and_increments_it_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A parameter written as an array is the pointer C17 6.7.6.3 p7 makes
        // it, so its function lowers and a read after its free is one.
        // Mutation: leave a parameter's type as written; the function is
        // `SC0304`, a type the IR cannot hold, and the read goes unasked.
        a_parameter_declared_as_an_array_is_followed_as_a_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // One whose length C17 6.9.1 p10 evaluates on entry, here a free, is
        // evaluated before the body, and behind the sequence point at the end
        // of the function's declarator, so the read of `*p` is a proved use
        // after free. Mutation: drop the call to `Lowering::lengths`; this
        // exits 0 with its free gone. Mutation: drop the marker after the
        // lengths; the use is only suspected.
        a_free_in_a_parameter_s_length_is_before_the_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two parameters' lengths are unsequenced against each other (6.7.6 p3:
        // a parameter's declarator is not a full declarator), so the read in
        // `b` written before the free in `a` is still suspected. Mutation:
        // walk each length as a full expression, or put a marker after each;
        // the read is taken as first, and this goes silent.
        two_parameter_lengths_are_not_ordered_against_each_other: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A free and a use in one length are one expression, so the note is
        // 6.5.2.2 p10's as in a body, but no statement can be written in a
        // declarator, so the remedy is the length's (#444). Mutation: answer
        // `AcrossLengths` wherever the free is in a length; the note cites
        // 6.7.6 p3. Mutation: answer `InTheBody` always; the remedy says to
        // write separate statements.
        a_free_and_a_use_in_one_parameter_length_keep_the_call_s_note: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The pass-after-free row is worded by the same rule as the use, so
        // a pointer passed in one length and freed in another cites 6.7.6 p3
        // and the length's remedy. Mutation: keep `UNSEQUENCED_REMEDY` in
        // that row; the remedy says to write separate statements.
        a_pointer_passed_in_one_length_and_freed_in_another_is_told_why: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Every definition's lengths are collected, not only the first's, so
        // the free in `f`, after `h` and its own length, still cites 6.7.6
        // p3. Mutation: collect from `ast.items().iter().take(2)`; the note
        // falls back to 6.5.2.2 p10 and separate statements. Found by review.
        a_definition_s_lengths_are_found_after_another_definition: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A length's value is read only once every length is lowered, as a
        // call's arguments are read at the call, so the read of `*q` in `b`
        // is checked against the `q` that `a` may write first, and the free
        // of what `q` then points at reaches it. Mutation: discard each
        // length's value where it finishes; the read is checked before the
        // write and the `SC0402` goes, leaving only the `SC0409` the two
        // lengths' conflict over `q` is (#529).
        a_read_in_one_length_meets_what_another_length_frees: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // A load beside the free-and-null idiom holds only what it lists, so
        // `p`, which it cannot name, is not doubted after `if (c) { free(p);
        // p = 0; }` (#495). Mutation: have the early return in
        // `Known::held_sites` ask `loaded` again; this reports.
        a_pointer_freed_and_nulled_on_one_arm_is_not_doubted_beside_a_load: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What that return still costs for a pointer a call wrote, which it
        // keeps: the same idiom beside `get(&u)` is doubted though `u` cannot
        // name `p`. Mutation: leave `foreign` out of the return; it loses its
        // report at the free. And what it no longer costs for a lost local:
        // a free in a loop beside a local lost to that loop's rebirth is not
        // doubted, and `*l`, a real use of last turn's freed allocation, is
        // still reported (#517). Mutation: ask `lost` in the return again;
        // the second reports at the free.
        a_pointer_freed_and_nulled_on_one_arm_is_doubted_beside_a_pointer_a_call_wrote: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_free_in_a_loop_is_not_doubted_beside_a_local_lost_to_its_rebirth: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And what that same rule holds in the safety direction: `u` written
        // by a call this check cannot read may be `p`, freed before the call,
        // and `r` is `u` on the arm that freed and nulled `q`, so `free(r)`
        // may free `p` twice. Found by review of #495. Mutation: leave a
        // pointer a call wrote out of the early return in
        // `Known::held_sites`; until #497 this exited 0 with nothing said,
        // and now the report at `free(r)` changes its words.
        a_double_free_through_a_pointer_a_call_wrote_on_one_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And a use, through each kind of value the early return in
        // `Known::held_sites` stands for, on the arm that freed and nulled
        // what the other arm names: a pointer a call wrote, read after the
        // join; and a box holding a pointer freed two turns of a loop ago,
        // read back beside an unrelated load, and beside an unrelated local
        // lost to the same loop. Each is a use after free on that arm. Found
        // by review of #495. Mutation: leave a pointer a call wrote out of the
        // early return; the first goes quiet. The other two are doubted since
        // #503 by the free of the lost value, and keep a report with lost and
        // loaded locals out of the return, in the words "this check stopped
        // following" (#495). And the box read back after a call this check
        // cannot read released the lost value it was handed, doubted since
        // #505 by the call itself. And an alias of `p` this check
        // lost to a call, freed on one arm. Found by review of #495.
        a_use_through_a_pointer_a_call_wrote_on_one_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_use_after_free_through_a_box_beside_a_load_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_use_after_free_through_a_box_beside_a_lost_local_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_use_after_free_through_a_box_after_a_call_released_a_lost_value_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        an_alias_this_check_lost_of_a_pointer_freed_on_one_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A free of a pointer a call this check cannot read wrote is a doubt
        // whatever else it was joined with: `r` is `u` on one arm and `q` on
        // the other, so it names `q`'s site, and `free(r)` was read as a free
        // of `q` alone, though `u` may be `p`, freed before the call (#497).
        // Mutation: drop the `Held::foreign` row from
        // `Allocations::touching`'s reasons; the report at `free(r)` goes, in
        // the second, with the arms the other way round, the same, and in the
        // third, the same handed to `realloc`, the report at it. Mutation:
        // keep only the foreign side of a `Held` join; the second goes.
        // Mutation: ask the row whether or not the pointer names a site; the
        // fourth, an output parameter freed alone, changes its words to
        // "cannot say what this points at".
        a_free_of_a_pointer_a_call_wrote_on_one_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_a_pointer_a_call_wrote_on_the_other_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_realloc_of_a_pointer_a_call_wrote_on_one_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_free_of_an_output_parameter_is_one_this_check_knows_no_allocation_for: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // `Element::ArgumentsEvaluated`, which ADR-0026 emits only where the
        // root reaches the call, and its absence here is what refuses the
        // exemption. C17 6.5 p2 makes the program undefined outright, and since
        // #525 the write and the read of `p` are reported beside it as
        // `SC0409`; the `SC0401` is still the guard. Dropping that term reports nothing at all
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
        // The same with seventy locals declared first, so that `x` is past the
        // first word of every set the check holds: the cases above fit in one.
        // `hatches` because the IR is long. Mutation: have `Bits::any` read only
        // the first word. It exits 0 with nothing printed.
        a_free_of_a_pointer_that_may_hold_the_address_of_a_local_past_the_sixty_fourth_is_unproven: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
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
        // `pp[1]` builds as well since the backend writes pointer arithmetic
        // (#621), and `an_ir_shape_the_backend_cannot_write` holds a refusal
        // that is left.
        a_zero_subscript_reaches_the_backend: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        // A message names the local its conclusion is about, read as itself
        // or read through at its first level: a double free, a free through
        // `q` then a use through `p` that names `p`, two uses at one caret
        // told apart, `**pp` after `free(pp)`, a return, an argument, a free
        // this check stopped following and an interior free. Mutation: the
        // driver passes `None` for the name; every message here loses it.
        // Mutation: the named use row is given the double free's words; the
        // uses fail.
        a_memory_diagnostic_names_the_value_it_is_about: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And a message about a pointer no written name holds names nothing,
        // a temporary or a pointer read out of memory: `p[2]`
        // reads a temporary holding `p + 2`, `free(*pp)` frees what `pp`'s
        // allocation held, and `**pp` with `pp` proved live is reported at
        // its second level. Mutation: `own_local` answers a projection's
        // root; `free(*pp)` says `pp`. Mutation: `used` names the local for a
        // level below the first; the last says `pp`.
        a_memory_diagnostic_about_an_unnamed_pointer_names_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    }

    "nullability" => {
        // The nullability check's cases, kept together for the reason the
        // memory check's are: one table, whose rows are the three-valued model
        // met by a program that proves the answer, one that leaves it unproven,
        // and one that tests the pointer and so needs neither.
        //
        // The row that proves it.
        a_null_pointer_dereferenced_is_unsafe: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two parameters' lengths are unsequenced (C17 6.7.6 p3), so `*p` in
        // `b` may follow `p = 0` in `a`. A length's value is read only once
        // every length is lowered, so the read meets the null. Mutation:
        // discard each length's value where it finishes; the `SC0403` goes,
        // leaving only the `SC0409` the two lengths' conflict over `p` is,
        // which is what the same pair prints in a body, `*p + (p = 0, 1)`
        // (#529).
        a_read_in_one_length_meets_a_null_another_length_writes: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // Code after a call that does not return is reached by nothing
        // (ADR-0051). Mutation: have the lowering give every call its
        // continuation; the first is reported. And a definition of `abort`,
        // above the call or below it, is refused at the definition (#343):
        // C17 7.1.3 reserves the name, and honouring the definition here
        // while believing it in another file of the run was a silence.
        // Mutation: have names report no reserved definition; the second and
        // third build, and the read after the call is asked of nothing.
        code_after_abort_is_not_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_function_defined_here_named_abort_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_function_defined_below_its_call_named_abort_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local or a parameter named `abort` or `exit` has no linkage, so
        // 7.1.3 p1 does not reserve it there, and names never asks. (`_Exit`
        // is reserved for any use, by p1's first bullet, and is accepted
        // there all the same, as `clang` accepts it.) Mutation: ask every
        // declaration rather than function definitions; this is refused.
        a_local_named_exit_is_not_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A name that only begins with one of the four is an ordinary
        // function: it is neither refused where it is defined nor believed
        // where it is called, so the read after the call is asked. Mutation:
        // match a reserved name as a prefix, in names or in the lowering; this
        // is refused, or goes silent. Found by review.
        code_after_a_function_whose_name_begins_with_exit_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call inside a larger full expression keeps its edge, because a
        // read written before it is lowered after it. Mutation: drop the
        // `root` test where the call decides its continuation; both reads go
        // silent. And the nullability check ends that path only at the next
        // sequence point: ending it at the call's edge instead silences both.
        a_read_written_before_a_call_that_does_not_return_inside_an_expression_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The arm where `p` is null calls `abort` inside an expression, so the
        // path ends at the comma after it and does not reach `*p` (#590).
        // Mutation: never make a path `Path::Ending`. `SC0403` at `*p`.
        a_pointer_tested_null_on_an_arm_that_calls_abort_inside_an_expression_is_read_in_silence: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*p` after the comma that follows `abort` is never evaluated, though
        // `p` is null on that arm. Mutation: report on an ended path in the
        // replay. It is proved a null dereference.
        a_read_after_abort_on_the_null_arm_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*p += e` reads `*p` unsequenced against `e` (C17 6.5.16.2 p3), so
        // the read may come before `exit` and is asked. Mutation: count a
        // compound assignment's place as the plain store's in
        // `Lowering::at_most_one_acts`. The comma keeps its marker, the path
        // ends there, and the read is silent.
        a_compound_assignment_reading_its_place_beside_exit_is_still_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A condition read after the comma that follows `abort` is never
        // evaluated: the terminator half of the replay's skip. Mutation: ask
        // the terminator of an ended path. It is proved a null dereference.
        a_condition_read_after_abort_on_the_null_arm_is_not_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The `&&`'s left operand is a null test, and its right one is reached
        // only where it held. Mutation: stop reading through the operand's
        // normalisation, or stop threading the branch past the empty
        // re-branch where the arms meet. `SC0403`.
        a_pointer_tested_on_the_left_of_and_is_not_null_in_its_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `p == 0 || g()` returns where `p` is null, so `*p` after it is not.
        // Mutation: stop threading. `SC0403`.
        a_pointer_tested_null_on_the_left_of_an_or_that_returns_is_not_null_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `p != 0 || g()` reaches its body with `p` null wherever `g()` is
        // true, so the doubt is right: the right operand's path keeps `p`
        // null. Mutation: refine the `otherwise` edge as the `then` edge in
        // `Analysis::edge`. It is silent.
        a_pointer_tested_on_the_left_of_an_or_may_be_null_in_its_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `!(p == 0)` is `p != 0` (C17 6.5.3.3 p5) through one more `== 0`.
        // Mutation: ignore the polarity a `== 0` adds. `SC0403`.
        a_negated_null_test_on_the_left_of_and_is_not_null_in_its_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Three `== 0` turn the test over three times, so `p` is null where
        // it is written through, and that is proved. Mutation: let the last
        // `== 0` decide the polarity, `inverted = equal`, rather than each of
        // them. It is silent. An even count cannot tell which way a `== 0`
        // turns, which the next case holds.
        a_null_test_through_three_comparisons_with_zero_is_read_with_its_polarity: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // One `== 0` around a null test turns it over, so `p` is not null in
        // the body. Mutation: have `through_zero` answer `== 0` as `!= 0`. It
        // is proved null.
        a_null_test_through_one_comparison_with_zero_is_read_turned_over: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `0 == (p == 0)`, with the zero on the left. Mutation: have
        // `zero_beside` read a zero on the right only. `SC0403`.
        a_null_test_compared_with_zero_on_its_left_is_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The right operand's test reaches the join by a `Goto`, the only way
        // into it once the left operand's edge is threaded, and the walk
        // reads it there (#595). Mutation: stop the walk in
        // `tested_against_null` at the top of the branching block. `SC0403`.
        a_pointer_tested_on_the_right_of_and_is_not_null_in_its_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same under `||`, returning where the right operand's `q` is
        // null. Mutation: as above. `SC0403`.
        a_pointer_tested_null_on_the_right_of_an_or_that_returns_is_not_null_after_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `p != 0 || q != 0` reaches its body with `q` untested wherever `p`
        // is not null, so the doubt is right. **No mutation of this change
        // silences it**, measured: the left operand's `then` edge reaches the
        // body without passing the right operand, so whatever the walk
        // learns there joins with nothing known. It records that the walk
        // claims nothing for that path.
        a_pointer_tested_on_the_right_of_an_or_may_be_null_in_its_body: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // **The cost**: `|| 0` never reaches the body through its right
        // operand, but `0 != 0` is not folded, so the path where `p` is null
        // reaches it as far as this check can tell. It is here so the cost is
        // seen, and fails if the threading or the normalisation starts
        // claiming more than it reads.
        a_pointer_tested_on_the_left_of_an_or_with_zero_is_still_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // Mutation: give the operands' writes the whole `&&` span again; both
        // carets move to it. That the two stay two reports with two remedies
        // even at one caret is held by
        // `a_doubt_through_memory_beside_a_plain_one_at_one_caret_each_keep_their_remedy`
        // in `crates/safec-ir/tests/nulls.rs`.
        a_doubt_through_memory_beside_a_plain_one_is_told_its_own_remedy: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*p` and `**q` in the two arms of a `?:` share the conditional's
        // caret, and each is a report of its own with its own remedy,
        // whichever arm holds which. Narrowing an arm's write to the arm moves
        // this case.
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
        // nothing is known about. Each is a diagnostic of its own that names
        // its pointer, because a dereference is one finding per place: the
        // doubt is not hidden behind the proof beside it. Mutation: `report`
        // takes the worst of an element's places again; the doubt goes.
        a_proved_null_dereference_beside_an_unproven_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same pair with the proved one written second: the places an
        // element dereferences are collected destination first, so this is
        // the order in which a rule keeping the first of an element's places
        // would say only the doubt. Both are said in either order.
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
        // A message names the pointer its conclusion is about, by the memory
        // check's rule: a dereference proved and unproven, two pointers at one
        // caret as `*q = *p` and `*p + *q` each saying its own, `**pp` doubted
        // at `pp`'s own level, `**pp = 1;` said once although it is a write
        // and a read-back that name `pp` differently, and an argument and a
        // return, each proved and unproven. Mutation: the fold keys a
        // dereference on its name rather than its place; `**pp = 1;` is said
        // twice. Mutation: the driver passes `None` for the name; every
        // message here loses it. Mutation: the fold in
        // `nullability::findings` drops the local from its key; `*p + *q`
        // folds into one.
        a_nullability_diagnostic_names_the_pointer_it_is_about: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And a message about a pointer no written name holds names nothing:
        // `**pp` with `pp` established not null, so only the pointer it loads
        // is in doubt, `g(*pp)`, `return *pp;`, and a constant returned.
        // Mutation: `report` names the place's local where only the loaded
        // pointer is in doubt; the first says `pp`. Mutation: `own_local`
        // answers a projection's root; `g(*pp)` and `return *pp;` say `pp`.
        a_nullability_diagnostic_about_an_unnamed_pointer_names_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `*p = *q;` with both pointers escaped, so neither dereference
        // refines its pointer and the read-back asks `p` again. The places
        // arrive at the caret as `p`, `q`, `p`, the two about `p` not
        // adjacent, and each pointer is said once. Mutation: have the fold in
        // `nullability::findings` look only at its neighbour; `p` is said
        // twice in each function.
        two_escaped_pointers_written_through_one_another_are_each_said_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // the proved `*q` and the unproven `*p` meet at one caret, and each is
        // a finding of its own: the proof is reported and the suspicion
        // listed, which in a hatch is the difference between building and
        // not. Narrowing an arm's write to the arm moves this case.
        a_proved_null_dereference_in_one_arm_of_a_conditional_in_a_hatch_is_still_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // One pointer read through in both arms of a `?:`, proved null in one
        // arm and unproven in the other, in both orders, so the two findings
        // are about one place at one caret and the fold in
        // `nullability::findings` keeps one of them. Mutation: never let the
        // later replace the standing one; `proof_first` lists the proof as a
        // suspicion and the run exits 0. Mutation: always let it; so does
        // `doubt_first`. Narrowing an arm's write to the arm moves this case.
        a_proved_null_dereference_after_a_doubt_about_one_pointer_in_a_hatch_is_still_reported: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
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
        // No spelling of a level on a function is read, because levels are
        // not mixed inside a translation unit. See ADR-0053. Mutation: accept
        // `safec_level(...)` beside the hatch; the function is no longer
        // refused.
        an_annotation_naming_a_level_is_refused: ["--emit", "ast"],
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
        // The buffer that grows with `realloc` in a loop, which C programs
        // write everywhere, is answered rather than walked until the budget
        // stops it. The `realloc` fact is recorded only while every old site
        // is `Live`, so its transfer is not monotone, and a header worked out
        // again from what comes back alternated between `Live` and `Unknown`
        // for good. The three doubts are false: the old buffer is freed only
        // where the call failed, and nothing reads it after that. See
        // ADR-0016. Mutation: fold at no block in `solve`; this panics.
        a_buffer_grown_by_realloc_in_a_loop_is_answered: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // A `realloc` fact holds nothing (#494): `if (c) p = realloc(p, 8);
        // if (!p) return 0;` is followed by a use and a free of `p` that are
        // defined on every path, and so is a free of `q` that is either the
        // result or `p`. Nothing names the old block on the arm that
        // overwrote it, so the other arm's `Live` is its state after the
        // join. Mutation: hold the fact's `old` in `Known::held_sites`
        // again; the first two report. Read through an alias, the old block
        // stays doubted, since the alias holds it. And where only one arm
        // dropped the old pointer after the `realloc`, the fact survives the
        // join: the null arm frees it once, and a use on the success arm is
        // proved. Mutation: count an `Unheld` side as disagreeing at the
        // join; the fourth reports at the free and the fifth is a doubt.
        a_pointer_reallocated_on_one_arm_is_not_doubted_after_its_null_test: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_reallocated_on_one_arm_and_copied_on_the_other_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_use_of_the_old_block_through_an_alias_after_a_realloc_on_one_arm_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_realloc_fact_survives_a_join_where_one_arm_dropped_the_old_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_use_after_a_realloc_is_proved_where_one_arm_dropped_the_old_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // The same with the arms the other way round, so the arm that left
        // the old block `Unheld` is the one the join keeps rather than the
        // one it takes in. The two cases above hold only the side taken in.
        // Mutation: count an `Unheld` side as disagreeing only where it is
        // the side kept; this reports at the free on the null arm.
        a_realloc_fact_survives_a_conditional_that_drops_the_old_pointer_on_its_second_arm: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
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
        // Nor does one of a pointer a call this check cannot read wrote,
        // joined with one it names: `r` is `u` or `q`, and `free(q)` after
        // the `realloc` of `u` frees `q` once (#499). Mutation: drop
        // `foreign` from the fact's condition; `free(q)` is proved a double
        // free again.
        a_realloc_of_a_pointer_a_call_wrote_joined_with_a_named_one_does_not_prove_a_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What that costs: with no fact, the null arm no longer says the old
        // allocations are live, so `free(q)` there is doubted, though `q`
        // is live on that arm whichever pointer `realloc` was handed. Found
        // by review.
        the_null_arm_of_a_realloc_of_a_pointer_a_call_wrote_doubts_the_one_it_was_joined_with: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
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
        // the second goes silent. And the write through `*t3` at `**t3 = r;`
        // is doubted, since `*t3` may be `*pp`, read after `cond()` staled
        // what a parameter points at (#482).
        a_store_two_levels_down_through_a_set_this_check_may_not_know_whole_is_exposed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A load out of memory this check cannot place reads what is exposed,
        // which is what code it cannot read may hold and what an unplaced
        // store put there. Stored through a load, freed and read back, the
        // allocation was read back as nothing and its use believed (#484); so
        // was one a call may have stashed. See ADR-0045. Mutation: have
        // `Known::levels_below` add no exposed site; every one of these goes
        // silent at the use. Mutation: ask only `foreign`; the first four do.
        // Mutation: ask only `loaded`; the fifth does, its pointer written by
        // a call. Mutation: follow a lost local no further, as before; the
        // fourth does, its `t` lost to the call it was handed to. The
        // second and third also doubt `u = *ppp`, since #487 lands `*t = m`
        // in `ppp`'s memory too, a false report the reader can see.
        an_allocation_stored_through_a_load_and_freed_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_stored_through_a_load_and_freed_is_doubted_when_read_back_one_level_further: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_stored_through_what_a_call_returned_and_freed_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_handed_to_a_call_and_freed_is_doubted_when_read_back_through_a_load: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_stored_through_a_pointer_a_call_wrote_and_freed_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And read back by `memcpy`, or as one place, `return **t;`: the
        // copy and the report each walk the levels themselves, and asked of
        // the load alone, `memcpy(d, t, 8)` and `**t` were believed where
        // `*d = *t;` and `q = *t; *q` were doubted. The same mutation as above
        // silences both.
        an_allocation_stored_through_a_load_and_freed_is_doubted_when_copied_back_by_memcpy: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_stored_through_a_load_and_freed_is_doubted_when_read_back_in_one_place: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The other direction: a store through a pointer this check cannot
        // place may land in memory a later load names, if code this check
        // cannot read can name it, so what it carries is recorded in every
        // exposed allocation and every one reachable from one (#487). See
        // ADR-0044. Mutation: record nothing there; both go silent at the use.
        // Mutation: record in the exposed allocations only; the first does,
        // since `*a = t;` records `t` in `a`'s memory without marking it.
        an_allocation_stored_through_a_load_may_land_in_memory_handed_to_the_caller: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_allocation_stored_through_a_load_may_land_in_another_parameter_s_memory: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And for the part of a store that is unplaced: `w` holds its own
        // allocation and, on one arm, a load, so the store is placed in the
        // first and unnamed for the rest. Mutation: ask only whether the store
        // is unplaced as a whole; this goes silent at the use.
        an_allocation_stored_through_a_pointer_that_may_also_be_a_load_may_land_in_memory_handed_to_the_caller: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Only a pointer lands: `x = +a;` copies the integer parameter's
        // site into `x`, and `**pp = x;` carried it into `pp`'s memory, so a
        // function with no free reported a use of freed memory at both reads
        // of `**pp`. Mutation: drop the `may_be_pointer` filter; this reports.
        an_integer_stored_through_a_load_lands_nowhere: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And the same store written as `memcpy`, which copies without going
        // through the store: `memcpy(w, s, 8)` was believed where `*w = *s;`
        // was doubted. Mutation: have the copy land nowhere when its
        // destination cannot be placed; this goes silent at the use.
        an_allocation_copied_through_a_load_by_memcpy_may_land_in_memory_handed_to_the_caller: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // review. Mutation: have `memcpy` carry no `Known::from_caller`; the
        // last two go silent. Having `reads_caller_memory` ask the local's
        // own allocations only silenced the first two until #486, and no
        // longer does: see that function's doc. It silences the use at
        // `return *s;` in
        // `a_caller_s_pointer_freed_two_levels_down_is_doubted_through_another_parameter`
        // below, a free of a caller's pointer two levels down read through
        // another parameter that may name it. Found by review.
        a_pointer_read_out_of_a_parameter_and_stored_is_doubted_when_read_back_two_levels_down_before_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_stored_is_doubted_when_read_back_two_levels_down_and_moved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_read_out_of_a_parameter_and_copied_by_memcpy_is_doubted_when_read_back_before_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        what_a_parameter_points_at_copied_by_memcpy_is_doubted_when_read_back_before_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A chain written as one place reads what the chain written through
        // locals reads, below the first level as a load does: `q = **ppp;`
        // after `**ppp = m; free(m);` read nothing of `m`, and the use at
        // `return *q;` was believed (#486). The report at the reload is
        // ADR-0044's: the unplaced store records `m` wherever a pointer this
        // check cannot place may point, `ppp`'s own memory among them, and
        // #492 is narrowing that. Mutation: have `Known::levels_below` add
        // nothing below the first level; the report at `return *q;` goes.
        // Only the second level is told apart: adding nothing from the third
        // on, for sites or for locals, fails no case, measured.
        a_pointer_stored_and_read_back_two_levels_down_a_parameter_as_one_place_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What that costs: a chain of this function's own allocations, read
        // as one place after an unrelated allocation was handed to a call,
        // reads what is reachable too, and `return *q;` is doubted though
        // `q` can only be `k`, live. The chain written through locals gave
        // the same doubt before #486. Mutation: add nothing below the first
        // level; this goes quiet.
        a_chain_of_this_function_s_own_allocations_read_as_one_place_is_doubted_as_through_locals: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_caller_s_pointer_freed_two_levels_down_is_doubted_through_another_parameter: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
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
        // And a pointer lost to a rebirth while its allocation was live, then
        // freed, then stored: the free is what makes it a value that may be
        // gone, and `*r` read back out of the box is a use after free that was
        // read in silence (#501). Mutation: never run
        // `Known::lost_may_be_gone`; the report at `*r` goes.
        a_lost_pointer_freed_then_stored_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // Only a lost local is marked: a pointer read out of memory may be the
        // null `free(NULL)` leaves alone, and a read of it behind `if (r)` is
        // read in no new doubt. Found by review. Mutation: treat a load as a
        // lost local, both at the trigger of `Known::lost_may_be_gone` and in
        // what it marks; this reports at `*r`.
        a_null_read_out_of_memory_freed_and_stored_is_not_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And every other spelling of a lost pointer that went, since a lost
        // value names no site and any of them may be the one freed (#503):
        // stored before the free, copied before it, freed through the box it
        // was stored in, and handed to a `realloc` that succeeded. Each reads
        // freed memory at `*r`. Mutation: do not record `Known::holds_lost` at
        // a store; the first goes quiet. Mutation: mark only the local handed
        // to `free`; the second. Mutation: no trigger for a place read through
        // memory; the third. Mutation: no trigger for `realloc`; the fourth.
        a_lost_pointer_stored_then_freed_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_copy_of_a_lost_pointer_made_before_its_free_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_freed_through_memory_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_reallocated_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // Found by review: the store on one arm of a join, which holds the
        // mark only if the join unions `Known::holds_lost`; the free spelled
        // through a load of the box, `t = *box; free(t);`, which triggers as
        // `free(*box)` does; and the box copied by `memcpy` or grown by
        // `realloc` before the free, which carry the mark as they carry
        // `stale`. Mutation: drop the union at the join; the first goes quiet.
        // Mutation: no trigger for a load; the second. Mutation: drop the mark
        // from the `memcpy` copy; the third. From the `realloc` copy; the
        // fourth.
        a_lost_pointer_stored_on_one_arm_then_freed_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_freed_through_a_load_of_its_box_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_box_of_a_lost_pointer_copied_by_memcpy_before_its_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_box_of_a_lost_pointer_reallocated_before_its_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And what a lost value read back before its free, or released by a
        // call this check cannot read, still left read in silence (#505): a
        // load out of the box made before `free(u)`; that load stored into a
        // second box; `u` handed to a call only declared, and to one defined
        // in the same file, which this check reads no better; and the box
        // itself handed to a call. Mutation: never set `Held::lost_read` at a
        // read; the first two go quiet. Mutation: do not carry it into a
        // container at a store; the second. Mutation: have
        // `Known::lost_may_be_gone` leave a `lost_read` local unmarked; the
        // first, while the second is still doubted through the second box,
        // which the free makes stale. Mutation: no trigger for an opaque call handed a lost
        // local; the third and fourth. Mutation: no trigger for one whose
        // handed memory reaches an allocation holding a lost value; the fifth.
        a_lost_pointer_read_back_before_its_free_is_doubted_when_used: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_load_of_a_lost_pointer_stored_again_before_its_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_released_by_a_call_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_released_by_a_defined_function_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_box_of_a_lost_pointer_released_by_a_call_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // Found by review: each place `Held::lost_read` is carried, and each
        // spelling of a call handed a lost value. Mutation, one per case in
        // order: drop it from a value built by arithmetic out of a load
        // (`built_from`); from the copy `memcpy` makes into a local; from the
        // join of two `Held`; from `Held::accumulated`; have the trigger for
        // an opaque call ignore a `lost_read` argument; ignore a load handed as
        // an operand, `release(*box)`; ignore a local's address, `&u`; leave
        // the allocations behind a local's address out of what it reaches.
        // Each goes quiet at the use.
        a_lost_pointer_read_back_and_moved_before_its_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_copied_into_a_local_by_memcpy_before_its_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_read_back_on_one_arm_before_its_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_load_of_a_lost_pointer_moved_by_arithmetic_before_its_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_load_of_a_lost_pointer_released_by_a_call_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_released_by_a_call_through_its_box_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_released_by_a_call_through_its_address_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_box_of_a_lost_pointer_released_through_a_local_s_address_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What the call trigger costs, and what it does not: a node read out of
        // a list built in a loop is doubted after the head is handed to a call
        // this check cannot read, which may free the node; and a call handed
        // nothing, `release_all()`, marks nothing. Found by review. Mutation:
        // ignore what a call's handed memory reaches; the first goes quiet.
        // Mutation: let every call this check cannot read trigger; the second
        // reports.
        a_node_read_out_of_a_list_is_doubted_after_its_head_is_handed_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_box_of_a_lost_pointer_is_not_doubted_after_a_call_handed_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And what a call this check cannot read returns when it was handed
        // a lost value, which may be that value: `r = id(*box);` with `id`
        // defined, and `r = id(u);` with it only declared, then `free(u);`
        // and `*r` (#508). Mutation: leave the result not lost; both go
        // quiet at `*r`.
        a_lost_pointer_returned_by_a_call_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_returned_by_an_unread_call_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And the result of such a call handed the lost value copied, `id(t)`,
        // or moved, `id(u + 1)`; each goes quiet at `*r` under the same
        // mutation. Found by review.
        a_lost_pointer_copied_and_returned_by_a_call_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_moved_and_returned_by_a_call_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What the result's mark costs: it is lost at once, as a named value
        // the call may have freed is doubted at once, so `*r` is doubted with
        // no free after the call, and a call that returns a fresh allocation,
        // ignoring the lost value it was handed, is doubted at its result's
        // use and free. Mutation: mark the result `Held::lost_read` rather
        // than lost, waiting for a later free; the first goes quiet (#511).
        a_result_of_a_call_handed_a_lost_value_is_doubted_with_no_free_after: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_fresh_allocation_returned_by_a_call_handed_a_lost_value_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And what a later such call returns, once a lost value was handed to
        // one that may have kept it: `stash(u); r = fetch(); free(u);` and
        // `*r`, on every path and with the call on one arm only (#511).
        // And with the lost value freed, or released by a call handed
        // nothing, before the later call returns it, where nothing after the
        // call frees one. Found by review. Mutation: have the result read
        // whether its own call was handed a lost value rather than
        // `Known::lost_exposed`; all six go quiet at `*r`.
        // Mutation: join `Known::lost_exposed` as an intersection; the
        // second alone does. Mutation: mark the result `Held::lost_read`
        // rather than lost; the third, fifth and sixth go quiet. Mutation:
        // clear `Known::lost_exposed` at a free; the fifth goes quiet, which
        // is what holds that the fact is never cleared. What it costs: the
        // result is doubted with no free after it, and so is a fresh
        // allocation a later call returns, since the call may have returned
        // the lost value instead.
        a_lost_pointer_handed_to_one_call_and_returned_by_another_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_handed_to_a_call_on_one_arm_and_returned_by_another_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_result_of_a_call_after_a_lost_value_was_handed_away_is_doubted_with_no_free_after: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_fresh_allocation_returned_after_a_lost_value_was_handed_away_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_handed_to_a_call_and_freed_before_another_returns_it_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_handed_to_a_call_and_released_before_another_returns_it_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And a lost value an escaped local holds, which a call this check
        // cannot read reaches as it does a named value's site: assigned to a
        // local whose address an earlier call kept, read there out of a box
        // of it, and two levels of address down, `pa = &a;` with `a` lost
        // (#515). Mutation: leave escaped locals out of `hands_a_lost_value`;
        // all three go quiet at the use. Mutation: ask an escaped local only
        // `Held::lost`, not `Held::lost_read`; the second does. Mutation:
        // ask every local, escaped or not;
        // `a_box_of_a_lost_pointer_is_not_doubted_after_a_call_handed_nothing`
        // reports.
        a_lost_pointer_stored_into_a_local_a_call_holds_the_address_of_is_doubted_when_a_later_call_returns_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_value_read_from_a_box_of_a_lost_pointer_into_an_escaped_local_is_doubted_when_a_call_returns_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_handed_through_two_levels_of_a_locals_address_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And a lost value in memory such a local points at, `w = box;` with
        // `*box = u;`, read out before a call handed nothing frees it. Found
        // by review. Mutation: leave out the sites escaped locals hold from
        // what the trigger reaches; this goes quiet at the use.
        a_lost_pointer_in_memory_an_escaped_local_points_at_is_doubted_after_a_call_reaches_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // What the trigger does besides the result: a value read out of a box
        // of the lost value before the call, `r = *box;`, is lost once a call
        // handed nothing reaches `u` through `w`, since it may free `u`.
        // Found by review. Mutation: skip `Known::lost_may_be_gone` when only
        // an escaped local reaches the lost value; the first goes quiet. And
        // what it costs, as a named value does: the result of any later such
        // call is doubted with nothing freed, `init(&p);` with `p` then lost
        // to a loop, and `b = get(); *b`.
        a_pointer_read_out_of_a_box_of_a_lost_value_is_doubted_after_a_call_reaching_it_through_a_local: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_result_of_a_call_beside_an_escaped_local_holding_a_lost_value_is_doubted_with_nothing_freed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
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
        // warning under `--allow-unknown` (#333). Nor is any size doubted:
        // `**t3 = r;` leaves `slot` where `r` starts. Mutation: leave the
        // offset `Held::accumulated` gives a local written two levels down;
        // this and five others gain an `SC0412`. Mutation: leave it for a
        // local written through one dereference;
        // `a_locals_address_read_back_out_of_an_allocation_is_written_through`
        // does.
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

    "bounds" => {
        // Whether an access fits the object its pointer reaches, `SC0412`.
        // See ADR-0060, whose Confirmation names the mutation each of these
        // fails under.
        //
        // The issue's own program: a null test on the moved pointer took away
        // the `SC0403` that had been standing in for this. Mutation: answer
        // `Offset::NonZero` as `Zero` in `fits`; this goes silent.
        a_write_through_a_pointer_moved_past_its_allocation_after_a_null_test: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Proved outside, and so an error that `--allow-unknown` leaves one.
        // Mutation: leave the allocator's argument unread in `made_extent`;
        // both turn unknown, and the second builds.
        an_int_written_into_one_byte: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_int_written_into_one_byte_is_an_error_where_unknown_is_allowed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // The same allocation read and written as wide as it is. Mutation:
        // the same; this turns unknown.
        an_int_written_into_four_bytes_is_inside: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer is eight bytes on one target and four on another, which
        // is `Target::pointer_bits`. Mutation: give `x86_64-pc-windows-msvc`
        // 32 pointer bits; the first goes silent.
        a_pointer_written_into_four_bytes_is_outside_where_a_pointer_takes_eight: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_written_into_four_bytes_fits_where_a_pointer_takes_four: ["--emit", "safety-ir", "--target", "i686-unknown-linux-gnu"],
        // Two allocations of different sizes, each exact, which proves
        // neither fit nor outside: a may-set proves nothing from one member.
        // They are two sites, so this is the fold in `fits` and not
        // `Extent::joined`, whose unit test is in `memory/parts.rs`.
        // Mutation: prove a fit when any object is wide enough rather than
        // every one; this goes silent.
        two_allocations_of_different_sizes_are_neither_proved_inside_nor_outside: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The `void *` round trip, which needs no cast: a load is believed by
        // the type it is read as, which here is `void *` and so nothing. Mutation:
        // believe every pointer read out of memory to point at eight bytes;
        // this goes silent.
        a_char_address_stored_as_void_and_read_as_int: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same round trip through locals, where the edge names `c` and
        // its size is read. Mutation: fold a local's address as
        // `Extent::AtLeast(0)`; this turns unknown.
        a_char_address_converted_through_void_and_written_as_int: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What arrives across a function's boundary is believed whole.
        // Mutation: give a parameter `Extent::AtLeast(0)` where the function
        // starts; the first is reported. Mutation: drop the belief in an
        // opaque call's result; the second is reported.
        a_parameter_written_through_is_inside: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_call_s_result_is_believed_whole: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A library call that takes a length is an access of that many
        // bytes. Mutation: drop `reached_by_library`; the first two go
        // silent, and the third is unchanged, which is why it is here.
        a_memset_wider_than_its_allocation: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_memset_of_a_length_nothing_bounds_is_unknown: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_memcpy_that_fits_both_objects_is_inside: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `calloc(m, n)` makes `m * n` bytes. Mutation: read only `m`; `p` is
        // reported as well.
        a_calloc_of_two_constants_is_their_product: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A doubt about `*p` and a proof about what `memset` writes through
        // `p`, at one call, each said, so that the proof stays an error
        // where unknown is allowed. Mutation: key `reached_by_library` with
        // the dereferences' list in `findings_within`; the proof goes and
        // this exits 0.
        a_proof_at_a_call_is_not_displaced_by_a_doubt_about_its_argument: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // A pointer moved off a null or off a local's address holds no site,
        // and is not therefore null: a null test on it takes the nullability
        // check's doubt away, and only this is left to ask. The second is the
        // shape every index past a local array's start has (#621). Mutation: leave
        // the offset out of `holds_something` in `fits`; both go silent.
        a_pointer_moved_off_null_and_tested_is_not_proved_inside: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_s_address_moved_past_it_and_tested_is_not_proved_inside: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A constant handed to a call is what the parameter receives, as the
        // backend passes it: 257 to a `char` is one, and 260 is four. Both
        // declarations are wrong for the library, which only a frontend that
        // reads `size_t` can refuse. Mutation: read the constant as written
        // in `constant_argument`; the first goes silent and the second is
        // proved outside.
        an_allocation_s_size_is_what_its_parameter_receives: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_length_is_what_its_parameter_receives: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Each row of `LIBRARY` that reaches by a length or a string, read.
        // Mutation: drop a row's `reaches`, or its row, and its case loses
        // the report: `memmove`, `memcmp` at both arguments, `strlen`.
        a_memmove_wider_than_its_destination: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_memcmp_longer_than_both_objects: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_strlen_is_not_shown_to_fit: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `strncpy` writes exactly `n` characters, so its destination is an
        // access of `n` and fits here; its source is read as a string, which
        // is not shown to fit. Mutation: read its destination as a string;
        // the destination is reported too.
        a_strncpy_of_its_destination_s_size_is_unknown_only_at_its_source: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `aligned_alloc`'s size is its second argument. Mutation: make it
        // the first; this goes silent, sixteen bytes read for one.
        an_int_written_into_one_aligned_byte: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A load is believed no wider than what its memory is recorded to
        // hold: a `char`'s address put there through a `void **` alias, or
        // by `memcpy`, and read back as an `int *`. No store either asks is
        // a departure of the type that is read. Mutation: answer a belief
        // without the record in `fits`; both go silent.
        a_char_address_stored_as_void_and_read_through_an_int_alias: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `void *` arrives believing nothing, so even one byte through it
        // is not shown to fit. Mutation: believe a `void *` one byte in
        // `pointee_size`; this goes silent.
        a_void_pointer_parameter_is_believed_to_point_at_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two loads meet as the narrower belief, here a `void *`'s nothing.
        // Mutation: keep the wider in `Held::joined`; this goes silent.
        a_belief_two_paths_meet_with_is_the_narrower: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A negative size is no size read. Mutation: read one as the
        // largest `u64` in `constant_argument`; this goes silent.
        an_allocation_of_a_negative_size_is_no_size_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A proof over two sizes names neither, and one byte is one byte.
        // Mutation: name the largest of the sizes in `fits`, or answer
        // `bytes` for every count in `words.rs`; each changes its message.
        an_access_wider_than_two_allocations_of_different_sizes_names_neither: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_byte_written_into_nothing_is_one_byte: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read out of memory and handed straight to `memset` is
        // believed by the type it is read as: four bytes fit, a hundred are
        // not shown to. Mutation: answer every such argument inside in
        // `reached_by_library`; the second goes silent.
        a_pointer_read_out_of_memory_and_handed_to_memset_is_believed_by_its_type: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer that leaves is asked whether it is whole, `SC0413`, at
        // each of the three doors where the other side believes it is.
        // Mutation: drop `departed_at_call`, or `departed_in_element`'s
        // `Return` or `Store` door; the first, second or third goes silent.
        a_pointer_moved_past_its_allocation_handed_to_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_moved_off_its_start_returned: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_moved_off_its_start_stored_into_memory: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same three doors, each handed a whole allocation. Mutation:
        // answer every departure unknown in `say_leaving`; this is reported
        // three times.
        a_whole_pointer_handed_on_is_silent: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `char` handed to an `int *` parameter through a `void *` is
        // proved narrower than the parameter believes. Mutation: take the
        // width from the argument's own type in `departed_at_call`; this goes
        // silent, since a `void *` asks no width.
        a_char_handed_to_an_int_parameter_is_shown_narrower: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Past every parameter, what the callee receives is the argument's
        // own type. Mutation: skip an argument with no parameter; this goes
        // silent.
        a_pointer_passed_past_every_parameter_is_asked_as_its_own_type: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `void *` leaves believing no width, and is asked whether it
        // starts an object at every door. Mutation, for the four that report:
        // answer `None` for `void` in `pointee_width`; each goes silent.
        //
        // At a call. Mutation: skip a width of zero in `departed_at_call`.
        a_void_pointer_off_its_start_leaving_is_asked_whether_it_starts_an_object: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // At a store in this function, and at one in a callee. Mutation: skip
        // a width of zero in `departed_in_element`; both go silent.
        a_pointer_off_its_start_stored_as_void_and_read_through_an_int_alias: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_off_its_start_stored_as_void_by_a_callee: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // In a local whose address leaves, named in the words a named pointer
        // gets. Mutation: skip a width of zero in `contents_leaving`; this
        // goes silent.
        a_void_pointer_in_a_local_whose_address_leaves_is_asked_its_start: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `void *` parameter handed on unchanged starts what it starts, and
        // nothing is known of its size: asked its start it is silent, asked
        // any width it would not be. Mutation: give `void` a width of one in
        // `pointee_width`; this is reported.
        a_void_pointer_of_no_known_size_leaves_asking_no_width: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An access that goes through a local's address asks what that local
        // holds at the access, whichever came first, the move or the address,
        // and at every level. Mutation: ask only the first level in
        // `may_not_fit`; the first four go silent, and three of the cases
        // below change. Mutation: stop after the second level; the fourth
        // does, and so does the one through an allocation below.
        a_pointer_moved_after_its_local_s_address_was_taken_is_asked_through_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_moved_before_its_local_s_address_was_taken_is_asked_through_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_void_pointer_moved_and_read_through_an_int_alias_of_its_local_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_three_levels_through_locals_is_asked_at_the_last: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A whole pointer, and one moved and moved back before the access,
        // say nothing of size: they keep the `SC0402` and `SC0403` they had
        // before, and gain no `SC0412`.
        // Mutation: doubt the escape of a local reached through an address in
        // `may_not_fit`; both gain an `SC0412`, and fifteen other cases
        // change.
        a_whole_pointer_read_through_its_local_s_address_says_nothing_of_size: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_moved_and_moved_back_before_it_is_read_through_its_local: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local's address stored in an allocation is found as one held by
        // a local is. Mutation: walk only the address edges of the level
        // above in `read_from_locals`, not `Known::levels_below`; this goes
        // silent.
        a_pointer_moved_after_its_local_s_address_was_stored_in_an_allocation_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read through a local's address and handed on, to a
        // function or to `memset`, asks what the local holds, as one gone
        // through does. Mutation: answer `None` from `through_locals`; both
        // go silent.
        a_pointer_read_through_its_local_s_address_and_handed_on_is_asked_what_the_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_through_its_local_s_address_and_handed_to_memset_is_asked_what_the_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `v` is asked twice at one caret, doubted through itself and proved
        // through `pi`, and the proof stands. Mutation: keep the first
        // answer in `say_fit`; this says a doubt instead.
        a_proof_through_a_local_s_address_stands_beside_a_doubt_about_the_local_itself: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `pi` is `&v` or `&u`, so `u` being one `char` is a doubt and not a
        // proof. Mutation: return `fit` unchanged from `only_if_certain`;
        // this says a proof. Mutation: keep only the first local in
        // `read_from_locals`; this goes silent.
        a_pointer_that_may_be_either_of_two_locals_proves_nothing_from_one: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Each local `pi` may be is asked, here the first, as the case above
        // asks the last. Mutation: keep only the last local in
        // `read_from_locals`; this goes silent.
        a_pointer_that_may_be_either_of_two_locals_is_asked_of_each: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read out of a local is believed only where the local's
        // value fits what it is read as: a `void *` a callee filled with a
        // `char`'s address, read through an `int **` alias, and a pointer
        // moved off its start and read out before it is used. Mutation: skip
        // the local check in `read_through`; both go silent.
        an_int_pointer_read_through_an_alias_of_a_void_pointer_a_callee_filled_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_its_local_before_it_is_used_is_asked_what_the_local_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A `char`'s address read back as a `char *` fits by size, so it is
        // believed. The `char **` alias of a `void *` local is itself C17 6.5
        // p7's, which this check does not follow; the conforming spelling,
        // read as a `void *` and converted, is #631. The null doubt is the
        // nullability check's. Mutation: leave every load
        // out of a local unbelieved in `read_through`; this gains an
        // `SC0412`.
        a_char_stored_as_void_in_a_local_and_read_back_as_char_says_nothing_of_size: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a pointer leaving lets leave is asked at the width the
        // receiver believes: a local at the wider of its own type and the
        // receiver's, and memory by what its record says it holds.
        // Mutation: ask each local at its own type only in
        // `contents_leaving`; the first goes silent. Mutation: walk no sites
        // there; the second does.
        a_void_pointer_s_address_handed_as_an_int_pointer_to_pointer_is_asked_at_the_wider_type: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        memory_holding_a_char_s_address_handed_as_an_int_pointer_to_pointer_is_asked_what_it_holds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Leaving as a `void *`, the receiver believes no width, and the
        // local is still asked at its own: whoever converts it back reads an
        // `int *`. Mutation: ask each local at the receiver's width only in
        // `contents_leaving`; this goes silent.
        a_local_whose_address_leaves_as_void_is_asked_at_its_own_type: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A whole `int` stored as a `void *` fits what the receiver reads,
        // and a parameter's memory holds nothing this function recorded.
        // Mutation: answer a site's content `Unknown` in
        // `contents_leaving`; the first gains an `SC0413`. Mutation: answer
        // memory whose record holds nothing `Unknown` there; the second
        // does.
        memory_holding_a_whole_int_s_address_stored_as_void_and_handed_as_int_says_nothing_of_size: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_parameter_s_memory_handed_on_unchanged_says_nothing_of_size: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two objects in memory too narrow for the receiver are one report
        // at the call, not two printed alike. Mutation: say a value with no
        // local each time in `say_leaving`; this prints it twice.
        two_objects_in_memory_both_too_narrow_are_one_report: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // One local asked at two types at one call is said narrower, in
        // whichever order the walk reaches them. Mutation: keep the first
        // answer for a local in `say_leaving`; both say a doubt instead.
        a_local_shown_narrower_is_said_so_when_reached_through_another_local_first: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_shown_narrower_is_said_so_when_reached_directly_first: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read out of memory and handed straight on lets what
        // that memory holds leave, as one assigned to a local first does.
        // Mutation: seed no sites for a dereferenced argument in
        // `contents_leaving`; this loses its `SC0413`.
        a_pointer_read_out_of_memory_and_handed_on_lets_what_that_memory_holds_leave: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Memory a pointer arrived pointing at holds pointers believed as the
        // type it arrived as, and a conversion keeps that: a `void **`
        // parameter's or result's `void *`s were asked no width. Mutation:
        // set no `contents_at_most` for a parameter; the first two go
        // silent, and so do the three readers below, which go through one
        // too. Mutation: set none for a call's result; the next two do.
        // Mutation: set none for a load; the fifth does.
        a_void_pointer_to_pointer_parameter_read_through_an_int_alias_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_void_pointer_to_pointer_parameter_handed_on_as_an_int_alias_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        memory_a_call_returned_as_void_read_through_an_int_alias_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        memory_a_call_returned_as_void_handed_on_as_an_int_alias_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        memory_two_levels_below_a_void_parameter_read_through_an_int_alias_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local a callee was handed is believed written no wider than it
        // was handed as, and as nothing through a `void *`. Mutation: ignore
        // `handed_at_most` where a call writes an escaped local; both go
        // silent. Mutation: give a local reached as `void` the local's own
        // width; the second does.
        a_local_a_callee_was_handed_as_void_is_believed_no_wider_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_handed_to_a_void_pointer_parameter_believes_nothing_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A load stored where the record names nothing bounds that memory
        // by what it was believed. Mutation: record nothing in
        // `stored_at_most`; this goes silent.
        a_load_stored_in_memory_is_believed_no_wider_than_it_was_read_when_handed_on: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Every reader of a load asks the same bound: one handed to a
        // function, one written through two levels, and one handed to
        // `memset`. Mutation: read the type alone in `leaving`; the first
        // goes silent. Mutation: skip the memory a deeper level goes through
        // in `may_not_fit`; the second does. Mutation: read the type alone
        // in `reached_by_library`; the third does.
        a_pointer_read_through_a_void_parameter_and_handed_on_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_write_two_levels_through_a_void_parameter_is_asked_of_the_memory: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_through_a_void_parameter_and_handed_to_memset_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A round trip through `void *` back to the type a pointer arrived
        // as, and a local a callee filled through the type it is, believe
        // what they are. Mutation: set `Some(0)` for every arrival in
        // `type_levels`; the first gains an `SC0412`, beside about a
        // hundred other cases. Mutation: give every local a call reaches as
        // a pointer a width of zero; the second does.
        an_int_pointer_to_pointer_round_trip_through_void_says_nothing_of_size: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_a_callee_filled_through_an_int_pointer_to_pointer_is_believed_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A copy carries the bound on what it copies, as a store of a load
        // would: out of a `void **` parameter, out of memory a load named
        // nothing in, and grown by `realloc`. Mutation: record no bound where
        // `memcpy` copies into memory; the first two go silent. Mutation:
        // carry none into what `realloc` returns; the third does.
        a_pointer_copied_out_of_a_void_parameter_by_memcpy_is_believed_as_it_arrived: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_load_stored_and_copied_by_memcpy_is_believed_no_wider_than_it_was_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_load_stored_and_grown_by_realloc_is_believed_no_wider_than_it_was_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A call made on one arm leaves the local bounded by what it was
        // handed as after the join, where the belief it left is gone.
        // Mutation: drop the `handed_at_most` question from `fits`; this
        // goes silent.
        a_local_a_callee_may_have_written_on_one_arm_is_believed_no_wider_than_it_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What memory holds on one arm bounds it after the join, beside a
        // whole object or a null on the other. Mutation: let a recorded
        // whole object stand over a narrower stored bound in
        // `contents_leaving`; the first goes silent. Mutation: join
        // `stored_at_most` by nothing; both do.
        memory_holding_a_whole_object_on_one_arm_and_a_load_on_the_other_is_asked_as_the_narrower: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        memory_holding_a_load_on_one_arm_and_a_null_on_the_other_is_asked_as_the_load: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A load stored in memory is bounded by what it was believed,
        // whatever sites the record says it names, which may be another
        // allocation or the caller's memory: stored over a whole allocation,
        // through a pointer read out of memory, and two levels below a
        // parameter. Mutation: bound only a load that names no site and no
        // local; all three go silent.
        a_load_stored_over_a_whole_allocation_is_still_bounded_by_its_belief: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_load_stored_through_a_pointer_read_out_of_memory_is_bounded_by_its_belief: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_load_two_levels_below_a_parameter_stored_in_memory_is_bounded_by_its_belief: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A read believes the narrower of the type it reads and what its
        // memory holds. Mutation: take the memory's bound alone in
        // `believed_read`; this goes silent.
        a_pointer_read_narrower_than_its_memory_holds_is_believed_as_it_was_read: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A load that believes nothing on some path is bounded at nothing
        // when stored. Mutation: record no bound for it; this goes silent.
        a_load_on_one_arm_and_a_null_on_the_other_stored_in_memory_is_bounded_at_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A bound on one arm survives the join, in either order. Mutation:
        // drop `contents_at_most` from `Held::joined`; the first goes silent.
        // Mutation: take the other side's alone; the second does.
        a_converted_void_parameter_on_the_first_arm_bounds_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_converted_void_parameter_on_the_second_arm_bounds_the_join: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A stored bound is asked by a read as well as a departure.
        // Mutation: skip `stored_at_most` in `believed_read`; this goes
        // silent.
        a_load_stored_in_memory_and_read_back_through_an_int_alias_is_bounded: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A later store does not raise a stored bound, since one site holds
        // every slot of an allocation and the later store may be into
        // another. Mutation: assign the bound at a store; this goes silent.
        a_stored_bound_is_not_raised_by_a_later_store_since_slots_are_not_told_apart: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read out of a parameter's memory and handed straight on
        // arrives as the type it is read as. Mutation: bound nothing for such
        // a departure in `contents_leaving`; this loses its `SC0413`.
        a_pointer_read_out_of_a_void_parameter_and_handed_straight_on_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Arithmetic keeps the bound of what it is built from. Mutation: drop
        // `contents_at_most` from `Held::accumulated`; this loses a report.
        a_converted_void_parameter_moved_by_arithmetic_keeps_its_bound: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Memory a pointer to no pointer arrived at holds nothing vouched for
        // as a pointer. Mutation: bound it at no limit in `type_levels`;
        // both lose a report. Mutation: bound it at one byte; the second
        // does.
        an_int_parameter_read_through_an_int_pointer_to_pointer_believes_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_int_parameter_read_through_a_char_pointer_to_pointer_believes_nothing: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What memory holds is believed at every level of the type it
        // arrived as, through a conversion, a store and a reload, a copy, a
        // join and a local's address (#634). Each case says the mutation
        // that makes it fail.
        //
        // Mutation: `read_through` takes the read type alone.
        a_void_pointer_three_levels_deep_converted_whole_is_believed_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `believed_read` takes no bound from memory below one
        // dereference.
        a_void_pointer_three_levels_deep_read_two_levels_at_once_is_believed_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `contents_leaving` compares the departing value at level
        // 0 alone.
        a_void_pointer_three_levels_deep_handed_on_whole_is_asked_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a store records level 0 alone.
        a_parameter_stored_in_memory_and_read_back_is_believed_as_it_arrived_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a store records level 0 alone. Mutation:
        // `Known::memory_levels` folds only the sites at the last level.
        a_parameter_stored_in_memory_and_read_two_levels_at_once_is_believed_as_it_arrived: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a store records level 0 alone. Mutation:
        // `contents_leaving` compares a site's stored bound at level 0 alone.
        a_parameter_stored_in_memory_and_handed_on_is_asked_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a store records the levels below only for a pointer that
        // is not a load.
        a_load_stored_in_memory_and_read_back_is_believed_as_it_was_read_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the `memcpy` copy records level 0 alone.
        a_void_pointer_three_levels_deep_copied_by_memcpy_is_believed_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the `memcpy` copy takes level 0 alone of what
        // `Known::memory_levels` finds in the source's memory.
        a_parameter_stored_and_copied_by_memcpy_is_believed_as_it_arrived_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `Levels::bound_by` lets a side with no bound at a level
        // loosen the other.
        a_converted_void_pointer_three_levels_deep_on_one_arm_bounds_the_join_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `Known`'s join takes level 0 alone of the other side's
        // `stored_at_most`.
        memory_holding_a_void_pointer_on_one_arm_and_an_int_pointer_on_the_other_is_believed_as_the_narrower_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `type_levels` answers `Some(0)` at every level, and this
        // is reported.
        an_int_pointer_three_levels_deep_round_trip_through_void_says_nothing_of_size: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the same, and this is reported.
        an_int_pointer_to_pointer_parameter_stored_in_memory_and_read_back_says_nothing_of_size: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: a call's result takes level 0 of its type alone.
        a_call_returning_a_void_pointer_three_levels_deep_is_believed_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `contents_leaving` takes the type a load is read as
        // alone.
        a_pointer_read_out_of_a_converted_void_parameter_and_handed_straight_on_is_asked_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `realloc` carries level 0 alone of the pointer it is
        // handed.
        a_void_parameter_three_levels_deep_grown_by_realloc_is_believed_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `realloc` carries level 0 alone of what the old
        // allocation held.
        a_parameter_stored_in_memory_grown_by_realloc_is_believed_as_it_arrived_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local reached through its address is memory holding its value,
        // which is asked below its own level by what that value believes.
        //
        // Mutation, for each of the next four: `contents_leaving` never finds
        // a local's value narrower.
        a_void_pointer_to_pointer_parameter_handed_by_address_is_asked_below_its_own_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_holding_a_converted_void_pointer_to_pointer_handed_by_address_is_asked_below_its_own_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_holding_a_converted_void_pointer_three_levels_deep_handed_by_address_is_asked_at_every_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_s_address_stored_in_memory_is_asked_by_what_the_local_believes_where_it_leaves: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `Known::memory_levels` skips the locals on the way.
        a_load_read_back_through_a_local_s_address_is_believed_as_it_was_read_below_its_first_level: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What the memory check concludes about an array and an index (#621):
        // the first element of a local array is read by the array's size, and
        // every access off a pointer's start is `Unknown`, as ADR-0060 says.
        //
        // Mutation: `size_of` answers `None` for an array; this is reported.
        // Mutation: set `inner_address` for a constant zero index; so is
        // this.
        an_access_at_the_start_of_a_local_array_says_nothing_of_size: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_access_past_the_start_of_a_local_array_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_index_past_the_start_of_an_allocation_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_element_of_an_array_of_arrays_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer stored into a row is reported where it is stored and read
        // back, and named as freed at `*q` (#639). Mutation: the edge is
        // dropped for every local again; this loses the `SC0402`.
        a_pointer_stored_into_a_row_and_freed_is_doubted_when_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `type_levels` stops at an array; this is reported.
        a_pointer_read_out_of_an_array_of_pointers_behind_a_parameter_is_believed_as_its_element: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: the width a local was handed as skips an array of
        // pointers rather than reading its element; this goes silent.
        a_pointer_a_callee_may_have_written_into_an_array_of_pointers_is_believed_no_wider_than_it_was_handed: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `size_of` wraps an array too large to count rather than
        // answering `None`; this goes silent.
        an_array_too_large_to_count_is_not_proved_inside: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `innermost_element` strips one dimension only; this panics at
        // `type_levels`.
        a_pointer_read_out_of_an_array_of_arrays_of_pointers_behind_a_parameter_is_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A pointer read out of memory and handed straight on is believed by
        // the type it is read as, here a `void *`, which is narrower than the
        // `int` the parameter believes. Mutation: answer such an argument
        // inside in `leaving`; this goes silent.
        a_pointer_read_as_void_and_handed_to_an_int_parameter_is_not_proved: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Handing on a pointer to memory hands on the pointers stored there,
        // asked at the type they were stored as, and at the receiver's where
        // that is wider: the third is also reported at `h(box)`, which reads
        // the copied pointer as an `int *`. Mutation: drop
        // `contents_leaving` from `departed_at_call`; the first two go
        // silent. Mutation: drop it from the `memcpy` arm; the third loses
        // its first report.
        a_pointer_in_a_local_whose_address_leaves_is_asked_at_its_own_type: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_assigned_after_its_local_s_address_was_taken_is_asked_where_the_address_leaves: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_into_memory_by_memcpy_is_asked_as_a_store: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A body is what runs, whatever it is called. Mutation: ask a call by
        // its library row even where the callee is defined here; this goes
        // silent.
        a_function_defined_with_a_library_name_is_asked_as_any_function: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A callee's write into an escaped local is believed only where the
        // local was whole before it. Mutation: believe it whatever the local
        // held, in the opaque call's write in `transfer.rs`; the `SC0413` at
        // `use(a)` goes. Mutation: never believe it; the corpus cases that
        // grow a pointer by address twice, `a_pointer_handed_by_address_twice_builds`
        // among them, are reported.
        a_pointer_moved_after_its_address_was_kept_is_not_believed_after_a_call: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A departure shown narrower says so, at a return and at a store too,
        // with where the allocation was made, and names no size where the
        // sizes differ; it stays unknown. Mutation: drop `narrower` in
        // `say_leaving`, or `made`; the first changes its words or loses its
        // label. Mutation: name a size where they differ; the second changes.
        a_char_returned_and_stored_as_an_int_pointer_is_shown_narrower: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_to_two_narrower_allocations_leaves_naming_neither: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `free` reads nothing it is handed as an object, and a library
        // function is asked how far it reaches instead. Mutation: ask `free`'s
        // argument in `departed_at_call`, or drop the skip of an argument the
        // library row reaches; each gains an `SC0413`.
        a_free_is_not_asked_as_a_departure: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_library_reach_is_not_asked_again_as_a_departure: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_to_a_char_copied_into_memory_by_memcpy_and_read_back: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // `returned`; this gains an `SC0406` about `tab`. Nothing wrote `tab`
        // before it was freed, so the read is `SC0411` as well: a free leaves
        // an allocation fresh (ADR-0057).
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
        // C sequences `p[0]` before `g`, and the `?:` under `=` says so: the
        // place `x` cannot act, so the operator keeps its marker and the read
        // is not pending at `g` (#178). Mutation: drop the root at every node
        // that does not sequence, as before; this is refused again. See
        // ADR-0022 and ADR-0042.
        a_read_sequenced_before_an_opaque_call_under_an_assignment_is_not_asked: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // only for a call the root reaches, and these are calls below a node
        // whose other operand acts. The first has no free in it at all. Mutation: drop
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
        // What a pointer handed to a call points at, at every level, is asked
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
        // What only the address reaches is exempt, and nothing more: `*h` is
        // `&slot` or the box `a`, and `s` is held by `slot` and stored in `a`,
        // so it is reached through this function's own memory too and is
        // asked. Mutation: have `handed_below` read nothing through memory;
        // this builds. See ADR-0042.
        a_pointer_reaching_one_allocation_through_memory_and_an_address_is_asked_through_memory: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // `Held::certain_target` and the skip before it. The first two print
        // the same report with `writes_elsewhere` ignored or with the first
        // of several edges answered alone, measured. Those two parts are
        // guarded by `Held::certain_target`'s unit test, whose doc says why
        // no program tells the second apart. Mutation: let a place of
        // dereferences hand its own edges; the third loses its second report.
        // See ADR-0047.
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
        // What a pointer parameter points at, and what a call this check
        // cannot read returned, may hold a pointer the other side kept, so a
        // free of it or a later such call leaves what is read out of them
        // afterwards one this check stopped following. Read again after the
        // free, it was a fresh value and believed (#482). Mutation: leave the
        // parameter's site unmarked in `on_entry`; the first and third lose
        // the report at the use. Mutation: leave the result's site unmarked
        // in the `Callee::Opaque` arm; the second does. See ADR-0040.
        a_pointer_read_again_out_of_a_parameter_after_a_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_again_out_of_what_a_call_returned_after_a_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_out_of_a_parameter_after_a_call_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Every exposed parameter is marked, not the first, and a mark made on
        // one arm survives the join. Mutation: mark only the first of
        // `exposed_parameters`; the first of these goes silent at the use.
        // Mutation: join `Known::from_caller` as an intersection; the second
        // does, since each call's result is marked on its own arm only.
        a_pointer_read_again_out_of_a_second_parameter_after_a_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_read_again_out_of_either_of_two_calls_results_after_a_free_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Two pointer parameters may point at the same memory, so a store
        // into one's memory lands where a store this check cannot place
        // would, and a load through memory it cannot place reads what is
        // reachable, not only what is marked. Each was silent at a use after
        // free under `f(&x, &x)` (#489). See ADR-0044 and ADR-0045. Mutation:
        // drop the handed-over container from the store's condition; the
        // first goes silent. Mutation: have `exposed_through` read the exposed
        // sites only; the second does. Mutation: drop it from the copy's
        // condition; the third does.
        a_pointer_stored_through_one_parameter_may_be_read_through_another: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What a call this check cannot read may store of what it was handed:
        // through an out-parameter, `get(&r, u)`, and into memory it was
        // handed, `put(box, u)`, each then read after `free(u)`, for a named
        // allocation and for a lost one (#510). Mutation: have the escaped
        // locals hold no new site after the call; the first goes quiet.
        // Mutation: leave them without `Held::lost_read` when a lost value was
        // handed; the second. Mutation: have the handed memory contain nothing
        // new; the third and fourth. Mutation: hold everything reachable rather than what was
        // handed; the fifth, the output-parameter idiom beside an exposed
        // allocation, which ADR-0017 declines to report, reports at `*p`.
        // Mutation: leave out what a local handed by address holds; the sixth,
        // `get(&r, &w)` with `w = u`, goes quiet; deeper ones are below. The
        // seventh pins the cost: an end pointer set by a call handed a string
        // is doubted, as the string already is. Two choices no case holds,
        // measured: holding only the first site of what was handed, and
        // setting `Held::lost_read` whether or not a lost value was handed.
        // Every site the call was handed is already doubted after it, so a
        // local that gained one is doubted either way, and one that gained
        // none is the shape of #511.
        a_pointer_written_through_an_out_parameter_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_written_through_an_out_parameter_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_stored_into_memory_a_call_was_handed_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_lost_pointer_stored_into_memory_a_call_was_handed_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        an_output_parameter_beside_an_exposed_allocation_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_handed_by_the_address_of_its_holder_is_doubted_through_an_out_parameter_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        // And two or three levels of address down, `pa = &a; copy2(&r,
        // &pa);` (#513). Mutation: follow `writes_to` one level only; both go
        // quiet at the use. What only a deeper level hands is kept out of a
        // local that holds a local's address, and what one level hands is
        // not: the third, `k = 0; r = &k; get(&r, u);`, was read in silence
        // when it was, found by review. Mutation: keep every stored site out
        // of such a local; the third goes quiet. Mutation: keep none out;
        // the fourth, `two(&pa, &pb); *pa = q;`, and the fifth, three levels
        // of address, report the write, as do
        // `a_write_through_a_pointer_whose_own_address_escaped` and
        // `a_write_through_a_pointer_whose_address_escaped_before_it_was_given_its_target`,
        // the output-parameter idiom.
        a_pointer_handed_through_two_levels_of_a_locals_address_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_handed_through_three_levels_of_a_locals_address_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_written_through_an_out_parameter_that_held_a_locals_address_is_doubted_after_its_free: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_write_through_one_of_two_chains_of_addresses_handed_to_a_call_is_not_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_write_through_a_pointer_whose_address_escaped_two_levels_up_does_not_doubt_that_pointer: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        an_end_pointer_set_by_a_call_handed_an_allocation_is_doubted: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "--allow-unknown"],
        a_pointer_stored_through_a_parameter_may_be_read_through_a_load_of_another: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_pointer_copied_through_one_parameter_by_memcpy_may_be_read_through_another: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Memory a parameter can name is memory reachable to code this check
        // cannot read, not memory that holds a caller's pointer: a buffer
        // this function allocated and stored `*pp` in is no parameter's, and
        // asking `Known::from_caller` landed `m` stored there in every
        // parameter's memory and reported a use at `*q` that cannot happen.
        // Found by review. Mutation: ask `from_caller` of the containers of
        // the store; the first reports. Of the copy `memcpy` makes; the
        // second does.
        a_pointer_stored_in_this_function_s_own_buffer_lands_in_no_parameter: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_copy_into_this_function_s_own_buffer_lands_in_no_parameter: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A local's address lands too, in what the memory may contain rather
        // than in what is exposed: `*ppp = &x;` then a write through `*r`
        // left `x` the null it started as, and the use behind `if (!q)` was
        // never asked. Found by review. Mutation: land the sites only; all
        // three go silent at the use. Mutation: have a load through memory
        // this check cannot place reach no local; the third does.
        a_local_s_address_stored_through_one_parameter_may_be_read_through_another: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_s_address_copied_through_one_parameter_by_memcpy_may_be_read_through_another: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_local_s_address_stored_through_a_load_may_be_read_through_another_load: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And below the first level of a chain written as one place, the
        // locals whose address that memory may hold: `t = **rrrr;` may be
        // `&x` that `*ppp = &x;` landed, so `*t = m;` writes `x` (#486).
        // Mutation: extend the locals at the first level only; the use at
        // `return *q;` goes silent.
        a_local_s_address_read_out_two_levels_down_another_parameter_is_written_through: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And what that costs, which was nothing measured: five idioms with
        // two pointer parameters answer as they did before the store and the
        // load were told the two may alias. A record of that, not a guard: no
        // mutation of this rule moves them, since landing a site changes
        // none of their verdicts. The two after them are the guards on what
        // the rule must not reach.
        two_pointer_parameters_swapped_are_answered_as_before: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_output_parameter_beside_an_input_is_answered_as_before: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_slot_freed_and_nulled_beside_another_parameter_is_answered_as_before: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_list_appended_through_a_head_and_a_tail_is_answered_as_before: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_copy_between_two_slots_is_answered_as_before: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A copy between this function's own allocations lands in no
        // parameter's memory, and neither does an integer stored through
        // one. Mutation: land every copy; the first reports a use at `*q`
        // that cannot happen. Mutation: let a store into handed-over memory
        // land whatever its type; the second does.
        a_copy_between_this_function_s_own_allocations_lands_in_no_parameter: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_integer_stored_through_a_parameter_lands_in_no_other: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // A parameter the IR cannot hold. A pointer to a function, because an
        // array or a function parameter is a pointer by C17 6.7.6.3 p7 and p8
        // and the IR holds that, and a pointer to an array is held since #621.
        // Chosen for what the IR cannot hold, and stops testing this the day
        // it can.
        a_function_the_ir_cannot_hold: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // An array source is assigned as the pointer it converts to (C17
        // 6.3.2.1 p3 and 6.5.16.1 p1), and two arrays of constant lengths are
        // compatible only where the lengths are equal (6.7.6.2 p6) (#621).
        //
        // Mutation: `assignable` answers nothing for an array source; the
        // first three build.
        an_array_initializing_a_pointer_to_another_type_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_array_converted_to_an_integer_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_array_passed_where_a_pointer_to_another_type_is_wanted_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `assignable` answers `false` for every array source;
        // this is refused.
        an_array_initializing_a_pointer_to_its_element_or_to_void_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `lengths_agree` answers `true`; this builds.
        a_pointer_to_an_array_of_another_length_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: `lengths_agree` asks the lengths to differ; this is
        // refused.
        a_pointer_to_an_array_of_the_same_length_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // `SC0310`. An array of unknown size or of a constant length may have
        // one under p3, but p16 asks for a brace-enclosed list, so `u`, `s`,
        // `bu` and `bs`, each initialized by `1`, are `SC0310` too, citing p16.
        // Mutation: stop asking whether the declared type is a function, or
        // `void`; `f` and `bf`, or `v` and `bv`, go. Mutation: stop asking
        // whether an array's length has a value; `va` takes the p16 wording,
        // and asking only the outermost one does that to `vb`, whose inner
        // length makes it a variable length array too (6.7.6.2 p4); `vp`
        // points at one and is not one, so it may be initialized. Mutation:
        // pass over an array that is not variable length again; the four
        // p16 reports go. Mutation: walk only the items, or only the statements; the
        // block's reports, or the file's, go. Mutation: ask only a
        // declaration's first declarator; `second` and `bsecond` go. `fv` is
        // a variable length array at file scope, so 6.7.6.2 p2 refuses its
        // declarator under `SC0309` as well, and both are reported; dropping
        // the file-scope walk's arrays loses its `SC0310`. `v` and `bv` are
        // `void` objects, which `check_complete_objects` leaves to this code
        // when they are initialized. Mutation: report an initialized `void`
        // there too; each gains a second report, `SC0315`.
        only_an_object_can_be_initialized: ["--emit", "ast"],
        // `void` is incomplete and can never be completed (C17 6.2.5 p19), so an
        // object of it is `SC0315` at file scope (6.9.2 p2) and in a block
        // (6.7 p7), and so is a parameter of a definition (6.7.6.3 p4),
        // named or not and wherever in the list. A declaration that is not a
        // definition may have one (p12), and `(void)` is the empty prototype
        // (p10), so `d`, `e` and `k` are not reported. The unnamed `f` and `n`
        // are also `SC0309`, since a definition's parameters have names
        // (6.9.1 p5), which `a_parameter_of_a_definition_has_a_name` holds. `n` is the struct
        // counterpart of the unnamed `void`: any unnamed parameter is "a
        // parameter" in the message, since its span is its specifier.
        // Mutation: drop the `Type::Void` arm in `check_complete_objects`;
        // every report but `n`'s goes. Mutation: name an unnamed parameter by
        // its span; `f`'s report says `void` has type `void`, and `n`'s names
        // `struct T`. Mutation: ask a declaration's parameters too; `d` and
        // `e` gain reports.
        an_object_or_a_definition_s_parameter_of_type_void_is_incomplete: ["--emit", "ast"],
        // C17 6.9.1 p5, a constraint: each parameter of a definition is named,
        // a lone `void` aside, so `f`, `g`, `h` and `k` are `SC0309` once per
        // unnamed parameter, `k` beside its `void`'s `SC0315`. Not reported:
        // `d`, a declaration; `e`, the empty prototype; and `p`, whose
        // parameter is named and whose parameter's own parameter need not be.
        // `clang` only warns without `-pedantic-errors`, calling it C23.
        // Mutation: drop `check_definition_parameters` from `check`; every
        // `SC0309` goes. Mutation: ask a declaration's parameters too; `d`
        // gains one. Mutation: walk parameters' own function types; `p`
        // gains one.
        a_parameter_of_a_definition_has_a_name: ["--emit", "ast"],
        // An array of unknown size is incomplete (C17 6.2.5 p22), so in a block
        // with no initializer it is `SC0315` (6.7 p7): `a`, `b` and `p`. What
        // is not `SC0315`: `t`, a tentative definition 6.9.2 p2 completes;
        // `q`, a parameter adjusted to a pointer (6.7.6.3 p7); and `bu`, whose
        // fault is its initializer (6.7.9 p16), not its type, and is reported
        // as that, `SC0310`. Asked of the
        // type check alone, which is what used to let these through. Mutation:
        // drop the `Type::Array` arm in `check_complete_objects`; the three
        // reports go. Mutation: drop `at_file_scope` from it; `t` gains one.
        // Mutation: drop `initialized`; `bu` gains one, and so does
        // `only_an_object_can_be_initialized`. `c`, `v` and `x` have an
        // element 6.7.6.2 p1 forbids, which is their `SC0309`, and get no
        // second report. Mutation: drop `forbidden_element` from the arm;
        // each gains an `SC0315`.
        an_array_of_unknown_size_in_a_block_is_incomplete: ["--emit", "ast"],
        // Nothing can be passed to a `void` parameter, and no `void` value can
        // be passed at all (6.5.2.2 p2, 6.3.2.2), so every call here is
        // `SC0302`, `u`'s too, which has no prototype to ask. Asked for the IR,
        // so that a call let through reaches the lowering: `f(g())` and
        // `u(g())` panicked there before, and `f(1)` built. Mutation: drop the
        // `void` parameter test in `check_argument`; the reports for `f(1)`
        // and `f(g())` go. Mutation: drop the test of an argument with no
        // prototype in `call`; the report for `u(g())` goes. Either way the
        // other reports keep the lowering away, so it is the stderr that
        // fails rather than a panic.
        an_argument_to_a_void_parameter_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C17 6.7.2.1 and 6.5.2.3 are read: a struct definition with its
        // members, a declaration of a tag and nothing else, an object, a
        // parameter, a local, and `.` and `->`, each in the tree. Every
        // object of struct type and every member access is typed, so the
        // only report is `s`, of a `struct T` nothing completes (`SC0315`,
        // C17 6.9.2 p2). `h` assigns one pointer to a struct to another of
        // its tag, and is not reported. Mutation: refuse `struct` in
        // `specifiers` again; this is `SC0201` at the first `struct`.
        // Mutation: skip a member access; its `Member` goes from the dump.
        // Mutation: read `->` as `.`; the dump's `"->"` goes, and `p->x` is
        // `SC0306`.
        // The rest is every place a struct is read: two members in one
        // declaration, a definition with a declarator at file scope and in a
        // block, a tag declared in a block, a `for` and a parenthesised
        // declarator beginning with `struct`, a struct with no tag, an array
        // of one and a function returning one, each used. Mutation: stop
        // reading a member list's `,`; `int a, b` is `expected ;`. Mutation:
        // test a `for` or a parenthesised declarator for a specifier with
        // `type_specifier_keyword` alone; the `for` or `abstract`'s parameter is a syntax
        // error.
        a_struct_is_read_and_refused_until_it_means_something: ["--emit", "ast"],
        // A program that uses structs as C allows, an object assigned from
        // `*p`, passed and returned, passes the type checker, and the lowering
        // refuses `make`, `take` and `f` under `SC0304`, since the IR holds no
        // struct, and lowers `main`. Mutation: refuse a declaration holding a
        // struct in the type checker again; the lowering is never reached,
        // `main` is not printed, and this fails.
        a_struct_program_is_typed_and_refused_by_the_lowering: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A struct with no tag declares nothing when nothing else is
        // declared (6.7 p2). Mutation: let any struct through without a
        // declarator; this loses its `SC0206`.
        a_struct_with_no_tag_and_no_declarator_declares_nothing: ["--emit", "ast"],
        // A declaration with no declarator has only its specifiers' type, and
        // a struct definition's members are declarations of their own, so
        // the zero-length member is refused as any array of length zero is
        // (`SC0309`, C17 6.7.6.2 p1). Mutation: leave a declaration's
        // `specified` type out of `check_declarators`; this exits 0.
        // Mutation: walk no member of a struct definition there; the same.
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
        // Three hundred `->` in a row through a struct nothing completes
        // are one fault, reported once, at the first (`SC0315`): every later
        // access has a base with no type, which was reported there. A report
        // each, with a span growing by one access each, was output and time
        // quadratic in the chain. Mutation: report a member access whose
        // base is an untyped member access as this compiler's gap; three
        // hundred reports.
        a_chain_of_member_accesses_is_refused_once: ["--emit", "ast"],
        // `s.b` names a member `struct S` does not have (C17 6.5.2.3 p1),
        // reported at `b` under `SC0316`, with the definition beside it.
        // Mutation: type a member access without looking its member up;
        // this goes silent.
        a_member_the_struct_does_not_have_is_reported: ["--emit", "ast"],
        // `p` is declared above `struct S`'s definition. `p->a` in `f` is
        // above it too, where the struct has no members yet (6.7.2.1 p8),
        // and is `SC0315`; in `g` it is below and typed. Mutation: ask
        // completeness where `p` was declared; `g`'s is reported too.
        a_member_of_a_struct_not_complete_where_it_is_named_is_reported: ["--emit", "ast"],
        // C17 6.7.2.1 p3's second sentence: a struct with a flexible array
        // member is no member of another (`SC0314`) and no element of an
        // array (`SC0309`). clang accepts both as an extension and refuses
        // them under `-pedantic-errors`. Mutation: drop either check; its
        // report goes.
        a_struct_with_a_flexible_array_member_is_no_member_and_no_element: ["--emit", "ast"],
        // A program using members as C allows passes the type checker, and
        // the lowering refuses its one member access, the first it meets,
        // under `SC0304`, since the IR holds no struct to take a member
        // of. Mutation: refuse a member access of the right shape in the
        // type checker again; that report is the type checker's, and the
        // lowering is never reached.
        a_program_using_members_is_typed_and_refused_by_the_lowering: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `struct S` is never completed, so `*p = *q` assigns to an object
        // of incomplete type (C17 6.5.16 p2) and reads the value of another,
        // and `*p, 0` reads one (6.3.2.1 p2): three `SC0315`. `&*p` reads no
        // value and is not reported. Mutation: report the operand of `&`
        // too; a fourth report, in `h`.
        an_incomplete_struct_whose_value_is_read_or_assigned_is_reported: ["--emit", "ast"],
        // An object at file scope is initialized with a constant expression
        // (C17 6.7.9 p4): `g`'s initializer reads `x` and `h`'s reads through
        // `s.p`, so both are `SC0317`, and the lowering, which drops a
        // file-scope initializer unread, is never reached. Before this, the
        // program built. Mutation: skip the check; this exits 0.
        an_initializer_at_file_scope_that_is_not_constant_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C17 6.7.2.3 p1: `V` given its content twice in one scope is
        // `SC0312`, and `W` given it again in a block is a new tag, which is
        // not. Mutation: look a definition up in every visible scope; `W` is
        // reported.
        // Mutation: drop the report; `V`'s goes.
        a_tag_defined_twice_in_one_scope_is_a_redefinition: ["--emit", "ast"],
        // C17 6.7.2.1 p3 and 6.7 p3: a member that is the struct being
        // defined, `void`, or a function is `SC0314`, two members of one name
        // are `SC0312`, and a flexible array member after a named one is
        // neither. Mutation: mark a definition complete when it is bound; `s` goes
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
        // C17 6.2.7 p1: `p` declared as a pointer to two tags is `SC0313`,
        // both struct types spelled. Mutation: answer every struct pair
        // compatible in `Ast::compatible`; the conflict goes silent.
        declarations_of_one_name_as_two_struct_types_conflict: ["--emit", "ast"],
        // Valid C that is not read yet, each told so with the paragraph that
        // makes it valid: an anonymous member (C17 6.7.2.1 p13) and a
        // bit-field, named or not (p9). Mutation: send a member with no
        // declarator to the `int;` check again; the anonymous member is
        // `SC0206`, a claim that valid C is invalid. Mutation: drop either
        // `:` check; that bit-field is `expected` something instead.
        a_struct_with_an_anonymous_member_is_not_read_yet: ["--emit", "ast"],
        a_bit_field_is_not_read_yet: ["--emit", "ast"],
        a_bit_field_with_no_name_is_not_read_yet: ["--emit", "ast"],
        // The two kinds `SC0201` and `SC0207` tell apart, each pinned whole: a
        // `switch` is C (6.8.4.2) this parser does not read, and a `case` with
        // no `switch` around it breaks 6.8.1 p2's constraint, so it is not C.
        // Mutation: add a `Keyword::Case` row to `unread_keyword`, and the
        // second case fails; drop the `switch` row, and the first does.
        a_switch_is_not_read_yet: ["--emit", "ast"],
        a_case_outside_a_switch_is_malformed: ["--emit", "ast"],
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
        // A path that names nothing: this case's `.c` is absent on purpose,
        // and that absence is the input. The note is this compiler's
        // words, so the report is the same bytes on every host, where the
        // operating system's own message is not. Mutation: delete the arm of
        // `reason` that names `NotFound` for a read; the note becomes the
        // operating system's and the blessed stderr differs.
        a_path_that_names_nothing_is_reported_in_this_compiler_s_words: ["--emit", "ast"],
        // A case that writes a file, here the tokens `-o` names, and leaves
        // nothing in the corpus, because it runs from a directory of its own.
        // Its stdout is empty because the artifact went to the file. Mutation:
        // run it from its group's directory; the refusal in `run_case` fails
        // it by name, and `a_case_is_run_from_a_directory_outside_the_corpus`
        // too. Without that refusal `out.tok` would be left beside the
        // expected files and the stray guard would fail on a later walk.
        a_case_that_writes_a_file_leaves_none_in_the_corpus: ["--emit", "tokens", "-o", "out.tok"],
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
        // Three broken items around one that is fine report three errors,
        // and the tree keeps the good one between the `Error` items (#35).
        // Mutation: do not clear `failed` before each item in `Parser::run`;
        // this reports one.
        three_syntax_errors_are_three_reports: ["--emit", "ast"],
        parentheses_regroup: ["--emit", "ast"],
        parsed_function: ["--emit", "ast"],
        pointer_declaration: ["--emit", "ast"],
        pointer_to_function: ["--emit", "ast"],
        precedence_additive: ["--emit", "ast"],
        // `a ? b : c = d` groups as `(a ? b : c) = d`, which is what the tree
        // shows; a conditional is no lvalue, so the type check refuses it as
        // `SC0318`, as `clang` does, and the tree is printed all the same.
        precedence_assignment: ["--emit", "ast"],
        // A prototype's length is never lowered (C17 6.7.6.2 p5 treats it as
        // `*`), so the type check is the only stage that sees an operand there
        // that cannot be written to or have its address taken, provided it
        // typed the operand. One prototype per place it is asked, `=`, postfix
        // `++`, prefix `--` and `&`, each citing its own clause, and one more
        // `=` whose left operand is `*g2`, an array, typed since `*` of an
        // array is. Mutation: stop asking at any one of them, or cite another
        // clause there; that prototype's report goes or changes and the
        // blessed stderr differs. Mutation: leave `*` of an array untyped
        // again; the last prototype's report goes.
        an_operand_in_a_prototype_s_length_that_cannot_be_written_or_addressed_is_refused: ["--emit", "ast"],
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
        // C17 6.4.6 p3: each digraph is its punctuator, so this is an array
        // and a body. Mutation: advance past a digraph by the punctuator's
        // own length; the second byte of each is scanned again as a token
        // of its own, `<:` becoming `[` then `:`, and this fails. Mutation:
        // drop the digraph table; `<%` is `<` then `%`.
        digraphs_are_the_punctuators_they_stand_for: ["--emit", "ast"],
        // `%:` at the start of a line is `#`, so `%:pragma` is a directive
        // and `SC0104`, as the `#pragma` beside it is, and `%:%:` is `##`,
        // which begins no directive at the start of a line or in the
        // middle of one. Mutation: ask the directive test of `#` alone; the
        // first report goes and `%:pragma` is read as tokens. Mutation: let
        // the test take `##` too; the `%:%: x` line becomes a directive.
        // Mutation: point the label at one byte; the caret under `%:`
        // narrows to `%`.
        a_directive_spelled_with_a_digraph_is_a_directive: ["--emit", "tokens"],
        // C17 5.1.1.2 p1 phase 2 splices `// a comment \` and the line
        // below it into one comment, so of the two `return`s only the last
        // is code. Mutation: end a line comment at the newline, as before;
        // two `Return`s.
        a_line_comment_goes_on_across_a_splice: ["--emit", "ast"],
        // A `*`, splices and a `/` close a block comment, one splice or two,
        // so `x` and `y` are tokens and not comment. A splice inside one
        // goes on the logical line, so the `#` after `*/` on line 9 follows
        // `int z;` and begins no directive. Mutation: close a block comment
        // only at `*/`; the first runs on and swallows both. Mutation: allow
        // one splice between `*` and `/`; the second is unterminated.
        // Mutation: let a splice's newline open a line; line 9 becomes a
        // directive and `SC0104`.
        a_block_comment_closes_across_a_splice: ["--emit", "tokens"],
        // A `\` with a space before the newline is spliced in a comment, as
        // clang and gcc splice it, with a warning (`SC0108`): the line
        // comment takes `return 1;` and the block comment closes across it.
        // Mutation: refuse spaces before the newline; `return 1;` is code
        // and the block comment swallows `return 0;`.
        a_spaced_splice_in_a_comment_is_spliced_with_a_warning: ["--emit", "ast"],
        // `??/` is a splice only where trigraphs are read, which clang's and
        // gcc's default modes do not, so in a comment, where it would decide
        // what is code, it is refused (`SC0107`). Mutation: perform it in
        // silence; both reports go.
        a_trigraph_splice_in_a_comment_is_refused: ["--emit", "ast"],
        // Outside a comment a splice or a trigraph is reported as not
        // supported yet (`SC0107`), at every place one can be: in an
        // identifier, in a string, `??-` in code, `??/` as a splice in code
        // (its caret three bytes wide), a `\\` whose second backslash
        // splices, a trigraph in a string and one after a `\` in a
        // character constant, and after a stray character, each in place of
        // the unexpected character, unterminated literal or syntax error it
        // was before. Mutation: skip any one report; its `SC0107` goes.
        a_splice_or_trigraph_outside_a_comment_is_reported: ["--emit", "tokens"],
        unterminated_block_comment: ["--emit", "tokens"],
        // One byte, one position, in the header and in the token artifact of
        // one run. A lone carriage return breaks no line to the source map, so
        // it breaks none in a header either and is shown as `?`: the `@` is
        // `1:16` in the `.stdout` and in the `.stderr`. Mutation: leave a lone
        // `\r` alone in `echoed`; the header says `2:9`.
        a_carriage_return_alone_is_not_a_line_break: ["--emit", "tokens"],
        // A byte-order mark in mid-line is three bytes of nothing on a
        // terminal, so its marker was drawn under the `1;` after it. It is
        // drawn as U+FFFD with the marker under that. Mutation: drop
        // `is_zero_width` from `echoed`; the marker lands under `1;`.
        a_zero_width_character_is_shown_where_it_is: ["--emit", "tokens"],
        // The byte after a file's last `\n` is `2:1` to the source map, where
        // the token artifact puts `eof`, and the header says so rather than
        // putting it at the end of line 1. Mutation: never give the rendered
        // copy its last empty line; the header says `1:7`.
        a_diagnostic_at_the_end_of_a_file_is_on_the_line_after_the_last: ["--emit", "ast"],
        unterminated_character_constant: ["--emit", "tokens"],
        unterminated_string_literal: ["--emit", "tokens"],
        void_is_not_the_only_parameter: ["--emit", "ast"],
        while_statement: ["--emit", "ast"],
    }

    "preprocessor" => {
        // `#include "file"`, each with a `.h` of its own name beside it where
        // it includes one. The directives the preprocessor refuses are in
        // `frontend`, where they were refused before it existed:
        // `unsupported_directive` for `#include <...>`, and
        // `a_directive_stops_the_input_it_is_in` for `#define`.
        //
        // Mutation: splice nothing in place of the directive. `twice` is a
        // name nothing declares.
        a_function_declared_in_an_included_header_is_called: ["--emit", "ast"],
        // Mutation: display an inclusion as any name but its path. The
        // header line stops naming the `.h`.
        a_diagnostic_in_an_included_file_names_that_file: ["--emit", "ast"],
        // Mutation: do not push the included file on the stack of those being
        // read. The header is read until the bound on inclusions refuses it,
        // as `SC0112` rather than `SC0110`.
        a_header_that_includes_itself_is_refused: ["--emit", "ast"],
        // Mutation: do not put the input on that stack. It is read once more
        // as an inclusion, and the report points at that directive rather
        // than saying it is the input.
        an_input_that_includes_itself_is_refused: ["--emit", "ast"],
        // The input includes a header that includes the input. Mutation: ask
        // only the innermost file being read rather than all of them. The
        // two are read in turn until the depth bound refuses the next, so the
        // report is `SC0111` rather than `SC0110`.
        a_file_that_includes_itself_through_another_is_refused: ["--emit", "ast"],
        // `./` before its own name. Mutation: compare the paths as they were
        // resolved rather than canonicalized. It is read once more as an
        // inclusion before the cycle is seen.
        an_input_that_includes_itself_by_another_spelling_is_refused: ["--emit", "ast"],
        // Mutation: drop the note. What was looked for, and why it was not
        // read, goes with it.
        an_include_that_names_no_file_is_refused: ["--emit", "ast"],
        // A Latin-1 byte in a comment, which is ordinary in the code this
        // compiler is for. Mutation: answer every failure with `reason`. The
        // note gives the standard library's words for a decoding failure
        // under a label saying they are the operating system's.
        an_included_file_that_is_not_utf_8_is_refused: ["--emit", "ast"],
        // A body whose braces are in one file and whose statement is in
        // another: the item holds the header whole, so it is read, as it is
        // written out. Mutation: refuse an item whose tokens are in two
        // files, as `Parser::in_one_file` did. It is refused.
        a_function_body_made_of_an_include_between_its_braces_is_read: ["--emit", "ast"],
        // `int a;` then `int` in the header and `b;` after the directive, so
        // the second declaration starts partway through the header. Mutation:
        // have `Parser::join` lift with `lift_start` and `lift_end`, which
        // panic on a cut. It panics.
        a_construct_that_cuts_an_include_is_refused: ["--emit", "ast"],
        // The end's counterpart: `int b` then a header of `= 1; int c;`, so
        // the declaration ends partway through it. Mutation: word the end's
        // label as the start's. The label is wrong.
        a_construct_ending_partway_through_an_include_is_refused: ["--emit", "ast"],
        // `a_call_in_the_exposing_call_s_arguments_is_not_asked_about_it`
        // with `release_all()` included as `memset`'s argument: it runs
        // before `memset`, as written out, so it cannot free what `memset`
        // exposes. Mutation: have `inside` compare without locating. The
        // included call is not inside `memset`'s span, and this reports.
        a_call_in_an_included_argument_is_not_asked_about_the_call_around_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `y =` then an `#include` of `(x + 1)` then `;`: the assignment ends
        // at the `)`, which is the header's last token. Mutation: join the
        // climb's nodes from their operands' spans, as `Parser::joined` did.
        // The assignment ends at `1`, cuts the header, and is refused.
        a_parenthesised_included_operand_is_not_refused: ["--emit", "ast"],
        // Object-like macros (#546), each in a file of its own. Every case
        // whose program can be written out was run beside its written-out
        // twin, and the two give the same errors and exit code.
        //
        // Mutation: expand nothing. `N` is a name nothing declares.
        a_macro_compiles_to_the_program_its_body_writes: ["--emit", "ast"],
        // `#define _Nonnull` with nothing after it, the portability idiom: the
        // declaration then promises nothing, and the dereference is the
        // `SC0403` it is written out. Mutation: offer only identifiers and
        // keywords for expansion. The annotation is kept and this exits 0.
        a_macro_that_removes_an_annotation_removes_its_promise: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: add no label at the use; or place labels without
        // `SourceMap::written`. The `expanded here` goes, or the header names
        // `<expansion>`.
        a_diagnostic_in_an_expansion_shows_the_body_and_the_use: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `#define x x + 1`: the `x` in the body is not replaced again
        // (C17 6.10.3.4 p2). Mutation: drop the open-expansion check in
        // `expansion`. It expands until the bound refuses it as `SC0113`.
        a_macro_that_names_itself_is_expanded_once: ["--emit", "ast"],
        // Mutation: read `F(` as an object-like macro, or let `##` and `#`
        // through. A report goes.
        a_function_like_macro_and_stringizing_are_not_read_yet: ["--emit", "ast"],
        // Mutation: read a `#define` whatever its line ends in. The body's
        // second line is compiled as code.
        a_define_ending_in_a_splice_is_refused: ["--emit", "ast"],
        // The same tokens, a comment between two of them in one and a space in
        // the other, which C counts as the same whitespace. Mutation: compare
        // the replacement lists' text. The comment makes them differ, and it
        // is refused.
        a_macro_defined_again_the_same_is_accepted: ["--emit", "ast"],
        // Mutation: accept any second definition. Both reports go.
        a_macro_defined_again_differently_is_refused: ["--emit", "ast"],
        // Mutation: drop `#undef`. The parameter is read as `4`.
        an_undefined_macro_is_an_identifier_again: ["--emit", "ast"],
        // A block comment that goes on past the `#define`'s line, holding a
        // null test: in C the test is part of the comment. Mutation: read a
        // directive with an open comment as if it ended at its newline. The
        // test is compiled, and the dereference is not reported.
        a_comment_past_a_define_line_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: take the name as ending at the first character that is
        // not part of one. `#undef A$B` undefines `A`, and `A` is the global.
        an_undef_of_a_name_with_a_dollar_is_refused: ["--emit", "ast"],
        // A comment is whitespace in a directive: between `#` and `define`,
        // after `define`, and between a name and its list. Mutation: read
        // the line without making its comments spaces. Two are refused.
        a_comment_in_a_define_line_is_whitespace: ["--emit", "ast"],
        // Mutation: drop the test for `defined`. It is read as a macro.
        defined_is_not_read_as_a_macro_s_name: ["--emit", "ast"],
        // ADR-0054's cut: `F(p)` with `#define F (x = *p) + release` starts
        // its call at `release`, partway through the expansion. Mutation:
        // have `Parser::join` panic on a cut. It panics.
        a_call_that_cuts_an_expansion_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // ADR-0054's nested cut: `#define B release` inside `#define A (x =
        // *p) + B`, used as `A(p)`. The call's callee is the whole of `B`'s
        // expansion and the end of `A`'s, so only the outer level is cut.
        // Mutation: check a lift's edge at the innermost level only. It is
        // joined.
        a_call_that_cuts_a_nested_expansion_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same call holding the whole expansion, `G(p)` with `#define G
        // release`, is joined at the use and checked as written out.
        // Mutation: have `SourceMap::join` answer a cut for any two spans in
        // different files. It is refused.
        a_call_holding_a_whole_expansion_is_checked_as_written: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `#define N (*p)`, the usual way to write one. Mutation: join the
        // climb from its operands' spans. `N + 1` starts at `*`, partway
        // through the expansion, and is refused.
        a_parenthesised_macro_body_is_not_refused: ["--emit", "ast"],
        // ADR-0054's first silence: a store and a call from one expansion.
        // Mutation: give every token of an expansion its use's span, the
        // design ADR-0054 rejected. The use of `q` is not reported.
        a_store_and_a_call_from_one_expansion_are_checked_as_written: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // ADR-0054's third: two declarators from one expansion are two
        // locals, and `a` used after its free is reported. Mutation: give
        // every token of an expansion its use's span. The two names collide
        // in the lowering's locals.
        two_declarators_from_one_expansion_are_two_locals: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // ADR-0054's read inside `g(N)`: an expanded call as an argument runs
        // before the call around it. Mutation: have `inside` compare without
        // locating. This reports.
        a_call_in_an_expanded_argument_is_not_asked_about_the_call_around_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // ADR-0054's free from a macro in a parameter's length. Mutation:
        // have `unordered` compare in one file. It is worded as the body's.
        a_free_from_a_macro_in_a_parameter_s_length_is_worded_as_a_length_s: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same with the included `(p)` starting the node: a member
        // access, a comma and an assignment, each beginning at the header's
        // `(`. Mutation: start any one of the three from its left operand's
        // span rather than from the token its production began at. It starts
        // at `p`, partway through the header, and is refused.
        a_parenthesised_included_operand_that_starts_a_node_is_not_refused: ["--emit", "ast"],
        // Spans this compiler builds for itself, falling back to the C
        // construct's own tokens where they would cut a header, each read as
        // its written-out twin is, with the same errors (#555). A declarator
        // list, `int` then an `#include` of `*a, *b` then `;`. Mutation: drop
        // `Parser::declaration_span`'s fallback. It is `SC0207`.
        a_declarator_list_from_a_header_is_read_as_written_out: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A parameter with a length after one without, both from the header,
        // so the first length starts partway through it. Mutation: drop the
        // fallback to the whole parameter list. It is `SC0304`.
        a_parameter_with_a_length_after_one_without_from_a_header_is_read_as_written_out: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `int` then an `#include` of `*a` then `= p;`: the store's name is
        // partway through the header, and its init-declarator holds it whole.
        // Mutation: drop `Lowering::store_span`'s fallback. It is `SC0304`.
        a_store_whose_name_is_partway_through_an_include_is_read_as_written_out: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And where the init-declarator cuts too, `x, *a` from the header then
        // `= 0;`, it is refused. Mutation: have the lowering's join `expect` a
        // span. It panics.
        a_store_whose_declarator_starts_partway_through_an_include_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The fallback holds this declarator and no other: `int y = *q,` then
        // an `#include` of `*a` then `= 0;` after `free(q)` reports the read
        // of `q`, and `a`'s store is placed at the directive. Mutation: fall
        // back to the declaration's span, from its specifiers. The store is
        // placed at `int` and this fails; the read is still reported, since
        // `y`'s initializer is sequenced before `a`'s store.
        a_sibling_declarators_read_beside_a_store_from_a_header_is_reported: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `a_read_in_one_length_meets_what_another_length_frees` with `int
        // b[*q]` included as a parameter: the same `SC0402` and `SC0409`.
        // Mutation: join the lengths' span from the first length's node to
        // the last's, as the lowering did. The node starts at `*q`, partway
        // through the header, and the function is refused as `SC0304`.
        a_parameter_with_a_length_from_an_include_is_checked_as_written: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A double free in a header included above a double free written
        // out: the header's findings come first, as its `#include` does.
        // Mutation: sort findings by file index, as `memory::findings` did.
        // The header's are listed last.
        an_included_free_is_listed_where_its_include_is: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same for the nullability check's findings. Mutation: sort them
        // by file index, as `nullability::findings` did. The header's is
        // listed last.
        an_included_null_dereference_is_listed_where_its_include_is: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // And for what a hatch concludes, in its dump: the included double
        // free is above the written one. Mutation: sort the dump's
        // conclusions by file index, as `dump_hatches` did. The header's is
        // dumped last.
        an_included_conclusion_is_dumped_where_its_include_is: ["--emit", "hatches", "--target", "x86_64-pc-windows-msvc"],
        // Mutation: do not pop a file off the stack when its inclusion ends.
        // The second inclusion is refused as one that includes itself.
        a_header_included_twice_is_read_twice: ["--emit", "ast"],
        // `#ifdef`, `#ifndef` and `#else` (#547), nested, the chosen groups
        // defining what `main` calls. Mutation: take the other group of an
        // `#ifdef`; or read `#ifndef` as `#ifdef`. `chosen` or `also` is
        // undeclared.
        ifdef_and_ifndef_select_a_group_by_whether_the_name_is_defined: ["--emit", "ast"],
        // Mutation: take an `#ifndef`'s group whatever is defined. The
        // second inclusion defines `struct S` again, `SC0312`.
        a_header_with_an_include_guard_included_twice_declares_once: ["--emit", "ast"],
        // Mutation: count no conditional opened in a skipped group. The inner
        // `#endif` closes the outer one, and `struct S broken` is compiled.
        a_nested_endif_in_a_skipped_group_closes_only_its_own: ["--emit", "ast"],
        // Read only for their names (C17 6.10.1 p6): `#elif` after `#else`,
        // `#include <...>`, `#pragma` and a function-like macro, each refused
        // outside a skipped group, pass. Mutation: read directives in a
        // skipped group. They are reported.
        a_directive_in_a_skipped_group_is_not_read: ["--emit", "ast"],
        // A run no token can begin is a preprocessing token, ignored where
        // its group is skipped and an error only once it is compiled, so
        // `@class` under an `#ifdef` not taken passes. Mutation: report an
        // `Unknown` token before `read_includes` asks whether its group is
        // skipped. It is `SC0103` and exits 1.
        a_character_no_token_can_begin_in_a_skipped_group_is_not_reported: ["--emit", "ast"],
        // What changes which groups there are is still reported there, and
        // so is what C leaves undefined in any group (C17 6.4 p3): a comment
        // left open swallows the `#endif`, and an apostrophe in prose opens
        // a character constant that does not close. Mutation: have the scan
        // stop reporting an unterminated comment; the first goes silent about
        // it. Mutation: have it stop reporting an unterminated literal; the
        // second exits 0.
        an_unterminated_comment_in_a_skipped_group_is_reported: ["--emit", "ast"],
        an_unterminated_character_constant_in_a_skipped_group_is_reported: ["--emit", "ast"],
        // Mutation: report nothing still open when a frame ends. It exits 0.
        an_unterminated_conditional_is_refused_at_its_directive: ["--emit", "ast"],
        // An `#ifndef` left open in a header, and an `#endif` for it in the
        // includer: a conditional is the file's it begins in (C17 6.10 p1).
        // Mutation: keep one stack of conditionals for every frame. Neither
        // is reported.
        a_conditional_does_not_go_on_across_a_header: ["--emit", "ast"],
        // Mutation: drop the check for an open conditional at `#else`,
        // `#endif` or `#elif`. One of the three goes unreported.
        an_endif_or_else_with_nothing_open_is_refused: ["--emit", "ast"],
        // Mutation: drop the check for a second `#else`. Nothing is reported.
        // Which group is handed over is asked of the tokens, in
        // `no_group_of_a_refused_or_doubled_conditional_is_handed_over`.
        a_second_else_is_refused: ["--emit", "ast"],
        // Mutation: read an `#ifdef` with no name as one naming nothing
        // defined. Its group is skipped and nothing is reported.
        an_ifdef_with_no_name_is_refused: ["--emit", "ast"],
        // `$` may be part of a name where extended characters are read, so it
        // is C not read yet, `SC0104`, rather than a name missing. Mutation:
        // drop the report. It exits 0.
        an_ifdef_of_a_name_with_a_dollar_is_refused: ["--emit", "ast"],
        // Mutation: drop the check for an `#else` before an `#elif`. It is
        // reported as an `#elif` not read yet instead, `SC0104`.
        an_elif_after_else_is_refused: ["--emit", "ast"],
        // `#ifdef __GNUC__` above a second free: `gcc` builds the double
        // free and MSVC does not, so no answer is right for every build. A
        // reserved name the source defines, `__MINE`, is read, and so is one C
        // does not reserve, `_lower`. Mutation: answer a reserved name no
        // target predefines as undefined. It exits 0, the free analysed away.
        an_ifdef_of_a_name_the_implementation_may_define_is_refused: ["--emit", "safety-ir", "--target", "x86_64-unknown-linux-gnu"],
        // `#define X \` in a skipped group joins `#else` to itself, so C
        // skips `p = malloc(4);` and builds a double free. Mutation: ask of a
        // splice only in a conditional the frame acts on. It exits 0.
        a_directive_in_a_skipped_group_ending_in_a_splice_is_refused: ["--emit", "safety-ir", "--target", "x86_64-unknown-linux-gnu"],
        // The same program for two targets (#559): `_WIN32` answers
        // differently, and `__linux__` is answered rather than refused where
        // no compiler for the target predefines it. Mutation: predefine
        // nothing. `_WIN32` is refused. Mutation: read one target's list for
        // every target. One of the two takes the other group.
        win32_is_defined_for_msvc: ["--emit", "ast", "--target", "x86_64-pc-windows-msvc"],
        win32_is_not_defined_for_linux: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu"],
        // `201710L` reaches the parser, which refuses its suffix as it would
        // written out. Mutation: predefine nothing. `__STDC_VERSION__` is a
        // name nothing declares.
        the_standard_version_expands_to_c17: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu"],
        // The double free #547's review found analysed away under `#ifdef
        // __STDC_VERSION__` is reported. Mutation: predefine nothing. The
        // `#ifdef` is refused and nothing is analysed.
        a_double_free_under_a_predefined_name_is_found: ["--emit", "safety-ir", "--target", "x86_64-unknown-linux-gnu"],
        // A header guarded by `_MT`, which `cl.exe` predefines and `clang`
        // does not, defining a macro the input's `#ifndef` asks about: a
        // `cl.exe` build skips the header and compiles the double free.
        // Mutation: read `#ifndef N` followed by `#define N` as a guard,
        // whatever `N` is. It exits 0.
        a_reserved_guard_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same program with `-U _MT`: one build, the one without `_MT`.
        // The header is read, `SINGLE_THREADED` is defined, and only one free
        // is compiled. Mutation: drop the settled clause in `act`. The guard
        // is `SC0104` again.
        a_reserved_guard_is_read_once_the_command_line_says_the_build_does_not_define_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "-U_MT"],
        // And with `-D _MT`: the build with it, which skips the header and
        // compiles both frees, so the double free is reported. Mutation: hand
        // `preprocess` no command-line macros. The guard is `SC0104` instead.
        a_reserved_guard_is_skipped_once_the_command_line_says_the_build_defines_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "-D_MT"],
        // The `extern "C"` idiom: C forbids an implementation to predefine
        // `__cplusplus` (C17 6.10.8 p3), so it is undefined in every build.
        // Mutation: leave it to the rule for reserved names. Both `#ifdef`s
        // and the `#ifndef` are refused.
        cplusplus_is_never_defined: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu"],
        // `unix` is defined by `gcc` and `clang` in their default GNU modes
        // and not under `-std=c17`. Mutation: leave it out of Linux's
        // unsettled names. It is answered as undefined, and the double free
        // analysed away.
        a_gnu_mode_s_name_is_refused_for_linux: ["--emit", "safety-ir", "--target", "x86_64-unknown-linux-gnu"],
        // `clang` predefines `__x86_64__` for MSVC and `cl.exe` does not, so
        // the double free `cl.exe` builds is in the `#else`. Mutation: answer
        // a name the target's compilers disagree on from the lists. It is
        // undefined, and the double free is reported where the `#ifdef`
        // should have been refused.
        a_name_its_compilers_disagree_on_is_refused_for_msvc: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // `-D TWICE` skips the `#ifndef` group's allocation and takes the
        // `#ifdef` group's second free, so the double free is reported.
        // Mutation: hand `preprocess` no command-line macros. Both groups go
        // the other way, and it exits 0.
        a_name_defined_on_the_command_line_chooses_its_group: ["--emit", "safety-ir", "--target", "x86_64-unknown-linux-gnu", "-D", "TWICE"],
        // A value of three tokens with blanks between them, rather than `1`.
        // Mutation: define every `-D` as `1`. The initialiser is a literal.
        a_value_given_on_the_command_line_is_expanded: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu", "-DSIZE=2 * 2"],
        // `-U __GNUC__` settles the name, so `#ifndef __GNUC__` takes its
        // group and the double free in it is reported (ADR-0059). Mutation:
        // drop the settled clause from the reserved-name refusal. It is
        // `SC0104` again and nothing is analysed.
        a_reserved_name_undefined_on_the_command_line_is_answered: ["--emit", "safety-ir", "--target", "x86_64-unknown-linux-gnu", "-U__GNUC__"],
        // A header guarded by `_TWICE_H`, included twice and holding a
        // function's definition: read once, or the second definition would be
        // refused. Mutation: drop the settled clause. The guard is `SC0104`
        // at the first inclusion.
        an_include_guard_named_with_a_reserved_name_is_read_when_the_command_line_undefines_it: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu", "-U_TWICE_H"],
        // `_WIN32` is the target's, and the command line describes the build
        // its compilers make. Mutation: let `-U` remove a predefined name. It
        // exits 0.
        a_name_this_target_predefines_cannot_be_undefined_on_the_command_line: ["--emit", "ast", "--target", "x86_64-pc-windows-msvc", "-U_WIN32"],
        // `-D N=2` meets `#define N 3` as two `#define`s meet: `SC0114`, with
        // the first at `<command line>`. Mutation: hand `preprocess` no
        // command-line macros. It exits 0.
        a_command_line_definition_and_a_different_one_in_the_source_are_refused: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu", "-DN=2"],
        // C17 6.10.8 p2's names stay the implementation's under `-D` and `-U`
        // alike: `SC0117` for each, with C17's note. Mutation: refuse a
        // standard name's `-U` as any predefined name's is. `__STDC__` is
        // told the target predefines it, which is not the rule it breaks.
        // `__x86_64__` is one MSVC's compilers disagree on, so it is refused
        // unless settled; `-U` settles it, and the `#ifndef` group's double
        // free is reported. Mutation: ask the settled set only of reserved
        // names the target does not list. It is `SC0104` again.
        a_name_the_target_leaves_open_undefined_on_the_command_line_is_answered: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "-U__x86_64__"],
        // `-D N=3` and `#define N 3` are two identical definitions, which
        // C17 6.10.3 p2 allows. Mutation: refuse a second definition of a
        // settled name whatever it says. It is `SC0114`.
        a_command_line_definition_and_the_same_one_in_the_source_are_accepted: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu", "-DN=3"],
        // `-D _WIN32=1` says what MSVC's target already says, and is accepted.
        // Mutation: refuse any `-D` of a predefined name. It is refused.
        a_name_this_target_predefines_may_be_defined_again_the_same_way_on_the_command_line: ["--emit", "ast", "--target", "x86_64-pc-windows-msvc", "-D_WIN32=1"],
        // `-U _FORTIFY_SOURCE -D _FORTIFY_SOURCE=2`, a hardened build's
        // spelling: read left to right, as `cc` reads it, so the name ends
        // defined as `2`. Mutation: put every `-D` before every `-U`. The `-U`
        // comes last, and `_FORTIFY_SOURCE` is a name nothing declares.
        the_command_line_is_read_in_the_order_it_was_given: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu", "-U_FORTIFY_SOURCE", "-D_FORTIFY_SOURCE=2"],
        a_standard_name_cannot_be_defined_or_undefined_on_the_command_line: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu", "-D__STDC_HOSTED__=0", "-U__STDC__"],
        // On MSVC `__STDC__` is not predefined but left open, so it is not a
        // macro when the command line is read. Refused all the same, as a name
        // C17 6.10.8 lists. Mutation: refuse a standard name only where it is
        // already defined, as the `#define` and `#undef` paths do. `-U` settles
        // it, `#ifdef __STDC__` skips the second free, and it exits 0.
        a_standard_name_this_target_leaves_open_cannot_be_settled_on_the_command_line: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc", "-U__STDC__"],
        // `#ifdef __LINE__` is answered, and the use is refused. Mutation:
        // expand a dynamic macro as its empty list. `int line = ;` is
        // reported as a syntax error instead.
        the_line_macro_is_refused_where_it_is_used: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu"],
        // C17 6.10.8 p2, for the names it lists. Mutation: let a `#define`
        // or `#undef` of a predefined name through. Nothing is reported.
        a_predefined_macro_defined_again_is_refused: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu"],
        // A target's names are not 6.10.8's: defined again the same, or
        // undefined, as `clang` accepts. Mutation: refuse a `#define` or an
        // `#undef` of every predefined name. Two are reported.
        a_target_s_name_may_be_defined_again: ["--emit", "ast", "--target", "x86_64-unknown-linux-gnu"],
        // `#ifndef A\u00e9` asks about `A\u00e9`, which nothing can define
        // here, and C builds the double free under it. Mutation: let a name
        // end at a `\`. It is `#ifndef A`, the free is skipped, and it exits
        // 0 with a warning.
        a_conditional_name_going_on_in_a_universal_character_name_is_refused: ["--emit", "safety-ir", "--target", "x86_64-unknown-linux-gnu"],
        // An `#ifndef` refused for a comment past its line still opens its
        // level, so its `#else` and `#endif` are not reported. Mutation: keep
        // no level for a line refused for its comment. Both are.
        a_conditional_refused_for_its_comment_keeps_its_level: ["--emit", "ast"],
        // Each directive is obeyed, with a warning for what follows it, and
        // a comment after one is no token. Mutation: make the warning an
        // error. It exits 1.
        tokens_after_endif_are_a_warning: ["--emit", "ast"],
        // A refused `#if`'s `#endif` is matched. Mutation: open no level at
        // `#if`. The `#endif` closes no conditional. That none of its groups
        // is handed over is asked of the tokens, in
        // `no_group_of_a_refused_or_doubled_conditional_is_handed_over`.
        if_and_elif_are_not_read_yet: ["--emit", "ast"],
        // C joins the empty line below to the `#ifndef`'s.
        // Mutation: read a conditional whatever its line ends in. It exits 0.
        a_conditional_ending_in_a_splice_is_refused: ["--emit", "ast"],
        // `-o` naming the header the input includes. Mutation: ask only the
        // inputs the command line named. The dump is written over the header
        // and the run succeeds.
        an_output_that_names_an_included_file_is_refused: [
            "--emit", "ast", "-o", "an_output_that_names_an_included_file_is_refused.h"
        ],
        // C joins the line below to the comment after the name, so the null
        // test is not code. Mutation: read the include whatever its line ends
        // in. The null test is compiled and the program checks as safe, which
        // in C it is not.
        an_include_whose_line_ends_in_a_splice_is_refused: ["--emit", "safety-ir"],
        // Mutation: report a missing token at the token that is there, as
        // every other expectation is. The caret goes under `int main`, in the
        // `.c`, where nothing is missing.
        a_missing_semicolon_at_the_end_of_a_header_is_placed_in_the_header: ["--emit", "ast"],
        // Mutation: write the note's path with no escaping on the way out.
        // The escape sequence reaches the terminal.
        an_included_path_with_a_control_character_is_shown_escaped: ["--emit", "ast"],
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
        // An array of arrays decayed, a row reached through a pointer to an
        // array, and a pointer moved by a `char`, widened by its sign
        // (#621). Mutation: spell the element of `getelementptr` as `i8`;
        // this changes.
        llvm_ir_of_arrays_and_pointer_arithmetic: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
        the_mvp_becomes_llvm_ir: ["--emit", "llvm-ir", "--target", "x86_64-pc-windows-msvc"],
    }

    "lowering" => {
        // The shape of the Safety IR a program lowers to, where no check and no
        // frontend report is what the case is about.
        an_initializer_becomes_a_store: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // C17 6.9.1 p10: a parameter's length is evaluated on entry, so the
        // store of `2` to `n` comes before the body, and a marker after it.
        // Mutation: refuse a length that is not a constant again; this is
        // `SC0304`. Mutation: drop the call to `Lowering::lengths`; the store
        // goes and `n` returns what the caller passed.
        a_parameter_s_length_is_evaluated_on_entry: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A declaration that is not a definition may have a `void` parameter
        // (C17 6.7.6.3 p12), and the type check accepts `f` and `d`; no IR type
        // holds nothing, so each is `SC0304`, the lowering's gap, where it was
        // the backend's `SC0801` before. Mutation: drop the `void` test in
        // `Lowering::signature`; both reports go, and the IR is printed.
        a_declared_void_parameter_is_not_compiled_yet: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A prototype's length is never lowered, so a part of it the type
        // check left untyped without a report, `p - q` today, hides a fault
        // nothing asks about. Each is refused as `SC0304`, with a label on
        // the untyped part. The first is untyped at its root and nowhere
        // else; the last one's untyped assignment is two levels below the
        // length, under a comma that is typed; the middle two are faults
        // `clang` refuses. Mutation: ask only that the length as a whole is
        // typed; the middle two and the last build. Mutation: look only at
        // the length's operands, not the length itself; the first builds.
        // Mutation: look only at the length's own operands; the last builds.
        // Mutation: drop the label on the untyped part; every block changes.
        an_untyped_part_of_a_prototype_s_length_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // The same length inside the type of a file-scope object, which
        // nothing lowers while nothing uses it, is asked too (#448): a
        // function pointer, a prototype nested in one of its parameters, and
        // an array of them. Mutation: have `declare_one` not walk an object's
        // type; the first builds. Mutation: have the walk not enter a
        // function's parameters, or an array's element; the third, or the
        // fourth, builds. And a length typed throughout still builds there.
        // Mutation: refuse every length the walk meets; the second is
        // refused.
        an_untyped_part_of_a_file_scope_function_pointer_s_length_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        a_file_scope_function_pointer_s_length_typed_throughout_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_untyped_part_of_a_length_in_a_prototype_nested_in_a_function_pointer_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_untyped_part_of_a_length_in_an_array_of_function_pointers_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // What the walk asks is only whether a length was typed, at any
        // depth, since nothing lowers an object's type: an inner variable
        // length is C (C17 6.7.6.2 p2, p5) and builds, and an inner one with
        // an untyped part is refused like an outer one. Mutation: ask
        // `adjusted_without_loss` instead; the first is refused. Mutation:
        // ask only the outermost length; the second builds. And a
        // declaration that declares nothing but defines a struct is walked
        // through its specifiers. Mutation: walk only declarators; the third
        // builds. Found by review.
        an_inner_variable_length_in_a_file_scope_function_pointer_builds: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_untyped_part_of_an_inner_length_in_a_file_scope_function_pointer_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_untyped_part_of_a_length_in_a_struct_member_s_prototype_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        // A length behind a pointer in a parameter, a prototype only the
        // return type reaches, and one fault in a struct two declarators
        // share, which is reported once. Mutation: have the length check stop
        // at a pointer; the first builds. Mutation: have the walk not enter a
        // function's return type; the second builds. Mutation: walk without
        // the set of types already walked; the third reports twice. Found by
        // review.
        an_untyped_part_of_a_length_behind_a_pointer_in_a_prototype_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_untyped_part_of_a_length_in_a_prototype_a_return_type_holds_is_refused: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
        an_untyped_part_of_a_length_in_a_struct_two_declarators_share_is_reported_once: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
        // A local array converts to the address of its first element and an
        // array reached through a pointer to a copy of that pointer (#621).
        // Mutation: decay `*pa` as `Address` of it with an index appended;
        // this changes.
        an_array_converts_to_the_address_of_its_first_element_or_a_copy_of_its_pointer: ["--emit", "safety-ir", "--safety", "off", "--target", "x86_64-pc-windows-msvc"],
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
        // A local the source wrote ends its line with the name it was written
        // with, and only that one: `g`'s parameter in a declaration with no
        // name has none, `p` and both `n`s have theirs, and every temporary,
        // the call's result among them, has none. The two `n`s stay `_2` and
        // `_4`, so the name is not the identity. Mutation: drop the name from
        // the printer; the named lines lose it. Mutation: have the call's
        // temporary pushed with a span; it gains a name the source never
        // wrote. Mutation: push a block's object with `push_local`; both `n`s
        // lose theirs. Mutation: drop a parameter's name in
        // `Lowering::signature`; `p` loses its.
        a_local_is_named_as_the_source_wrote_it: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
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
    // The input, the header it may include, and the three streams.
    const EXPECTED: [&str; 5] = ["c", "h", "stdout", "stderr", "exit"];

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
/// The compiler is run **from** a directory of its own, [`work_dir`], holding a
/// copy of the case's input, and handed a bare file name, because it echoes
/// back the path it was given: `safec --emit
/// tokens tests/cases/frontend/add.c` prints
/// `tests/cases/frontend/add.c:3:1 keyword "int"`, while the same
/// run from inside the directory prints `add.c:3:1 keyword "int"`. That is
/// what makes an expected file mean the same thing on every machine, and it is
/// why passing a path here would silently break every case that reports a
/// position.
///
/// **Not from the corpus**, because a run may write a file: `-o` names one and
/// `--emit executable` writes one without being asked to. Run from the group's
/// directory, that file landed beside the expected files, where the stray
/// guard reported it as a file nobody listed, on whichever run happened to
/// walk the directory after the write, and `SAFEC_BLESS=1` could not clear it.
/// The directory is removed before the outputs are compared, so a case that
/// fails leaves nothing behind either. See ADR-0007.
///
/// `--color never` is passed rather than relied on. `ColorMode::Auto` resolves
/// against whether the stream is a terminal, and a test whose meaning depends
/// on not being one changes meaning when somebody runs it differently.
fn run_case(group: &str, name: &str, args: &[&str]) {
    let input = format!("{name}.c");
    let dir = work_dir(group, name);
    // **Refused before anything is removed**, because this function removes
    // `dir` twice and a `dir` inside the corpus would be the corpus: having
    // `work_dir` return the group's directory deleted every group of it, while
    // `a_case_is_run_from_a_directory_outside_the_corpus`, which reports the
    // same mistake, ran in parallel and too late to stop it. A panic naming
    // the path is what that mistake has to cost instead.
    assert!(
        !dir.starts_with(cases_dir()),
        "case `{name}`: refusing to run from, and remove, {}, which is inside the corpus",
        dir.display()
    );
    // One of this name can only be an earlier process's that had this pid and
    // was killed before it removed it, so it is stale. A killed run's own
    // directories stay in the temporary directory under its pid, which no
    // later run looks at.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a case's working directory can be made");
    // Only where there is one: a case may have no input on purpose, and
    // `a_path_that_names_nothing_is_reported_in_this_compiler_s_words` is the
    // one whose absence is what it tests. Run from the group's directory,
    // that case met a missing file too.
    //
    // The header goes with it, so that `#include "{name}.h"` finds it beside
    // the input the way it would beside any other.
    for file in [input.clone(), format!("{name}.h")] {
        let source = group_dir(group).join(&file);
        if source.exists() {
            std::fs::copy(&source, dir.join(&file))
                .unwrap_or_else(|error| panic!("case `{name}`: `{file}` cannot be read: {error}"));
        }
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_safec"));
    command
        .current_dir(&dir)
        .args(["--color", "never"])
        .args(args)
        .arg(&input);

    // **Asked of the command, not of `work_dir`**, because it is the directory
    // the compiler really runs in that decides where a file it writes lands:
    // `.current_dir(group_dir(group))` passed every test that asked
    // `work_dir`, left `out.tok` beside the expected files, and failed only the
    // stray guard, under its own name, on every run until somebody deleted the
    // file. Asked here, before the run, it fails this case by name and writes
    // nothing. A comparison of the corpus before and after the run cannot do
    // that: cases run in parallel, so it blames whichever case of the group
    // looked after another one wrote.
    let runs_in = command
        .get_current_dir()
        .unwrap_or_else(|| panic!("case `{name}`: the compiler would run where this test does"));
    assert!(
        !runs_in.starts_with(cases_dir()),
        "case `{name}`: the compiler would run in {}, inside the corpus",
        runs_in.display()
    );

    let output = command
        .output()
        .expect("the compiler binary was built for this test");

    // Before comparing, so that a case whose outputs differ removes it too.
    // Best effort: a directory that cannot be removed is a temporary file, and
    // failing the case for it would report the wrong thing. Nothing holds that
    // this runs, or the removal before the run: a directory left behind is in
    // the temporary directory, where no case reads it.
    let _ = std::fs::remove_dir_all(&dir);

    let code = output
        .status
        .code()
        .unwrap_or_else(|| panic!("case `{name}`: the compiler was killed by a signal"));

    check(group, name, "stdout", &output.stdout);
    check(group, name, "stderr", &output.stderr);
    check(group, name, "exit", &expected_exit(code));
}

/// Where one group's cases live.
fn group_dir(group: &str) -> PathBuf {
    cases_dir().join(group)
}

/// Where one case is run: a directory of its own outside the corpus, named by
/// this process and the case's place in the table.
///
/// Its own per case, because the harness runs cases in parallel and two runs
/// sharing a directory would read each other's files, and per process, so
/// that two `cargo test` runs at once do not either. **The place rather than
/// the name**, which is unique too, since one name in two groups is `E0428`:
/// the longest name is over a hundred characters, the input inside the
/// directory spells it again, and a working directory on Windows has to stay
/// under about 258 characters, so a name in the path would fail a case for
/// its length with a message about a directory. The place is also free of the
/// separator a name could contain, where `{group}_{name}` would give group
/// `a_b` with case `c` the directory of group `a` with case `b_c`.
fn work_dir(group: &str, name: &str) -> PathBuf {
    let place = CASES
        .iter()
        .position(|&case| case == (group, name))
        .unwrap_or_else(|| panic!("case `{name}` is not in group `{group}` of the table"));
    std::env::temp_dir().join(format!("safec-case-{}-{place}", std::process::id()))
}

/// A case is run from outside the corpus, so a file it writes cannot land
/// beside the expected files.
///
/// Asked of every case the table names rather than of one, because the
/// directory is a function of the group and the case.
///
/// Mutation: have `work_dir` return `group_dir(group)`. This fails, by name and
/// on the first run, and so does every case, at the refusal in `run_case`,
/// which is what keeps the mistake from removing the corpus. Without that
/// refusal the harm would surface only as
/// `every_file_in_the_corpus_belongs_to_a_case_in_the_table` failing on
/// whichever run walked the directory after
/// `a_case_that_writes_a_file_leaves_none_in_the_corpus` wrote into it.
#[test]
fn a_case_is_run_from_a_directory_outside_the_corpus() {
    let corpus = cases_dir();
    for (group, name) in CASES {
        let dir = work_dir(group, name);
        assert!(
            !dir.starts_with(&corpus),
            "case `{name}` would run from {}, inside the corpus",
            dir.display()
        );
    }
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
