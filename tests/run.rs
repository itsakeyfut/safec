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
    // A fixed time, so a program that prints `__DATE__` or `__TIME__` is
    // the same program on every run, as a build that sets it is.
    let built = Command::new(env!("CARGO_BIN_EXE_safec"))
        .env("SOURCE_DATE_EPOCH", "0")
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
    /// `id2(x)` being `x + 10`, and when `__VA_OPT__` writes it, `POPT(x)`.
    /// Mutations: leave its mark behind in a join, or in `__VA_OPT__`'s
    /// text; it is `x+1 + 10`, or `x+1+1`. Mutations: look the mark up at offset 0, or scan the
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
    /// Variadic macros stringized against what `clang 22.1.8 -E` writes for
    /// each, every expected string taken from its output: GNU's comma with
    /// its arguments as written (`F(F(1), F(2,3))` keeps the inner `F` as
    /// `F`, C17 6.10.3.1 p1), left out with the white space before it,
    /// given empty, joined on (`z(a , b1)`); the white space of an
    /// expansion to nothing kept before the next token (`[ E]` is `[ ]`);
    /// `__VA_OPT__` with a named parameter, with `#`, `##` and GNU's comma
    /// inside, of white space alone, empty (`EV(1)` panicked once). Found
    /// by review. Mutations: replace the comma's arguments first; keep the
    /// space before a dropped comma; read a longer comma run as a plain
    /// join; drop an empty expansion's white space; replace no argument
    /// `__VA_OPT__` uses alone, or not its own; shift no edit but a
    /// parameter's into `__VA_OPT__`; write a piece for content of white
    /// space; read `x ## __VA_ARGS__` as GNU's comma; each changes a string
    /// or panics.
    variadic_macros_write_what_clang_writes,
    /// `#if` and `#elif` choose by their expressions (C17 6.10.1, #1218):
    /// `defined` both ways, a function-like macro, an `#elif` after a group
    /// not taken, `-1 > 0u` under the usual conversions, `0 && (1 / 0)` not
    /// dividing, a name left as 0, an `#elif` after a taken group not
    /// evaluated, the operators and their precedence, `?:`, character and
    /// octal, hex and unsigned constants, and an `#if` inside a skipped
    /// group not evaluated. Mutations: put back the refusal of `#if`, and
    /// this does not build; compile no group of an `#if`, and `taken` is 0;
    /// compare `-1 > 0u` as signed; evaluate the right
    /// of `&&` always, or an `#elif` after a taken group; each fails a bit.
    /// Also, in `more`, each relational operator both ways, the truth
    /// tables of `||` and `&&`, short circuits through `||` and `?:` that
    /// must not refuse, precedence written without parentheses, every
    /// unsigned operator, a decimal constant past `intmax_t`, `'\x7f'`, and
    /// `#elif 0` then `#else`. Mutations: drop the equal half of `<=` or
    /// `>=`, swap two precedence levels, evaluate a dead arm of `?:` or
    /// `||`, compute an unsigned operator as signed or its result as
    /// signed; each sets a bit.
    an_if_chooses_by_its_constant_expression,
    /// `__DATE__` and `__TIME__` under `SOURCE_DATE_EPOCH=0`, `__LINE__`
    /// written in an argument, in a list called over three lines, in an
    /// object-like list, in an argument's macro, through a macro whose list
    /// calls another, and copied by `##` from an argument of a call over
    /// three lines, which is the line of its `)`, and `__FILE__` (#1219,
    /// ADR-0130). Mutations: put
    /// back the refusal of a dynamic macro, and this does not build; take the
    /// clock where `SOURCE_DATE_EPOCH` is set, and `__DATE__` is not 1970;
    /// take the line of a call's name, or of an argument's call, rather than
    /// of its `)` or where the argument was written; each fails a bit.
    the_place_and_time_of_a_use_are_what_clang_writes,
    /// The pragmas ADR-0129 ignores, by the directive, by `_Pragma` and by
    /// `_Pragma` a macro writes, change nothing; `#warning` does not stop
    /// the build, nor `#error` in a group that is skipped; and `#line`,
    /// macro-replaced, renumbers `__LINE__` and names `__FILE__`, a later
    /// one without a name keeping it, and a name's `\` read as one
    /// (#1220); and `_Pragma` whose operand a macro writes, or whose name an
    /// alias does (#1345). Mutations: refuse an ignored pragma, or report `#warning`
    /// as an error, and this does not build; leave `#line` unread, or
    /// count from its own line, and a bit fails.
    pragmas_and_line_are_read_as_c_says,
    /// A call whose `(` lies past the expansion its name came out of,
    /// through one alias and two, inside an argument and as one, past an
    /// expansion that handed nothing over; and one whose `)` lies past the
    /// expansion its arguments began in, C17 6.10.3.5 EXAMPLE 3's `h 5)`
    /// shape (#1336). Mutations: answer `Kept` where the name's frame ends,
    /// or stop collecting where the arguments' frame ends, and this does
    /// not build.
    a_call_read_past_an_expansion_is_what_clang_writes,
    /// GNU line markers, macro-replaced, renumber `__LINE__` and name
    /// `__FILE__` as `#line` does, from line `0`, with flags `1` to `4` read
    /// and ignored and a marker with no name keeping the last (#1346).
    /// Mutations: refuse line `0`, refuse flags, or leave a marker unread;
    /// this does not build or a bit fails.
    line_markers_are_read_as_clang_reads_them,
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
    /// An array of `char`, `unsigned char` and `signed char` from a string
    /// literal, in braces or not, unsized, exactly as long as its characters
    /// and longer, at block and file scope, two of one typedef with two
    /// lengths, and a static one written to (#1244). Mutations: put the
    /// `SC0304` refusal back in `Checker::check_initialized`, and this does
    /// not build; copy one byte fewer in `Lowering::store_string`, and it
    /// fails; write a static's bytes as a private constant in
    /// `Emitter::global`, and the write faults.
    a_char_array_takes_its_length_and_bytes_from_a_string_literal,
    /// The bytes of a local array past its string literal are zero, not what
    /// the stack held before (C17 6.7.9 p21), however dirty the frame
    /// before it left them (#1244). Mutation: leave out the `memset` in
    /// `Lowering::store_string`; this exits with the stale bytes' sum.
    a_char_array_longer_than_its_string_literal_is_zero_filled,
    /// A program's own `static` function named `memset` and `static` object
    /// named `memcpy`, which C17 7.1.3 p1 allows without the header, are
    /// neither called nor named by an initializer's fill and copy (#1244
    /// review). Mutations: take the program's `memset` in
    /// `Lowering::library_own`, or leave the program's name alone in
    /// `Emitter::name`, and this fails.
    an_initializer_calls_no_memset_the_program_has,
    /// Braced lists for arrays, structs, nested ones, a union, bit-fields
    /// and a scalar, with brace elision, an unsized array, rows of string
    /// literals, a struct taken whole, a `const` local, the rest zero over
    /// a dirtied frame, at block and file scope and in a block's `static`
    /// (#1241). `{ "q" }` for a struct is a list whose first member the
    /// literal initializes, not the literal (#1244 review). Review of #1241
    /// added: a union's bytes past its first member and a struct's padding
    /// zeroed over a dirtied frame, an unnamed bit-field skipped, a
    /// flexible array member past the list, unsized arrays whose items
    /// elide braces or are whole structs, a row's literal in braces, and a
    /// static's zero bit-field and literal inside a struct. Mutations:
    /// refuse a `{` in `Parser::initializer`, or answer `true` in
    /// `Parser::takes_braced_literal`, and this does not build; take an
    /// expression whole for an aggregate subobject in `Shapes::fill`, or
    /// put every member at index zero in `Lowering::static_init`, zero a
    /// local without a `memset` where its scalars are all written in
    /// `Lowering::store_list`, count every item as an element in
    /// `Shapes::extent`, or drop the braced literal in `Shapes::object`,
    /// and it fails.
    a_braced_initializer_fills_arrays_and_structs,
    /// Statics that start as addresses: string literals in a table, one
    /// plus a number, `&g`, an element's, an array plus a number, a
    /// member's, one declared later, and a struct table of both (#1241).
    /// Mutation: answer every member at offset zero in
    /// `Lowering::member_offset`; this fails.
    a_static_holds_addresses_of_statics,
    /// Designators by member and by index, chained, mixed with positional
    /// items that go on from them, through an anonymous member, into a
    /// union's other member, an unsized array sized by one, an item
    /// overridden by a later one and not evaluated, a literal's byte
    /// overridden in a static, and a static of a million elements two of
    /// which are written (#1242). The review of #1242 added: a later
    /// braced list for a subobject clearing what earlier items wrote in it,
    /// a union holding the member written last, locally, as a static and
    /// beside a pointer member, an override by a prefix, a designation
    /// after a deeper one, an array sized by its largest index and not its
    /// last, and a member after an unnamed bit-field. Mutations: keep every
    /// leaf in `Shapes::resolve`, cover only an equal path there, keep a
    /// union's earlier member, reset no frame on a designation in
    /// `Shapes::list`, size by the last index in `Shapes::place`, answer a
    /// member's index for its position in `Shapes::find`, or zero a union's
    /// first member over another in `Lowering::store_list`, and this fails;
    /// the sparse aggregate is held by `an_address_initializes_a_static`.
    a_designated_initializer_names_its_subobject,
    /// Compound literals: a struct's and an array's in a block, made again
    /// each time a loop reaches one, a scalar's, one under `.` and `[]`,
    /// one passed to a function and one under `sizeof`, and at file scope
    /// an array's, a struct's address and a table of strings, and arrays of
    /// unknown size completed by a braced string, a designator and brace
    /// elision (C17 6.5.2.5, #1243). Mutations: initialize a block's once,
    /// not where evaluated, in `Lowering::compound`, and this exits 1; read
    /// no `{` after a type name in `Parser::cast`, or complete a braced
    /// string from the items' count in `Parser::compound_literal`, and it
    /// does not build; count no compound literal's list in
    /// `counted_lengths`, and it does not build either.
    a_compound_literal_is_an_unnamed_object,
}
