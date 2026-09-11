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
    a_block_declaration_does_not_leave_its_block: ["--emit", "ast"],
    a_comma_in_a_controlling_expression: ["--emit", "ast"],
    a_dangling_else: ["--emit", "ast"],
    a_definition_that_is_not_a_function: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_function_the_ir_cannot_hold: ["--emit", "safety-ir", "--target", "x86_64-pc-windows-msvc"],
    a_declaration_is_not_a_body: ["--emit", "ast"],
    a_failed_parse_reports_no_names: ["--emit", "ast"],
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
    // Every one of these is also in `llvm.rs`, which hands it to `clang`. The
    // text and whether the text is LLVM are two claims.
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
    array_declaration: ["--emit", "ast"],
    array_length_is_not_evaluated: ["--emit", "ast"],
    assigning_the_wrong_type: ["--emit", "ast"],
    block_declaration: ["--emit", "ast"],
    block_declaration_without_a_semicolon: ["--emit", "ast"],
    block_function_declaration: ["--emit", "ast"],
    call: ["--emit", "ast"],
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
    pipeline_not_implemented: ["--emit", "object"],
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
/// it implicit in what the expected files happen to contain. Nine cases carry a
/// `.exit` and one does not, so the rule is now demonstrated many times over and
/// written down once.
///
/// Mutation: return `Vec::new()` whatever the code. Ten tests fail: the unit
/// test below, which is the one that says what the rule is, and every case that
/// exits non-zero.
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
