//! Programs that have to build at `--safety off` and run to the exit code C
//! gives them: the "compiles to an executable and runs" half of what each
//! Systems C construct is accepted on (`docs/roadmap.md`, Phase 11).
//!
//! The corpus in `tests/cases.rs` holds what the compiler says about a
//! program. This holds what the program does once built, which no `.stderr`
//! can: a lowering that compiles a construct to the wrong code is silent
//! there and wrong here.
//!
//! **A literal table, not the files on disk** (ADR-0007), each source brought
//! in by `include_str!`, so a missing or renamed program is a build failure
//! rather than a test that runs nothing. Each program exits `0` when what it
//! computed is what C says, and something else when it is not, so the table
//! needs no expected value of its own.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Whether there is a `clang` on this machine to link a program with.
fn clang_is_here() -> bool {
    Command::new("clang")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Build `source` at `--safety off` into an executable and run it; it has to
/// build and exit `0`. Skipped with a note where there is no `clang`, and a
/// failure where `SAFEC_REQUIRE_LLVM` is set, as in `tests/examples.rs`.
fn builds_and_exits_zero(name: &str, source: &str) {
    if !clang_is_here() {
        assert!(
            std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
            "SAFEC_REQUIRE_LLVM is set and there is no `clang` to link a program with"
        );
        eprintln!("no `clang` on this machine: `{name}` was not run");
        return;
    }
    let dir: PathBuf = std::env::temp_dir().join(format!("safec_run_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the temporary directory is writable");
    fs::write(dir.join("program.c"), source).expect("the temporary directory is writable");
    let exe = dir.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    let built = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--safety", "off", "-o"])
        .arg(&exe)
        .arg("program.c")
        .current_dir(&dir)
        .output()
        .expect("the compiler binary was built for this test");
    assert!(
        built.status.success(),
        "`{name}` did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(&exe)
        .status()
        .expect("the program it built runs");
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(ran.code(), Some(0), "`{name}` ran to the wrong result");
}

macro_rules! programs {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {
        $(
            $(#[$doc])*
            #[test]
            fn $name() {
                builds_and_exits_zero(
                    stringify!($name),
                    include_str!(concat!("run/", stringify!($name), ".c")),
                );
            }
        )*
    };
}

programs! {
    /// A typedef name for a scalar, a pointer and an unsigned type, in a
    /// parameter, a cast, `sizeof` and a `for`, and an object in an inner
    /// block that hides one (#1205). Mutation: stop declaring typedef names in
    /// `Parser::declare_declarators`; this does not build.
    a_typedef_names_the_type_it_was_declared_with,
    /// A typedef in a block that hides the file's, an object that hides it
    /// from the end of its own declarator (C17 6.2.1 p7), so `int T =
    /// sizeof(T);` measures the `int`, and a label spelled like it, which has a
    /// name space of its own (#1205 review). Mutation: declare a name after
    /// its initializer in `Parser::init_declarator_list`; this exits 1.
    a_typedef_in_a_block_is_scoped_as_c_says,
    /// An enumeration's constants with values implicit, written, and counted
    /// on from a written one, negative, in a `switch`, an array length and
    /// `sizeof`, one in a block that hides a file's and an object that hides
    /// one (#1204). Mutation: count the first of a list from `1` in
    /// `Checker::decide_enumerations`; this exits 1.
    an_enum_gives_each_enumerator_its_value,
    /// `static`, `const` and `restrict` in a parameter's brackets, in each
    /// order C17 6.7.6.2 p1 allows, and on the outer array of an array of
    /// arrays (#1210). Mutation: refuse them again in `Parser::suffixes`;
    /// this does not build.
    a_static_length_in_an_array_parameter_is_read,
    /// A struct local, a global and a heap one, read and written through `.`
    /// and `->`, a pointer member, an array member, a member's address and
    /// `sizeof`, laid out by the IR with padding after a `char` (#1200).
    /// Mutation: take every member's offset as zero in the backend's
    /// `Projection::Field` arm; this exits with another value.
    a_struct_is_laid_out_and_read_through_its_members,
    /// `(*x).m` where `x` is read out of memory: `(**pp).m`, through a global
    /// pointer and through a pointer member, written, compound-assigned,
    /// incremented and taken the address of (#1200 review). Mutation: take
    /// `Lowering::member_base`'s temporary in `Lowering::struct_address`;
    /// this does not build.
    a_member_through_a_pointer_read_out_of_memory_is_read,
    /// A struct member of struct type and an array member of structs, reached
    /// by `.` chains and through a pointer to the inner struct, a global and
    /// a local array of structs, a list that points at itself walked to its
    /// end, a pointer to a struct nothing defines, `sizeof` of the outer, and
    /// a struct reached through a pointer before the struct holding it by
    /// value is laid out (#1202). Mutation: take every member's offset as zero in the
    /// backend's `Projection::Field` arm; this exits with another value.
    a_nested_struct_and_an_array_of_structs_are_read_by_member,
    /// Structs of each size class the C ABIs tell apart, 3, 8, 12 and 24
    /// bytes, returned and passed by value, a parameter written in the callee
    /// without reaching the caller's, and a struct assigned and copied
    /// through pointers (#1201). Mutation: pass a struct argument's own
    /// address instead of a copy's in `Task::Copied`; this fails.
    a_struct_is_assigned_passed_and_returned_by_value,
    /// A union's members share their first byte, so an `int` written is the
    /// low half of the `long long` read; a union sits in a struct as a
    /// member, and is assigned, passed and returned by value (#1203).
    /// Mutation: lay a union out as a struct in `record::lay_out_union`; the low
    /// half is read from the wrong bytes and this fails.
    a_union_holds_its_members_at_one_place,
    /// The members of an anonymous struct inside an anonymous union are
    /// their container's, at their offsets within it: `low` and `high` share
    /// `whole`'s bytes beside `kind`, read through `.` and `->` (#1207).
    /// Mutation: keep only the first step of `path` in `Lowering::member`;
    /// `high` reads the union's first bytes and this fails. Mutation: append
    /// each step in `Checker::find_member` rather than prepend it; `low`'s
    /// path is reversed and this fails.
    an_anonymous_member_is_reached_through_its_container,
    /// Bit-fields packed into their units beside a `char` and a 40-bit
    /// `long long` one, each read and written with the other bits of its
    /// unit kept, a signed one carrying its sign, an assignment's value cut
    /// to the width, a step that wraps, an `unsigned` one promoted to `int`,
    /// and `unsigned long long` and `unsigned long` ones promoted too, as
    /// `clang` promotes them; and a function after one that reaches a
    /// bit-field writing only its own local (#1206). Mutation: drop the shift
    /// in `Lowering::extract`; the reads come out of the wrong bits and this
    /// fails.
    a_bit_field_is_read_and_written_within_its_unit,
}
