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
    /// `clang` promotes them, but an `unsigned` one as wide as `int` not;
    /// two declarators sharing one declaration, each with its width; ones
    /// inside an anonymous struct; and a function after one that reaches a
    /// bit-field writing only its own local (#1206). Mutations: promote at
    /// `<=` in `Checker::read_as`, read no width after a second declarator
    /// in `Parser::members`, or stop at the first step of an anonymous
    /// member's path in `Lowering::member`; each fails this. Mutation: drop the shift
    /// in `Lowering::extract`; the reads come out of the wrong bits and this
    /// fails.
    a_bit_field_is_read_and_written_within_its_unit,
    /// A flexible array member takes no bytes of its struct beyond the
    /// padding to its elements' alignment, and its elements, written through
    /// `->` and through a pointer that walks them, are the bytes allocated
    /// past it (#1208). Mutation: size it as one element in
    /// `Checker::struct_layout`; `sizeof` is 8 and this fails.
    a_flexible_array_member_holds_what_was_allocated,
    /// Variable length arrays sized when they run, `sizeof` included, one
    /// passed on as a pointer, and one made afresh on each of 200000 passes
    /// of a loop, its storage given back at the end of each (#1209).
    /// Mutation: skip the restore at `StorageDead` in the backend; the loop
    /// keeps every pass's 4096 bytes and the program dies of a stack
    /// overflow (`0xC00000FD` on Windows).
    a_variable_length_array_is_sized_when_declared,
    /// `<`, `<=`, `>` and `>=` of pointers into one array, each answered
    /// both ways, and a walk up to one past its end, built and run (#1211).
    /// Mutation: put back the backend's refusal of a relational comparison
    /// of two pointers; this is refused with `SC0801`.
    a_comparison_of_pointers_into_one_object_runs,
    /// A pointer minus a pointer is how many elements apart they are, as
    /// `ptrdiff_t`: either way round, to one past the end, and over a struct
    /// with padding (#638). Mutations: divide by 1 in `Emitter::difference`,
    /// and the counts are bytes; answer no type in `Checker::binary`, and
    /// this is refused.
    the_difference_of_two_pointers_into_one_array_is_its_subscripts,
    /// A function pointer stored in a local, an array element and a struct
    /// member, called through and by `(*f)()`, compared with a function and
    /// with null, passed and returned, and a struct passed and returned by
    /// value through one; and a designator called through `*` and `&`
    /// (#1212). Mutation: give a designator no value of its own in
    /// `Lowering::begin_value`; this does not build. The struct ABI of a call
    /// through a pointer is held by
    /// `a_struct_crosses_a_call_through_a_pointer_as_the_target_abi_says`, since
    /// Win64 passes these structs through memory however it is classified.
    a_function_pointer_is_stored_called_and_compared,
    /// `snprintf` called with an `int`, a string, a `char`, a `short`, an
    /// `unsigned char` and a `long long` past its parameters, directly and
    /// through a pointer to it, its output compared (#1213). Mutation: refuse
    /// `...` again as `SC0207`, or declare the function without it in
    /// `Lowering::standing_variadic`; this does not build. Pass a `char` and
    /// a `short` past the parameters unpromoted; this runs to the wrong
    /// result.
    a_variadic_function_is_called_with_its_arguments_promoted,
    /// A variadic function defined and its arguments read with the
    /// `__builtin_va_*` builtins: `int`s in a loop, a pointer, a `long long`,
    /// and a copy of the list that reads on (#1214). Mutation: write LLVM's
    /// `va_arg` on Windows rather than walking the list's pointer; this runs
    /// to the wrong result there, the second `int` read as 0. Refuse a
    /// variadic definition again; this does not build.
    a_variadic_function_reads_its_arguments,
    /// Function-like macros called and rescanned as C17 6.10.3 says: an
    /// argument over two lines, a call inside an argument, a parenthesised
    /// comma, a call of no arguments, an argument set off from the body's
    /// `-`, a name with no `(` left alone, `id(x)` with `#define x x+1`
    /// read as `x+1`, a call inside its own argument, an empty body, and a
    /// keyword for a parameter (#1215). Mutations: put back the refusal of a
    /// function-like `#define`, and this does not build; drop the mark on a
    /// name kept inside its own expansion, or leave it behind when the name
    /// is copied into a call, and `id(x)` is `x+1+1`; set no argument off
    /// from the body, and `NEG(-1)` is `--1`; write an argument in without
    /// replacing it first, and `WRAP(WRAP(3))` meets `WRAP` open; give an
    /// empty body no piece, and it panics; find only identifiers among the
    /// uses of a parameter, and `K`'s `int` is not replaced. Also: a name
    /// kept inside its own expansion at an offset other than 0, `id(z)`; a
    /// name ending two expansions with `(` past the outer one only, `H (0)`;
    /// an argument the body never uses, `UNUSED(FIRST(1))`; an argument's
    /// tokens kept apart, `DECL(int d = 3;)`; one set off from the body after
    /// it, `SUB(5 -)`; a macro named by a keyword inside an argument,
    /// `id(register)` with `#define register id(4)`, which must be
    /// replaced there, before `id` is open, or its own `id` is kept. A name
    /// kept inside its own expansion stays kept when a join copies it,
    /// `id2(x)` being `x + 10`. Mutation: leave its mark behind in a join;
    /// it is `x+1 + 10`. Mutations: look the mark up at offset 0, or scan the
    /// frames below outermost first, or replace an unused argument, or join
    /// an argument's tokens, or set nothing off after an argument, or offer
    /// no keyword for replacement inside an argument; each fails one.
    a_function_like_macro_is_replaced_and_rescanned,
    /// `#` and `##` make what C17 6.10.3.2 and 6.10.3.3 say: an argument
    /// spelled as written with one space where it was apart, a string or
    /// character constant in it escaped, an empty one as `""`; joins of two
    /// tokens, with an empty argument as a placemarker, chained, beside a
    /// `#`, and in an object-like macro; and the operand of `#` not
    /// replaced, `str(N)`, where `xstr(4 + N)` is (#1216). Mutations: put
    /// back the refusal of `#` and `##`, and this does not build; replace an
    /// operand of `#`, and `str(N)` is `"3"`; drop the escaping, and the
    /// `"x\n"` line does not build; join with a space between, and
    /// `C(x, y)` is two names; replace an argument only stringized, and
    /// `S(ONE(1, 2))` is refused for its count; scan a list that begins
    /// with `#` as a file, and `S`'s `#x` is a directive; ignore an
    /// object-like macro's joins, and `AB` hands `##` to the parser. Also:
    /// white space as C has it, not as the expansion's text spaces tokens,
    /// `XS(VERSION)` being `"1.2"` and C17 6.10.3.5 EXAMPLE 4's
    /// `xstr(INCFILE(2).h)` being `"vers2.h"`; an operand of `##` not
    /// replaced, `C(N, 1)`; operands of several tokens, `C(1 + 1, 0 + 2)`;
    /// two empty ones, `C(,)`; and a join set off from the `-` before it,
    /// `M(-, -)` being `v - --w`. Mutations: read every space of the text
    /// as white space, and `XS(VERSION)` is `"1 . 2"`; replace an operand of
    /// `##`, and `C(N, 1)` is `31`; drop an operand's tokens after the
    /// first, or write them touching, and `C(unsigned u, v = 3;)` declares one name;
    /// leave a piece where a join made nothing, and `CX(,)` panics; write a
    /// join touching the list, and `M` is `v---w`.
    stringizing_and_pasting_make_what_c_says,
    /// `...` and `__VA_ARGS__` (C17 6.10.3.1 p2): the arguments past the
    /// named ones, commas and all, replaced; left out entirely; GNU's
    /// `, ## __VA_ARGS__` dropping its comma where they are left out; C23's
    /// `__VA_OPT__` writing its content only where they replace to a token,
    /// and `#__VA_OPT__` stringizing it (#1217). Mutations: put back the
    /// refusal of `...`, and this does not build; keep the comma where the
    /// arguments are left out, and `L(1)` is refused; split the variable
    /// arguments at their commas, and `REST(0, 4, 5)` has too many; write
    /// `__VA_OPT__`'s content whatever the arguments are, and `O(1, EMPTY)`
    /// passes a comma and nothing; stringize its text's spaces as white
    /// space, and `SO2(1)` is `"( 1 )"`; write nothing for `#` on an empty
    /// one, and `SO()` is no expression.
    a_variadic_macro_takes_what_is_past_its_parameters,
    /// A struct a function reaches only through pointers is laid out where
    /// a pointer to it is stepped or subtracted, so `end - first` counts its
    /// elements and `at + 1` steps over one, though nothing in those
    /// functions declares an object of it (#638). Mutation: skip
    /// `Lowering::lay_out_stepped`; the difference is refused and the step
    /// panics.
    a_struct_only_pointers_reach_is_stepped_and_subtracted,
    /// A typedef of a variable length array has its length worked out
    /// where the typedef is reached (C17 6.7.8 p3): not again at each
    /// object; once, however many objects are declared with it, and with
    /// none; by `sizeof` of the type; afresh on each pass of a loop; from
    /// the name in scope at the typedef; and not again through an alias, a
    /// pointer or a qualified copy of it, while every length its own
    /// declarator wrote, through pointers, nested arrays and a function's
    /// return, is (#1294). Mutations: evaluate the length at the object
    /// again, and `later` takes 7; walk no typedef in the resolver, and
    /// `scoped` takes the inner `s`; evaluate every length met, and `alias`
    /// takes 7; ask `ty` itself in the resolver's skip, and `qualified` is
    /// refused; stop at a function type, and `returning` counts 4.
    a_typedef_of_a_variable_length_array_is_sized_where_the_typedef_is,
}
