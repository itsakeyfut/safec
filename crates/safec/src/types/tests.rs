use super::*;
use crate::ast::spell_type;
use crate::lexer::lex;
use crate::parser::parse;
use crate::sema::resolve;
use safec_ir::target::Target;

struct Checked {
    ast: Ast,
    sources: SourceMap,
    types: Types,
    diagnostics: DiagnosticSink,
}

/// Scan, parse, resolve and check, the way the driver does.
fn checked(text: &str) -> Checked {
    let mut sources = SourceMap::new();
    let file = sources.add_virtual("t.c", text);
    let mut diagnostics = DiagnosticSink::new();
    let tokens = lex(file, sources.file(file), &mut diagnostics);
    let mut ast = parse(file, &tokens, &mut diagnostics);
    assert!(
        !diagnostics.has_errors(),
        "the input did not parse: {:?}",
        diagnostics.diagnostics()
    );
    let resolution = resolve(&sources, &ast, &mut diagnostics);
    // Named rather than taken from the host: what a constant's type is
    // turns on the range of `int`, so a test that did not name a target
    // would be asserting about whichever machine ran it. `int` is 32 bits
    // on every row of `Target::ALL`, which is
    // what makes one triple enough here.
    let target = Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple");
    let types = check(
        &sources,
        &mut ast,
        &resolution,
        target.int(),
        &mut diagnostics,
    );

    Checked {
        ast,
        sources,
        types,
        diagnostics,
    }
}

impl Checked {
    /// How the expression written as `text` is spelled as a type.
    ///
    /// Found by its own text among the expressions, so a test names one
    /// the way a reader would. It must be the only expression written
    /// that way: `x` in this file is two of them, and a helper that
    /// quietly took the first would let a row ask about something other
    /// than what it says. A declarator is not an expression, so the `*p`
    /// in `int *p;` is not a candidate.
    ///
    /// `None` is spelled `?`, so that a row asserting silence looks
    /// different from a row asserting a type.
    fn spelling(&self, text: &str) -> String {
        let written: Vec<ExprId> = self
            .ast
            .expr_ids()
            .filter(|&id| self.sources.snippet(self.ast.expr(id).span()) == text)
            .collect();

        let [id] = written[..] else {
            panic!("{text:?} is {} expressions, not one", written.len());
        };

        match self.types.of(id) {
            Some(ty) => spell_type(&self.sources, &self.ast, ty),
            None => "?".to_owned(),
        }
    }

    /// What the constant written as `text` is worth.
    ///
    /// Found the same way and under the same rule as `spelling`: it has
    /// to be the only expression written that way.
    fn value_of(&self, text: &str) -> Option<i128> {
        let written: Vec<ExprId> = self
            .ast
            .expr_ids()
            .filter(|&id| self.sources.snippet(self.ast.expr(id).span()) == text)
            .collect();

        let [id] = written[..] else {
            panic!("{text:?} is {} expressions, not one", written.len());
        };

        self.types.value(id)
    }

    fn messages(&self) -> Vec<&str> {
        self.diagnostics
            .diagnostics()
            .iter()
            .map(Diagnostic::message)
            .collect()
    }

    fn codes(&self) -> Vec<&str> {
        self.diagnostics
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code().map(Code::as_str))
            .collect()
    }

    /// Every label's message, so that a test can hold what a caret says
    /// and not only which code said it.
    fn labels(&self) -> Vec<&str> {
        self.diagnostics
            .diagnostics()
            .iter()
            .flat_map(Diagnostic::labels)
            .map(Label::message)
            .collect()
    }

    fn notes(&self) -> Vec<&str> {
        self.diagnostics
            .diagnostics()
            .iter()
            .flat_map(Diagnostic::notes)
            .map(String::as_str)
            .collect()
    }
}

/// One program per spelling, so that a row is about one constant.
fn returning(spelling: &str) -> Checked {
    checked(&format!("int f(void) {{\n    return {spelling};\n}}\n"))
}

/// An integer constant is worth what its base says, C17 6.4.4.1 p1.
///
/// The values are written out rather than computed from the spellings,
/// because a table that works the expectation out the
/// way the code does agrees with the code however wrong both are.
///
/// Every row is unsuffixed, because a suffix is refused rather than read:
/// `a_suffix_asks_for_a_type_this_compiler_does_not_have` is that half.
///
/// Mutation: give the hexadecimal arm radix ten. `0x10` is ten and this
/// fails. Mutation: give the octal arm radix ten. `010` is ten and this
/// fails, along with most of the suite, because `0` stops being readable
/// too.
#[test]
fn an_integer_constant_is_worth_what_its_base_says() {
    for (spelling, value) in [
        ("0", 0),
        ("00", 0),
        ("16", 16),
        ("010", 8),
        ("0777", 511),
        ("0x10", 16),
        ("0X10", 16),
        ("2147483647", 2147483647),
    ] {
        let checked = returning(spelling);
        assert_eq!(checked.messages(), Vec::<&str>::new(), "{spelling}");
        assert_eq!(checked.value_of(spelling), Some(value), "{spelling}");
        assert_eq!(checked.spelling(spelling), "int", "{spelling}");
    }
}

/// Every suffix C17 6.4.4.1 p1 allows is refused, and refused as a type
/// this compiler lacks rather than as a spelling C lacks.
///
/// The whole table is walked, written out as literals, because a suffix
/// arm is the contents of a table and nothing else checks them: a typo
/// turning `ULL` into `ULK` makes `1ULL` a malformed spelling rather than
/// an unsupported type, which is this compiler blaming the program for
/// its own gap. Measured: that typo passed the entire suite before this
/// test existed.
///
/// **Refused rather than read.** A suffix decides what arithmetic on the
/// constant means. With `3u` read as an `int`, `-6 / 3u` compiles to a
/// signed division, and C17 6.3.1.8 makes it an unsigned one whose answer
/// is 1431655763 rather than -2. Measured against `clang 20.1.6 -std=c17
/// -pedantic-errors`, whose constant evaluator agrees.
///
/// Mutation: read a suffixed constant as an `int` instead of answering
/// `Reading::Suffixed`. Every row stops being reported and this fails.
#[test]
fn a_suffix_asks_for_a_type_this_compiler_does_not_have() {
    for suffix in [
        "u", "U", "l", "L", "ll", "LL", "ul", "uL", "Ul", "UL", "lu", "lU", "Lu", "LU", "ull",
        "uLL", "Ull", "ULL", "llu", "llU", "LLu", "LLU",
    ] {
        let spelling = format!("1{suffix}");
        let checked = returning(&spelling);
        assert_eq!(
            checked.messages(),
            ["this compiler has no type for a suffixed constant"],
            "{spelling}"
        );
        assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
        assert_eq!(checked.value_of(&spelling), None, "{spelling}");
    }

    // The zero of each base with a suffix on it, because those are the
    // ones `a_pointer_takes_a_zero_and_not_a_one` used to accept as null
    // pointer constants and no longer does.
    for spelling in ["0L", "0u", "0UL", "0x0u"] {
        let checked = returning(spelling);
        assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
    }
}

/// `e` is a hexadecimal digit before it is an exponent marker.
///
/// The half of `read` a reader would get wrong: 6.4.4.2 p1 spells a
/// decimal's exponent `e` and a hexadecimal's `p`, so the base has to be
/// settled before the marker is looked for. Both directions are here,
/// because a rule that only refused would pass against one that refused
/// everything.
///
/// Mutation: look for `e`/`E` in a hexadecimal constant too, by giving
/// both arms the same markers. `0xe1` is reported as a floating constant
/// and this fails, along with
/// `a_floating_constant_is_reported_as_one_this_compiler_does_not_read_yet`,
/// whose `0x1p3` row stops being read as one.
#[test]
fn a_hexadecimal_digit_e_is_not_an_exponent() {
    let checked = returning("0xe1");
    assert_eq!(checked.messages(), Vec::<&str>::new());
    assert_eq!(checked.value_of("0xe1"), Some(225));

    for spelling in ["1e5", "0e1", "0x1p3"] {
        let checked = returning(spelling);
        assert_eq!(
            checked.messages(),
            ["this compiler does not read floating constants yet"],
            "{spelling}"
        );
    }
}

/// A value outside `int` is reported rather than wrapped, C17 6.4.4 p2.
///
/// The first is a constant `clang 20.1.6 -std=c17 -pedantic-errors
/// --target=x86_64-pc-windows-msvc` accepts, which is why the message
/// blames this compiler rather than the program: what is missing is
/// `long`, not a well-formed constant. `docs/frontend.md` carries the
/// divergence.
///
/// **The last two are the two ways the carrier itself can be overrun, and
/// each is here for one line.**
///
/// `u128::MAX` written out is what `read_number` returns when the
/// accumulation lands exactly on the top. Converted with `as` rather than
/// asked whether it fits an `i128` it is -1, which `int` holds, so the
/// constant would be accepted silently as minus one.
///
/// `2^128` is one more, and it is congruent to zero. A wrapping
/// accumulator reaches the end of it holding zero, which `int` also
/// holds, so the constant would be accepted silently as nought. The forty
/// nines cannot catch that: they wrap to a number still outside `int`, so
/// they are reported either way. Both are the `010`-lowers-to-ten shape,
/// a plausible wrong number rather than a refusal.
///
/// Mutation: answer `true` from `Integer::holds`. The first is accepted
/// with a value `int` does not hold and this fails. Mutation: accumulate
/// with `wrapping_mul` and `wrapping_add`. `2^128` becomes zero, nothing
/// is reported, and this fails. Mutation: replace
/// `i128::try_from(value).ok()` with `Some(value as i128)`. `u128::MAX`
/// becomes `Constant -1` with nothing reported and this fails.
#[test]
fn a_constant_no_type_here_can_hold_is_reported_and_has_no_value() {
    for spelling in [
        "2147483648",
        "9999999999999999999999999999999999999999",
        "340282366920938463463374607431768211455",
        "340282366920938463463374607431768211456",
    ] {
        let checked = returning(spelling);
        assert_eq!(
            checked.messages(),
            ["no integer type this compiler has can hold this constant"],
            "{spelling}"
        );
        assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
        assert_eq!(checked.value_of(spelling), None, "{spelling}");
        assert_eq!(checked.spelling(spelling), "?", "{spelling}");
    }

    // The edge is the edge: `int` is 32 bits on the named target, so one
    // less than the reported value is an ordinary constant.
    let checked = returning("2147483647");
    assert_eq!(checked.messages(), Vec::<&str>::new());
    assert_eq!(checked.value_of("2147483647"), Some(2147483647));
}

/// A floating constant is this compiler's gap, not the program's fault.
///
/// `1.5` is typed `int` today with nothing said, which is the silent
/// wrong answer this replaces.
///
/// Mutation: report a floating constant as `TOO_LARGE`. The message is
/// about a width rather than about a type that does not exist, and this
/// fails. Mutation: change `FLOATING`'s label. The label assertion below
/// fails, and nothing else in the suite does, which is why the label is
/// asserted here rather than left to a corpus case that does not exist.
#[test]
fn a_floating_constant_is_reported_as_one_this_compiler_does_not_read_yet() {
    for spelling in [
        "1.5", "0.0", ".5", "1.", "1e5", "1E+5", "0x1p3", "1.5f", "0x1.8p0",
    ] {
        let checked = returning(spelling);
        assert_eq!(
            checked.messages(),
            ["this compiler does not read floating constants yet"],
            "{spelling}"
        );
        assert_eq!(checked.codes(), ["SC0305"], "{spelling}");
        assert_eq!(
            checked.labels(),
            ["this is a floating constant"],
            "{spelling}"
        );
        assert_eq!(checked.value_of(spelling), None, "{spelling}");
    }
}

/// A spelling with a `.` or an exponent that C17 6.4.4.2 p1 does not
/// spell that way is the program's fault, not this compiler's gap.
///
/// The distinction the whole two-code arrangement rests on, at the one
/// place it is easiest to lose: `1.5` is a constant C gives a type to and
/// this compiler has none for, so `SC0305` is honest; `1e` is not a
/// constant at all, and telling its author that floating constants are
/// unsupported would be a false reason for a true refusal.
///
/// Every row is an error under `clang 20.1.6 -std=c17 -pedantic-errors
/// --target=x86_64-unknown-linux-gnu`, which reports `1e` as "exponent
/// has no digits" and `0x1.8` as "hexadecimal floating constant requires
/// an exponent". Measured, not recalled.
///
/// Mutation: answer `Reading::Floating` for anything with a `.` or an
/// exponent marker, without checking the grammar. Every row starts
/// reporting `SC0305` and this fails.
#[test]
fn a_spelling_that_only_looks_like_a_floating_constant_is_the_programs_fault() {
    for spelling in [
        "1e", "1E+", "1e-", "0x1p", "0x1.8", "1.2.3", "0xp1", "1.5ll",
    ] {
        let checked = returning(spelling);
        assert_eq!(
            checked.messages(),
            ["this is not a constant C allows"],
            "{spelling}"
        );
        assert_eq!(checked.codes(), ["SC0106"], "{spelling}");
    }
}

/// A spelling that is no constant at all is the program's fault.
///
/// Each row was measured against `clang 20.1.6 -std=c17
/// -pedantic-errors --target=x86_64-pc-windows-msvc`, which reports every
/// one of them as an error; `-pedantic-errors` is the flag that makes the
/// answer C's rather than clang's.
///
/// Mutation: accept any suffix. `1lL`, `1uu`, `123abc` and `0b101` stop
/// being reported and this fails. Mutation: answer `NOT_A_CONSTANT` for
/// an octal digit out of range. `09` keeps its code and loses its label,
/// and this fails along with the corpus case
/// `a_spelling_that_is_not_a_constant`, whose blessed stderr holds the
/// same label. Two guards, and both are about the label rather than the
/// code, which is what says the label is guarded at all.
#[test]
fn a_spelling_that_is_not_a_constant_is_the_programs_fault() {
    for (spelling, label) in [
        (
            "123abc",
            "this is not a digit of the constant's base, and not a suffix C allows",
        ),
        (
            "1lL",
            "this is not a digit of the constant's base, and not a suffix C allows",
        ),
        (
            "1uu",
            "this is not a digit of the constant's base, and not a suffix C allows",
        ),
        (
            "0b101",
            "this is not a digit of the constant's base, and not a suffix C allows",
        ),
        ("09", "an octal constant has digits `0` to `7`"),
        (
            "0x",
            "a hexadecimal constant needs at least one digit after the `0x`",
        ),
    ] {
        let checked = returning(spelling);
        assert_eq!(
            checked.messages(),
            ["this is not a constant C allows"],
            "{spelling}"
        );
        assert_eq!(checked.codes(), ["SC0106"], "{spelling}");
        assert_eq!(checked.labels(), [label], "{spelling}");
    }
}

/// What each shape of expression is worth, spelled the way C declares it.
///
/// The spellings are written out rather than derived from the types,
/// because a table built the way the code builds one compares the code
/// with itself.
///
/// Mutation: give `UnOp::AddrOf` the operand's type rather than a pointer
/// to it, or `UnOp::Deref` the operand's. Either fails. Mutation: type
/// `BinOp::Add` as `int` whatever its operands are; `p + 1` stops being a
/// pointer and this fails.
#[test]
fn every_shape_of_expression_has_the_type_c_gives_it() {
    let checked = checked(
        "int f(int a) { return a; }\n\nint main(void) {\n    int x;\n    char c;\n    int *p;\n    int *r;\n    int q[3];\n    p = &x;\n    q[0] = *p + f(2) + x++;\n    p = p + 1;\n    p = q + 1;\n    p++;\n    -c;\n    p - r;\n    1 - p;\n    p - 1;\n    return 0;\n}\n",
    );

    // `1 - p` is the one expression here C refuses, and it is here for
    // its type rather than for the report.
    assert_eq!(checked.messages(), ["`-` cannot take `int` and `int *`"]);

    for (text, spelling) in [
        // An array's length is an expression like any other, and is one
        // no walk of the statements would reach.
        ("3", "int"),
        ("2", "int"),
        ("&x", "int *"),
        ("*p", "int"),
        ("q[0]", "int"),
        ("f(2)", "int"),
        ("x++", "int"),
        // An increment keeps its operand's type: 6.5.2.4 p2 makes `p++`
        // a pointer, and typing it `int` broke nothing before this row.
        ("p++", "int *"),
        // 6.3.1.1's promotions make a unary operator's result `int` even
        // when its operand is a `char`.
        ("-c", "int"),
        ("p + 1", "int *"),
        ("p = p + 1", "int *"),
        // 6.3.2.1 p3 converts the array first, so this is pointer
        // arithmetic rather than arithmetic on something that is not a
        // pointer. Before that conversion existed, `p = q + 1` was
        // reported against a program `clang` compiles.
        ("q + 1", "int *"),
        ("p - 1", "int *"),
        // 6.5.6 p9: a pointer minus a pointer is `ptrdiff_t`, which this
        // compiler cannot name. p3 allows the pointer only on the left,
        // so `1 - p` is not an expression C gives a type to at all.
        ("p - r", "?"),
        ("1 - p", "?"),
    ] {
        assert_eq!(checked.spelling(text), spelling, "{text}");
    }
}

/// C17 6.3.2.3 p3 lets a pointer take a null pointer constant and nothing
/// else that is an integer.
///
/// Both halves in one test, because a test for the silence alone passes
/// against a compiler that checks nothing.
///
/// The suffixed zeros `0u`, `0L` and `0UL` are C's null pointer constants
/// too and are not here, because a suffix is refused before this rule is
/// reached: `a_suffix_asks_for_a_type_this_compiler_does_not_have` holds
/// that, and holds those three spellings by name so that what left this
/// list is findable from where it went.
///
/// Mutation: have `is_null_pointer_constant` answer `false` always. The
/// first program starts reporting and this fails. Mutation: have it answer
/// `true` always. The second stops and this fails.
#[test]
fn a_pointer_takes_a_zero_and_not_a_one() {
    for zero in ["0", "0x0", "0X0", "00", "000"] {
        let checked = checked(&format!(
            "int main(void) {{ int *p; p = {zero}; return 0; }}\n"
        ));
        assert_eq!(checked.messages(), Vec::<&str>::new(), "{zero}");
    }

    let checked = checked("int main(void) { int *p; p = 1; return 0; }\n");
    assert_eq!(checked.messages(), ["cannot assign `int` to `int *`"]);
}

/// Which assignments between pointers C17 6.5.16.1 p1 allows.
///
/// One program with all four, so that the three silences are asserted
/// against a run that does report something and cannot pass by checking
/// nothing.
///
/// Mutation: drop the `void` half of the pointer arm of `assignable`. The
/// two `void *` lines start reporting and this fails. Mutation: have
/// `Ast::compatible` answer `true` always. The `char *` line stops
/// reporting and this fails, which is the only place in the suite that
/// holds the comparison against a program rather than against a table.
#[test]
fn a_pointer_takes_the_same_pointer_or_a_void_one() {
    let checked = checked(
        "int main(void) {
    int *p;
    char *c;
    void *v;
    p = p;
    p = v;
    v = p;
    p = c;
    return 0;
}
",
    );

    assert_eq!(checked.messages(), ["cannot assign `char *` to `int *`"]);
}

/// C17 6.5.2.2 p2 makes the argument count a constraint only where the
/// callee's type includes a prototype, and 6.7.6.3 p14 makes `()` an empty
/// identifier list rather than one.
///
/// Both halves in one test, because the silence alone passes against a
/// compiler that counts nothing.
///
/// Mutation: treat `Parameters::Unspecified` as a list of no parameters.
/// The first program starts reporting three arguments too many, against a
/// program `clang` compiles, and this fails.
#[test]
fn an_empty_parameter_list_says_nothing_about_the_count() {
    let nothing_said = checked(
        "int f();

int main(void) { return f(1, 2, 3); }
",
    );
    assert_eq!(nothing_said.messages(), Vec::<&str>::new());

    let a_prototype = checked(
        "int f(void);

int main(void) { return f(1, 2, 3); }
",
    );
    assert_eq!(
        a_prototype.messages(),
        ["too many arguments: expected 0, found 3"]
    );
}

/// C17 6.5.15 p5 and p6's type for a conditional whose arms p3 allows.
///
/// Two arithmetic arms are `int`, whatever either is; two `void` arms are
/// `void`; two pointers to one type are that type; and a pointer beside a
/// pointer to `void` is the pointer to `void`, on either side. A corpus
/// case reads one of these through the nullability check, but only this
/// spells each, so an arm answering the wrong one of two types it was
/// given fails here.
///
/// Mutation: have `Checker::conditional` answer the first arm's type for
/// two arithmetic arms; the `ch` row is `char` and this fails. Mutation:
/// answer the pointer for a pointer to `void` on the left; the `v : p` row
/// fails.
#[test]
fn a_conditional_has_the_type_c_gives_its_arms() {
    let checked = checked(
        "void h(void);
int main(void) {
    int *p;
    void *v;
    char ch;
    int x;
    x = 1 ? 2 : 3;
    x = 1 ? ch : ch;
    1 ? h() : h();
    p = 1 ? p : p;
    1 ? p : v;
    1 ? v : p;
    return 0;
}
",
    );

    assert_eq!(checked.messages(), Vec::<&str>::new());

    for (text, spelling) in [
        ("1 ? 2 : 3", "int"),
        ("1 ? ch : ch", "int"),
        ("1 ? h() : h()", "void"),
        ("1 ? p : p", "int *"),
        ("1 ? p : v", "void *"),
        ("1 ? v : p", "void *"),
    ] {
        assert_eq!(checked.spelling(text), spelling, "{text}");
    }
}

/// C17 6.7.6.3 p15 makes a prototype and an empty identifier list
/// compatible when the default argument promotions leave every parameter
/// alone, so the first assignment is ordinary C and `clang` accepts it.
///
/// Mutation: answer `false` for `(Prototype, Unspecified)` in
/// `Ast::compatible_parameters`, which is what reading 6.7.6.3 p10 and p14
/// alone gives. The first line starts reporting and this fails.
#[test]
fn a_function_pointer_takes_one_of_a_compatible_type() {
    let checked = checked(
        "int main(void) {
    int (*p)(void);
    int (*q)();
    int (*r)(char);
    p = q;
    r = q;
    return 0;
}
",
    );

    assert_eq!(
        checked.messages(),
        ["cannot assign `int (*)()` to `int (*)(char)`"]
    );
}

/// A type in a message is one line, however the file wrote it.
///
/// An array's length is spelled with the source's own bytes, so a comment
/// inside one can carry a newline, and `render.rs::shown` keeps a newline
/// on purpose. Without `Checker::spelled` a file could print a line of its
/// own invention into this compiler's report, which is what the input
/// below tries to do.
///
/// Mutation: spell the types in `assignment` with `spell_type` directly.
/// The message and the labels carry the comment's newlines and this fails.
#[test]
fn a_type_in_a_message_is_one_line_however_it_was_written() {
    let checked = checked(
        "int main(void) {
    int (*r)[1 /*
error[SC0302]: no problems found
*/ + 2];
    int *p;
    p = r;
    return 0;
}
",
    );

    let reported = checked.diagnostics.diagnostics();
    let [reported] = reported else {
        panic!("{reported:?}");
    };

    assert!(
        !reported.message().contains('\n'),
        "{:?}",
        reported.message()
    );
    for label in reported.labels() {
        assert!(!label.message().contains('\n'), "{:?}", label.message());
    }
}

/// A compound assignment is C17 6.5.16.2, whose constraints are its own:
/// `p += 1` is how a pointer is advanced.
///
/// Mutation: have the compound arm of `type_of` call `assignment` rather
/// than `compound_assignment`. This fails with a diagnostic about correct
/// C.
#[test]
fn a_compound_assignment_is_not_the_rule_for_a_plain_one() {
    let checked = checked("int main(void) { int *p; p += 1; return 0; }\n");

    assert_eq!(checked.messages(), Vec::<&str>::new());
}

/// C17 6.5.16.2 refuses a pointer only where it says so: `+=` and `-=`
/// take one on the left with an integer on the right, and nothing else
/// takes one at all.
///
/// One program with the silences and a report together, so the silences
/// are asserted against a run that does report. `v += 1` reports, because
/// `void` is not a complete object type and p1 asks for a pointer to one.
///
/// Mutation: have `compound_assignable` answer `Some(false)` for a
/// pointer with `+=` or `-=`. `p += 1` and `p -= 1` start reporting.
/// Mutation: have it answer `Some(true)` for a pointer with `+=` or `-=`
/// whatever it points to. `v += 1` goes silent. Mutation: give the report
/// `MISMATCH`. The code fails.
#[test]
fn a_compound_assignment_refuses_a_pointer_only_where_c_does() {
    let checked = checked(
        "int main(void) {
    int *p;
    int i;
    char c;
    void *v;
    p += 1;
    p -= 1;
    i += 1;
    c *= 2;
    i <<= 1;
    v += 1;
    p *= 2;
    return 0;
}
",
    );

    assert_eq!(
        checked.messages(),
        [
            "`+=` cannot take `void *` and `int`",
            "`*=` cannot take `int *` and `int`"
        ]
    );
    assert_eq!(checked.codes(), ["SC0306", "SC0306"]);
}

/// Every operator and every pair of operand types 6.5.16.2 answers for,
/// each in a program of its own, with the message and the primary label
/// written out rather than worked out, so as not to compare the code with
/// itself.
///
/// A row is `(statement, message, primary label)`, and an empty message is
/// a statement that must stay silent. The primary label is the first one,
/// and it names the operand the rule refuses.
///
/// Mutation: move any of `/`, `%`, `>>`, `&`, `^` or `|` into the arm
/// that lets a pointer take an integer. Its row goes silent. Mutation:
/// answer `None` or `Some(true)` for a `char` place and a pointer value.
/// The `c += p` row goes silent, and that is a local declared `char`
/// holding an addition over a pointer, which ADR-0030 drops. Mutation:
/// refuse a pointer to a pointer or to an array of known length. Their
/// rows start reporting. Mutation: have `unsteppable` answer `None` for
/// an array of unknown length, `void` or a function. The `q += 1`,
/// `v -= 1` or `fp -= 1` row goes silent. Mutation: answer `None` for a `void` place or a
/// `void`, array or function value. Those rows go silent. Mutation: pick
/// the primary label by whether the value is a pointer, as this did
/// before. The `void` and array value rows name the place.
#[test]
fn a_compound_assignment_answers_for_every_operator_and_operand() {
    for (statement, message, primary) in [
        (
            "p /= 2;",
            "`/=` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p %= 2;",
            "`%=` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p >>= 1;",
            "`>>=` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p &= 1;",
            "`&=` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p ^= 1;",
            "`^=` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p |= 1;",
            "`|=` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "c += p;",
            "`+=` cannot take `char` and `int *`",
            "this is `int *`",
        ),
        (
            "*v += 1;",
            "`+=` cannot take `void` and `int`",
            "this is `void`",
        ),
        (
            "i += g();",
            "`+=` cannot take `int` and `void`",
            "this is `void`",
        ),
        (
            "i += a;",
            "`+=` cannot take `int` and `int[2]`",
            "this is `int[2]`",
        ),
        ("pp += 1;", "", ""),
        ("pa -= 1;", "", ""),
        (
            "q += 1;",
            "`+=` cannot take `int (*)[]` and `int`",
            "this is `int (*)[]`",
        ),
        (
            "v -= 1;",
            "`-=` cannot take `void *` and `int`",
            "this is `void *`",
        ),
        (
            "fp -= 1;",
            "`-=` cannot take `void (*)(void)` and `int`",
            "this is `void (*)(void)`",
        ),
    ] {
        let checked = checked(&format!(
            "void g(void);
int main(void) {{
    int *p;
    char c;
    void *v;
    int i;
    int a[2];
    int **pp;
    int (*pa)[2];
    int (*q)[];
    void (*fp)(void);
    {statement}
    return 0;
}}
"
        ));

        if message.is_empty() {
            assert_eq!(checked.messages(), Vec::<&str>::new(), "{statement}");
        } else {
            assert_eq!(checked.messages(), [message], "{statement}");
            assert_eq!(checked.labels().first(), Some(&primary), "{statement}");
        }
    }
}

/// Every binary operator against the constraint of its own clause, C17
/// 6.5.5 p2 to 6.5.14 p2, each in a program of its own, with the message
/// and the primary label written out rather than worked out, so as not to
/// compare the code with itself.
///
/// A row is `(expression, message, primary label)`, and an empty message
/// is an expression that must stay silent. The silences are every pairing
/// C allows, so a clause answered too strictly fails here as surely as one
/// answered too loosely.
///
/// Mutation: have `binary` stop calling `binary_operable`. Every reporting
/// row goes silent. Mutation: have the `==` arm answer `false` for a
/// pointer beside an integer. `p == 0` and `0 == p` start reporting;
/// answer `left_is_null` on both sides and `p == 0` alone does. Mutation:
/// drop `compatible` from `-`, `<` or `==`. `p - c`, `p < c` or `p == c`
/// goes silent. Mutation: drop the function test from the relational arm.
/// `g < g` goes silent. Mutation: have `is_void_beside_an_object` ignore
/// `other`. `v == g` goes silent. Mutation: answer `Void` as a scalar for
/// `&&`. `g() && 1` goes silent. Mutation: have `report_operands` put the
/// primary label on the left always. `1 >> p`, `n - p` and `p == 1` name
/// the wrong operand; on the right always, and `p * 1` and `g() * 1` do.
/// Mutation: spell the undecayed types. The `a * 1` row names `int[2]`.
///
/// Every operator has a row that it refuses, because the four relational
/// operators share one arm and a mutation that splits them is otherwise
/// seen only through `<`. Mutation: answer `Some(true)` for `>`, `<=` and
/// `>=` alone. `p > 1`, `p >= c` and `g <= g` go silent. Mutation: answer
/// `Some(true)` for a `void` operand of a relational operator. `g() < 1`
/// goes silent. Mutation: move `>>` into `takes_a_pointer`'s `true` arm,
/// or `!=`, `&&` and `||` into its `false` arm. `p >> 1`, `p != 1`,
/// `p && g()` and `p || g()` name the wrong operand. Mutation: refuse a
/// `char` on the left alone. `c[0] * p` names `c[0]`.
///
/// A pointer to something with no size is refused by `+` and `-`, C17
/// 6.5.6 p2 and p3, and named whichever side it is on. Mutation: have the
/// `+` arm answer `Ok` for a pointer and an integer whatever it points to.
/// `v + 1`, `1 + v`, `g + 1` and `u + 1` go silent. Mutation: ask only the
/// left pointee of a subtraction of two pointers. `pa - u` goes silent.
/// Mutation: pick the primary label by `Refused::Pairing`'s rule for a
/// `Refused::Pointee` too. `v + 1` and `u - pa` name the wrong operand.
#[test]
fn a_binary_operator_answers_for_every_operator_and_operand() {
    for (expression, message, primary) in [
        (
            "p * 1",
            "`*` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p / 1",
            "`/` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p % 2",
            "`%` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p << 1",
            "`<<` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "1 >> p",
            "`>>` cannot take `int` and `int *`",
            "this is `int *`",
        ),
        (
            "p & 1",
            "`&` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p ^ 1",
            "`^` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "p | 1",
            "`|` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "a * 1",
            "`*` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "n - p",
            "`-` cannot take `int` and `int *`",
            "this is `int *`",
        ),
        (
            "p + q",
            "`+` cannot take `int *` and `int *`",
            "this is `int *`",
        ),
        (
            "p - c",
            "`-` cannot take `int *` and `char *`",
            "this is `char *`",
        ),
        (
            "p < c",
            "`<` cannot take `int *` and `char *`",
            "this is `char *`",
        ),
        (
            "p == c",
            "`==` cannot take `int *` and `char *`",
            "this is `char *`",
        ),
        (
            "p < 0",
            "`<` cannot take `int *` and `int`",
            "this is `int`",
        ),
        (
            "p < n",
            "`<` cannot take `int *` and `int`",
            "this is `int`",
        ),
        (
            "p == 1",
            "`==` cannot take `int *` and `int`",
            "this is `int`",
        ),
        (
            "1 != p",
            "`!=` cannot take `int` and `int *`",
            "this is `int *`",
        ),
        (
            "g < g",
            "`<` cannot take `void (*)(void)` and `void (*)(void)`",
            "this is `void (*)(void)`",
        ),
        (
            "v == g",
            "`==` cannot take `void *` and `void (*)(void)`",
            "this is `void (*)(void)`",
        ),
        (
            "g() * 1",
            "`*` cannot take `void` and `int`",
            "this is `void`",
        ),
        (
            "1 * g()",
            "`*` cannot take `int` and `void`",
            "this is `void`",
        ),
        (
            "g() && 1",
            "`&&` cannot take `void` and `int`",
            "this is `void`",
        ),
        (
            "n || g()",
            "`||` cannot take `int` and `void`",
            "this is `void`",
        ),
        (
            "g() == 1",
            "`==` cannot take `void` and `int`",
            "this is `void`",
        ),
        (
            "g() < 1",
            "`<` cannot take `void` and `int`",
            "this is `void`",
        ),
        (
            "p >> 1",
            "`>>` cannot take `int *` and `int`",
            "this is `int *`",
        ),
        (
            "c[0] * p",
            "`*` cannot take `char` and `int *`",
            "this is `int *`",
        ),
        (
            "p > 1",
            "`>` cannot take `int *` and `int`",
            "this is `int`",
        ),
        (
            "p >= c",
            "`>=` cannot take `int *` and `char *`",
            "this is `char *`",
        ),
        (
            "g <= g",
            "`<=` cannot take `void (*)(void)` and `void (*)(void)`",
            "this is `void (*)(void)`",
        ),
        (
            "p != 1",
            "`!=` cannot take `int *` and `int`",
            "this is `int`",
        ),
        (
            "p && g()",
            "`&&` cannot take `int *` and `void`",
            "this is `void`",
        ),
        (
            "p || g()",
            "`||` cannot take `int *` and `void`",
            "this is `void`",
        ),
        ("n * c[0]", "", ""),
        ("n % 2 << 1 & 3 ^ 4 | 5", "", ""),
        ("p - q", "", ""),
        ("p + 1", "", ""),
        ("1 + p", "", ""),
        ("p - 1", "", ""),
        ("a + 1", "", ""),
        ("p < q", "", ""),
        ("p > q", "", ""),
        ("p <= q", "", ""),
        ("v >= v", "", ""),
        ("p == q", "", ""),
        ("p == 0", "", ""),
        ("0 == p", "", ""),
        ("p != v", "", ""),
        ("a == p", "", ""),
        ("g == g", "", ""),
        ("p && q", "", ""),
        ("p || n", "", ""),
        (
            "v + 1",
            "`+` cannot take `void *` and `int`",
            "this is `void *`",
        ),
        (
            "1 + v",
            "`+` cannot take `int` and `void *`",
            "this is `void *`",
        ),
        (
            "v - 1",
            "`-` cannot take `void *` and `int`",
            "this is `void *`",
        ),
        (
            "v - v",
            "`-` cannot take `void *` and `void *`",
            "this is `void *`",
        ),
        (
            "g + 1",
            "`+` cannot take `void (*)(void)` and `int`",
            "this is `void (*)(void)`",
        ),
        (
            "u + 1",
            "`+` cannot take `int (*)[]` and `int`",
            "this is `int (*)[]`",
        ),
        (
            "pa - u",
            "`-` cannot take `int (*)[2]` and `int (*)[]`",
            "this is `int (*)[]`",
        ),
        (
            "u - pa",
            "`-` cannot take `int (*)[]` and `int (*)[2]`",
            "this is `int (*)[]`",
        ),
        ("pa - pa", "", ""),
    ] {
        let checked = checked(&format!(
            "void g(void);
int main(void) {{
    int *p;
    int *q;
    char *c;
    void *v;
    int n;
    int a[2];
    int (*pa)[2];
    int (*u)[];
    {expression};
    return 0;
}}
"
        ));

        if message.is_empty() {
            assert_eq!(checked.messages(), Vec::<&str>::new(), "{expression}");
        } else {
            assert_eq!(checked.messages(), [message], "{expression}");
            assert_eq!(checked.codes(), ["SC0306"], "{expression}");
            assert_eq!(checked.labels().first(), Some(&primary), "{expression}");
        }
    }
}

/// A pointer refused for what it points to says so, with the paragraph of
/// the spelling that refused it, and a pointer refused for the pairing it
/// is in says nothing more than the pairing: `p - v` is two pointers that
/// are not compatible, and a note about `void` having no size would be a
/// reason that is false there.
///
/// Mutation: attach the note to every refusal with a pointer operand. The
/// `p - v`, `v + v` and `n - v` rows gain one. Mutation: cite p2 for `-`.
/// The `v - 1` row fails. Mutation: drop the note from
/// `compound_assignment`. The `+=` and `-=` rows lose theirs. Mutation:
/// drop the `+=` and `-=` test from `compound_assignment`'s note. `v *= 2`
/// gains one, and `*=` refuses a pointer whatever it points to. Mutation:
/// swap two of `unsteppable`'s reasons. The rows for those two fail.
#[test]
fn a_pointer_refused_for_what_it_points_to_says_why() {
    for (code, note) in [
        (
            "v + 1;",
            "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.6 p2)",
        ),
        (
            "v - 1;",
            "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.6 p3)",
        ),
        (
            "g + 1;",
            "a pointer steps by the size of what it points to, and a function is not an object (C17 6.5.6 p2)",
        ),
        (
            "u + 1;",
            "a pointer steps by the size of what it points to, and an array of unknown length has no size (C17 6.5.6 p2)",
        ),
        (
            "v += 1;",
            "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.16.2 p1)",
        ),
        (
            "u -= 1;",
            "a pointer steps by the size of what it points to, and an array of unknown length has no size (C17 6.5.16.2 p1)",
        ),
        ("p - v;", ""),
        ("v + v;", ""),
        ("n - v;", ""),
        ("v *= 2;", ""),
    ] {
        let checked = checked(&format!(
            "void g(void);
int main(void) {{
    int *p;
    void *v;
    int n;
    int (*u)[];
    {code}
    return 0;
}}
"
        ));

        assert_eq!(checked.codes(), ["SC0306"], "{code}");
        if note.is_empty() {
            assert_eq!(checked.notes(), Vec::<&str>::new(), "{code}");
        } else {
            assert_eq!(checked.notes(), [note], "{code}");
        }
    }
}

/// An increment and a subscript are held to the step rule `v + 1` is, by
/// C17 6.5.2.4 p2, 6.5.3.1 p2 and 6.5.2.1 p1, each with the message, the
/// primary label and the paragraph of its own spelling, and a pointer to
/// an `int` or an array of `int` steps in silence in both.
///
/// Mutation: have `increment` stop asking `unsteppable`. The `++` and `--`
/// rows fail. Mutation: have `subscript` stop asking it. The `[]` rows
/// fail. Mutation: have `subscript` ask only when the base is the pointer.
/// `1[v]` fails. Mutation: put the primary label on the base always.
/// `1[v]`'s label fails. Mutation: drop `decayed` from the base in
/// `subscript`. `g[1]` fails, and from the index, `1[g]`. Mutation: swap
/// the two increments' clauses. `v++` and `--v` fail. Mutation: have
/// `unsteppable` refuse `int`. The silent rows fail.
#[test]
fn an_increment_and_a_subscript_take_the_step_the_additive_operators_do() {
    for (code, message, primary, note) in [
        (
            "v++;",
            "`++` cannot take `void *`",
            "this is `void *`",
            "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.2.4 p2)",
        ),
        (
            "--v;",
            "`--` cannot take `void *`",
            "this is `void *`",
            "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.3.1 p2)",
        ),
        (
            "fp--;",
            "`--` cannot take `void (*)(void)`",
            "this is `void (*)(void)`",
            "a pointer steps by the size of what it points to, and a function is not an object (C17 6.5.2.4 p2)",
        ),
        (
            "++u;",
            "`++` cannot take `int (*)[]`",
            "this is `int (*)[]`",
            "a pointer steps by the size of what it points to, and an array of unknown length has no size (C17 6.5.3.1 p2)",
        ),
        (
            "v[1];",
            "`[]` cannot take `void *` and `int`",
            "this is `void *`",
            "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.2.1 p1)",
        ),
        (
            "1[v];",
            "`[]` cannot take `int` and `void *`",
            "this is `void *`",
            "a pointer steps by the size of what it points to, and `void` has no size (C17 6.5.2.1 p1)",
        ),
        (
            "u[0];",
            "`[]` cannot take `int (*)[]` and `int`",
            "this is `int (*)[]`",
            "a pointer steps by the size of what it points to, and an array of unknown length has no size (C17 6.5.2.1 p1)",
        ),
        (
            "g[1];",
            "`[]` cannot take `void (*)(void)` and `int`",
            "this is `void (*)(void)`",
            "a pointer steps by the size of what it points to, and a function is not an object (C17 6.5.2.1 p1)",
        ),
        (
            "1[g];",
            "`[]` cannot take `int` and `void (*)(void)`",
            "this is `void (*)(void)`",
            "a pointer steps by the size of what it points to, and a function is not an object (C17 6.5.2.1 p1)",
        ),
        ("p++;", "", "", ""),
        ("--p;", "", "", ""),
        ("p[1];", "", "", ""),
        ("1[p];", "", "", ""),
        ("a[1];", "", "", ""),
    ] {
        let checked = checked(&format!(
            "void g(void);
int main(void) {{
    int *p;
    void *v;
    void (*fp)(void);
    int (*u)[];
    int a[2];
    {code}
    return 0;
}}
"
        ));

        if message.is_empty() {
            assert_eq!(checked.messages(), Vec::<&str>::new(), "{code}");
        } else {
            assert_eq!(checked.messages(), [message], "{code}");
            assert_eq!(checked.codes(), ["SC0306"], "{code}");
            assert_eq!(checked.labels().first(), Some(&primary), "{code}");
            assert_eq!(checked.notes(), [note], "{code}");
        }
    }
}

/// A refused increment or subscript keeps the type it would have had, as
/// a refused `+=` does, because C gives each from an operand rather than
/// from the step that was refused: `p++` is `p`'s type and `v[1]` is what
/// `v` points at, whichever side of the `[]` the pointer is on.
///
/// Mutation: have `increment` answer `None` after its report. The `++`
/// and `--` rows fail. Mutation: have `subscript` answer `None` after its
/// report. The `[]` rows fail. Mutation: have `subscript` read the type off
/// the base only. The `1[v]` and `1[g]` rows fail.
#[test]
fn a_refused_increment_or_subscript_keeps_its_type() {
    for (code, written, ty) in [
        ("v++;", "v++", "void *"),
        ("--v;", "--v", "void *"),
        ("v[1];", "v[1]", "void"),
        ("1[v];", "1[v]", "void"),
        ("u[0];", "u[0]", "int[]"),
        ("g[1];", "g[1]", "void (void)"),
        ("1[g];", "1[g]", "void (void)"),
    ] {
        let checked = checked(&format!(
            "void g(void);
int main(void) {{
    void *v;
    int (*u)[];
    {code}
    return 0;
}}
"
        ));

        assert_eq!(checked.codes(), ["SC0306"], "{code}");
        assert_eq!(checked.spelling(written), ty, "{code}");
    }
}

/// A subscript beside an index nothing typed is still what its pointer
/// points at, so the check around it goes on.
///
/// `p - q` has no type here, for want of a `ptrdiff_t`, and nothing is
/// reported about it because it is valid C. `p[p - q]` and `(p - q)[p]` are
/// an `int` whatever the other operand is, which is what makes giving either
/// to an `int *` a mismatch.
///
/// Mutation: answer no type where either operand is untyped. Both `SC0302`s
/// go and this fails. Mutation: answer no type where the base is untyped.
/// The second goes and this fails.
#[test]
fn a_subscript_beside_an_untyped_operand_keeps_its_pointers_type() {
    let checked = checked(
        "int main(void) {
    int *p;
    int *q;
    int *r = p[p - q];
    int *s = (p - q)[p];
    return 0;
}
",
    );

    assert_eq!(checked.codes(), ["SC0302", "SC0302"]);
    assert_eq!(checked.spelling("p[p - q]"), "int");
}

/// A subscript refused for its pairing has no type, so the check around it
/// says nothing about a value the program never had.
///
/// Mutation: keep reading the pointer's type after a refused pairing. `p[q]`
/// is an `int`, `int *r = p[q];` gains an `SC0302`, and this fails.
#[test]
fn a_subscript_refused_for_its_pairing_has_no_type() {
    let checked = checked(
        "int main(void) {
    int *p;
    int *q;
    int *r = p[q];
    return 0;
}
",
    );

    assert_eq!(checked.codes(), ["SC0306"]);
}

/// An initializer is held to the rule for a plain `=`, C17 6.7.9 p11, at
/// file scope and in a block, with the words a declaration was written in.
///
/// One program with the silences and the reports together, so that the
/// silences are asserted against a run that does report and cannot pass by
/// checking nothing. The silences are a null pointer constant, and a
/// `void *` both ways, which is an implicit conversion with no cast in the
/// grammar. `w` and `q` read an object at file scope, so each is also not
/// the constant C17 6.7.9 p4 asks (`SC0317`), and `w` is that alone: its
/// `void *` is still converted in silence.
///
/// Mutation: have `collect_receivers` skip an `Item::Declaration`. The
/// file-scope report goes and this fails; nothing else in the suite
/// declares at file scope with a mismatch. Mutation: pass `false` for
/// `source_is_null` in `check_received`. `int *g = 0;` starts reporting
/// and this fails. Mutation: give the initializer the `Return` arm's
/// place label. The labels fail.
#[test]
fn an_initializer_in_either_scope_is_held_to_the_rule_for_assignment() {
    let checked = checked(
        "int h;
int *g = 0;
void *v = 0;
int *w = v;
int *q = h;
int main(void) {
    char c = 1;
    int *p = w;
    char *s = p;
    return 0;
}
",
    );

    assert_eq!(
        checked.messages(),
        [
            "cannot initialize `int *` with `int`",
            "cannot initialize `char *` with `int *`",
            "the initializer of `w` is not a constant",
            "the initializer of `q` is not a constant",
        ]
    );
    assert_eq!(
        checked.labels(),
        [
            "this is `int`",
            "this holds `int *`",
            "this is `int *`",
            "this holds `char *`",
            "not a constant expression",
            "not a constant expression",
        ]
    );
}

/// Every declarator of a declaration is checked, not only the first, and
/// one with no initializer does not end the search.
///
/// `m` is written first on purpose: it is the trivial value a list of one
/// would pass every other test with, and a widened input that every other
/// test passes the trivial value is held by nothing.
///
/// Mutation: have `initializers` read `declarators.iter().take(1)`.
/// Mutation: have it `break` rather than `continue` at a declarator with
/// no initializer. Either way `n` stops being checked and this fails.
#[test]
fn every_declarator_of_a_declaration_has_its_initializer_checked() {
    let checked = checked(
        "int main(void) { int *p = 0; int m, n = p; return 0; }
",
    );

    assert_eq!(checked.messages(), ["cannot initialize `int` with `int *`"]);
}

/// An initializer and a `return` are reported where they are written,
/// among the other diagnostics, rather than after all of them.
///
/// Mutation: run `check_received` in a loop of its own after the walk that
/// works out the types. The assignment moves ahead of the initializer and
/// this fails.
#[test]
fn an_initializer_and_a_return_are_reported_in_the_order_they_are_written() {
    let checked = checked(
        "int main(void) {
    int *p = 0;
    int n = p;
    n = p;
    return p;
}
",
    );

    assert_eq!(
        checked.messages(),
        [
            "cannot initialize `int` with `int *`",
            "cannot assign `int *` to `int`",
            "cannot return `int *` from a function returning `int`",
        ]
    );
}

/// Every place a `return` can be written, which is every place a statement
/// can hold another.
///
/// Mutation: drop any arm of `Checker::receivers_in` that recurses. The
/// `return` under it stops being checked, the list is short by one, and
/// this fails. Every initializer under that arm stops being checked with
/// it, which is why the two are found by one walk: see `Receiving`.
#[test]
fn a_return_is_found_wherever_it_is_written() {
    let checked = checked(
        "int *g(void) {\n    if (1) return 1; else return 1;\n    while (1) return 1;\n    for (;;) return 1;\n    { return 1; }\n    return 1;\n}\n",
    );

    assert_eq!(
        checked.messages(),
        ["cannot return `int` from a function returning `int *`"; 6]
    );
}

/// A name nothing declares has no type, and an expression with no type is
/// not reported on.
///
/// Mutation: report when either side of an assignment is `None`. The
/// undeclared name gains a second diagnostic about a type nobody knows and
/// this fails. Mutation: have `additive` or `unary` answer `int` for an
/// operand nothing typed. The rows with an operator in them gain the same
/// second diagnostic, which is what they are here for: the bare name alone
/// passed against a compiler that guessed.
///
/// The rows with `*` hold `Checker::binary`, which answers `int` beside an
/// operand untyped as this compiler's gap and must not beside one untyped
/// because a fault in it was reported. Mutation: have `binary` answer `int`
/// beside any untyped operand, as it did before #238. All three gain an
/// `int` given to a pointer. Mutation: do not mark a name the resolver
/// could not resolve. `nowhere * 1` fails. Mutation: do not carry a mark up
/// from an operand. `-nowhere * 1` and `(nowhere + 1) * 2` fail.
#[test]
fn a_name_that_resolved_to_nothing_is_reported_once() {
    for value in [
        "nowhere",
        "nowhere + 1",
        "1 + nowhere",
        "-nowhere",
        "nowhere * 1",
        "-nowhere * 1",
        "(nowhere + 1) * 2",
    ] {
        let checked = checked(&format!(
            "int main(void) {{ int *p; p = {value}; return 0; }}\n"
        ));

        assert_eq!(
            checked.messages(),
            ["use of undeclared identifier `nowhere`"],
            "{value}"
        );
    }
}

/// Every operator an integer constant expression may hold gives its C value,
/// and an operation C leaves without one gives none.
///
/// C17 6.6 p6, at `int`: division truncates toward zero (6.5.5 p6), a right
/// shift of a negative value is arithmetic as the backend's `ashr` is
/// (6.5.7 p5 leaves it to the implementation), the relational, equality and
/// logical operators give 1 or 0 (6.5.8 p6, 6.5.9 p3, 6.5.13 p3, 6.5.14 p3)
/// whatever non-zero value made them true, and `?:` is the arm its condition
/// picks. No value for a result outside `int`, `INT_MIN / -1` and
/// `INT_MIN % -1` and `-INT_MIN` included, a division or remainder by zero,
/// a shift by a negative amount or by the width, a left shift of a negative
/// value, anything with a variable in it, even in an arm not taken or an
/// operand `&&` or `||` would not evaluate, since 6.6 p6 asks it of every
/// operand, and a comma, even of two constants (6.6 p3). An operand that is
/// not evaluated need not have a value, only be a constant: `1 || 1 / 0` is
/// 1, and `1 && 1 / 0`, which evaluates the division, has none.
///
/// Each comparison has a row on each side of its boundary, and each logical
/// operator one with a non-zero operand other than 1, so a comparison that
/// moved its boundary, or a truth that compared with 1, fails.
///
/// Mutation: break any one operator, move a boundary, or let any one
/// undefined case through. Its row fails.
#[test]
fn a_constant_expression_has_the_value_c_gives_it() {
    for (written, value) in [
        ("1 + 2", Some(3)),
        ("7 - 9", Some(-2)),
        ("3 * 4", Some(12)),
        ("7 / 2", Some(3)),
        ("-7 / 2", Some(-3)),
        ("7 % 3", Some(1)),
        ("-7 % 3", Some(-1)),
        ("1 << 3", Some(8)),
        ("16 >> 2", Some(4)),
        ("-8 >> 1", Some(-4)),
        ("16 >> 31", Some(0)),
        ("1 < 2", Some(1)),
        ("2 < 2", Some(0)),
        ("2 > 1", Some(1)),
        ("2 > 2", Some(0)),
        ("1 <= 1", Some(1)),
        ("2 <= 1", Some(0)),
        ("1 >= 1", Some(1)),
        ("1 >= 2", Some(0)),
        ("1 == 1", Some(1)),
        ("1 == 2", Some(0)),
        ("1 != 2", Some(1)),
        ("2 != 1", Some(1)),
        ("1 != 1", Some(0)),
        ("6 & 3", Some(2)),
        ("6 ^ 3", Some(5)),
        ("6 | 3", Some(7)),
        ("2 && 3", Some(1)),
        ("1 && 0", Some(0)),
        ("2 || 0", Some(1)),
        ("0 || 0", Some(0)),
        ("-5", Some(-5)),
        ("+5", Some(5)),
        ("~0", Some(-1)),
        ("!3", Some(0)),
        ("!0", Some(1)),
        ("2 ? 4 : 5", Some(4)),
        ("0 ? 4 : 5", Some(5)),
        ("-2147483647 - 1", Some(-2147483648)),
        ("2147483647 + 1", None),
        ("-2147483647 - 2", None),
        ("2147483647 * 2", None),
        ("-(-2147483647 - 1)", None),
        ("(-2147483647 - 1) / -1", None),
        ("(-2147483647 - 1) % -1", None),
        ("1 / 0", None),
        ("1 % 0", None),
        ("1 << -1", None),
        ("1 << 32", None),
        ("16 >> 32", None),
        ("16 >> -1", None),
        ("-1 << 1", None),
        ("1 << 31", None),
        ("x + 1", None),
        ("x ? 2 : 3", None),
        ("1 ? 2 : x", None),
        ("0 && x", None),
        ("1 || x", None),
        ("1 || 1 / 0", Some(1)),
        ("0 && 1 / 0", Some(0)),
        ("1 ? 2 : 1 / 0", Some(2)),
        ("0 ? 1 / 0 : 3", Some(3)),
        ("1 && 1 / 0", None),
        ("0 ? 2 : 1 / 0", None),
        ("(1, 2)", None),
    ] {
        let checked = checked(&format!(
            "int main(void) {{\n    int x;\n    int r;\n    r = {written};\n    return 0;\n}}\n"
        ));
        // The right of the one assignment, rather than an expression found
        // by its text, because a parenthesised operand's span leaves the
        // parentheses out.
        let assigned = checked
            .ast
            .expr_ids()
            .find_map(|id| match checked.ast.expr(id) {
                Expr::Assign { value, .. } => Some(*value),
                _ => None,
            })
            .expect("the assignment");
        assert_eq!(checked.types.value(assigned), value, "{written}");
    }
}

/// An object of struct type is typed, and what C says of a struct is asked
/// of it: assigned from a struct of its tag and of nothing else, and an
/// operand of no operator, no condition and no `*` (C17 6.5.16.1 p1, 6.5.3,
/// 6.5.5 to 6.5.14, 6.8.4.1 p1).
///
/// Mutation: answer `assignable` true for every struct pair; `s = t` goes
/// silent. Mutation: give `OperandClass::of` no class for a struct, as
/// before; `s + 1` goes silent. Mutation: refuse only `void` as a
/// condition; `if (s)` goes silent. Mutation: drop the struct arm in
/// `increment`; `s++` goes silent. Each fails this.
#[test]
fn a_struct_is_typed_and_held_to_what_c_says_of_one() {
    let checked = checked(
        "struct S {\n    int a;\n};\nstruct T {\n    int b;\n};\nint f(struct S s, struct T t, struct S u) {\n    int x = s;\n    s = t;\n    s = u;\n    if (s) {\n        return 1;\n    }\n    s++;\n    -s;\n    x = s + 1;\n    return *s;\n}\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "cannot initialize `int` with `struct S`",
            "cannot assign `struct T` to `struct S`",
            "`if` cannot take `struct S`",
            "`++` cannot take `struct S`",
            "`-` cannot take `struct S`",
            "`+` cannot take `struct S` and `int`",
            "`*` cannot take `struct S`",
        ]
    );
}

/// Pointers to structs are compared, subtracted, chosen between and
/// assigned by their tag at every place the type checker asks
/// `Ast::compatible`: two tags are refused at each, and one tag at none.
///
/// Mutation: pass `|_, _| true` for `same_struct` at any of the five calls
/// in this module that compare pointers; that call accepts two tags, and
/// this fails. The two that compare struct values, in `assignable` and
/// `conditional`, are held by
/// `a_struct_is_an_operand_of_no_operator_and_is_held_to_its_tag`.
#[test]
fn pointers_to_structs_are_held_to_their_tag_everywhere_types_are_compared() {
    let checked = checked(
        "struct S {\n    int a;\n};\nstruct T {\n    int b;\n};\nint f(struct S *p, struct S *r, struct T *q, int c) {\n    int x = p == r;\n    x = p < r;\n    x = p - r;\n    p = c ? p : r;\n    p = r;\n    x = p == q;\n    x = p < q;\n    x = p - q;\n    p = c ? p : q;\n    p = q;\n    return x;\n}\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "`==` cannot take `struct S *` and `struct T *`",
            "`<` cannot take `struct S *` and `struct T *`",
            "`-` cannot take `struct S *` and `struct T *`",
            "`?:` cannot take `struct S *` and `struct T *`",
            "cannot assign `struct T *` to `struct S *`",
        ]
    );
}

/// A step of a pointer to a struct asks whether the struct is complete
/// where the step is written, not where the pointer was declared: `p + 1`
/// above `struct S`'s definition is refused, and below it is not (C17 6.5.6
/// p2, 6.7.2.1 p8).
///
/// Mutation: ask completeness where the type was written, by answering
/// `complete_here` with `Resolution::complete`; the second step is refused
/// too. Mutation: answer every struct complete in `unsteppable`; the first
/// is not refused. Either fails this.
#[test]
fn a_step_of_a_struct_pointer_asks_completeness_where_it_is_written() {
    let checked = checked(
        "struct S;\nstruct S *p;\nvoid f(void) {\n    p + 1;\n}\nstruct S {\n    int a;\n};\nvoid g(void) {\n    p + 1;\n}\n",
    );

    assert_eq!(
        checked.messages(),
        ["`+` cannot take `struct S *` and `int`"]
    );
}

/// An object whose struct is not complete where C needs it is reported: in
/// a block at the end of its declarator (C17 6.7 p7), although `S` is
/// defined below it, and at file scope by the end of the translation unit
/// (6.9.2 p2). One completed later at file scope, and one with no tag, are
/// not.
///
/// Mutation: ask a file-scope object at its declarator as a block's is;
/// `u` is reported. Mutation: ask a block's object by the end of the
/// translation unit as a file-scope one is; `s` goes silent. Mutation: ask
/// only whether the tag is defined; the struct with no tag is reported.
/// Mutation: drop the check; `s` and `t` go silent. Mutation: change either
/// note; its row fails. Each fails this.
#[test]
fn an_object_of_an_incomplete_struct_is_reported() {
    let checked = checked(
        "struct S;\nvoid f(void) {\n    struct S s;\n}\nstruct T t;\nstruct U u;\nstruct U {\n    int a;\n};\nstruct {\n    int x;\n} v;\nstruct S {\n    int x;\n};\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "`t` has incomplete type `struct T`",
            "`s` has incomplete type `struct S`",
        ]
    );
    assert_eq!(checked.codes(), ["SC0315", "SC0315"]);
    assert_eq!(
        checked.notes(),
        [
            "an object at file scope has a type completed by the end of the translation unit (C17 6.9.2 p2)",
            "an object with no linkage has a complete type by the end of its declarator (C17 6.7 p7), and so does a parameter of a definition (6.7.6.3 p4)",
        ]
    );
}

/// What C needs complete of a function is held to it where it is written: a
/// definition's parameter (C17 6.7.6.3 p4) and what it returns (6.9.1 p3),
/// and what a call returns, where the call is (6.5.2.2 p1). A declaration
/// that is not a definition may name an incomplete struct, and a call below
/// the definition of `S` is complete.
///
/// Mutation: collect no parameter of a definition; `s` goes silent.
/// Mutation: drop `report_incomplete_return`; `g` goes silent. Mutation:
/// drop the check in `call`; `h()` in `k` goes silent. Mutation: ask the
/// call with `Resolution::complete`, where `h`'s type was written; `h()` in
/// `m` is reported too. Each fails this.
#[test]
fn a_function_is_held_to_what_c_needs_complete_of_it() {
    let checked = checked(
        "struct S;\nint f(struct S s) {\n    return 0;\n}\nstruct S g(void) {\n}\nstruct S h(void);\nint q(struct S s);\nvoid k(void) {\n    h();\n}\nstruct S {\n    int a;\n};\nvoid m(void) {\n    h();\n}\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "this call returns incomplete type `struct S`",
            "`g` returns incomplete type `struct S`",
            "`s` has incomplete type `struct S`",
        ]
    );
    assert_eq!(checked.codes(), ["SC0315", "SC0315", "SC0315"]);
}

/// `.` takes a struct and `->` a pointer to one (C17 6.5.2.3 p1), an array
/// of structs being one after 6.3.2.1 p3: the right shape is typed by the
/// member it names, the wrong one is the program's fault, and a base this
/// compiler left untyped is its own gap.
///
/// Mutation: refuse every member access as not yet checked, whatever its
/// base; `s.a` goes untyped and `.` on a pointer is called this compiler's
/// gap. Mutation: answer an array base as the wrong shape; `a->a` is called
/// a fault. Mutation: say nothing of a base with no type; `(i - j).a` and
/// `(i - j)->a`, left untyped as this compiler's gap, go silent. Mutation:
/// spell that report's operator as `.` whatever was written, or change its
/// label or note; its row fails. Each fails this.
#[test]
fn a_member_access_of_the_right_shape_is_typed_and_of_the_wrong_one_is_the_programs() {
    let checked = checked(
        "struct S {\n    int a;\n    char *c;\n};\nint f(struct S s, struct S *p, int *i, int *j) {\n    struct S a[2];\n    return s.a + *p->c + a->a + p.a + s->a + a.a + (i - j).a + (i - j)->a;\n}\n",
    );

    assert_eq!(checked.spelling("s.a"), "int");
    assert_eq!(checked.spelling("p->c"), "char *");
    assert_eq!(checked.spelling("a->a"), "int");
    assert_eq!(
        checked.messages(),
        [
            "`.` needs a struct, and this is `struct S *`",
            "`->` needs a pointer to a struct, and this is `struct S`",
            "`.` needs a struct, and this is `struct S[2]`",
            "cannot check `.` yet",
            "cannot check `->` yet",
        ]
    );
    assert_eq!(
        checked.codes(),
        ["SC0306", "SC0306", "SC0306", "SC0304", "SC0304"]
    );
    let untyped =
        "the base has no type here: either a fault reported above, or a gap in this compiler";
    assert_eq!(checked.notes()[3..], [untyped, untyped]);
    assert_eq!(
        checked.labels()[3..],
        [
            "a member of something with no type",
            "a member of something with no type"
        ]
    );
}

/// A base that is neither a struct for `.` nor a pointer to one, or an
/// array of them, for `->` is the program's fault, whatever else it is:
/// `void`, a function, `char`, a pointer to a pointer to a struct, and an
/// array of pointers to structs. `p->b` names the struct, not the pointer,
/// in its report.
///
/// Mutation: answer `void`, a function or `char` as a struct; that row is
/// not reported, and asking its tag panics. Mutation: look through one more
/// pointer for `->`; the `pp` and `ap` rows go silent. Mutation: spell the
/// base's type in a missing member's report; `p->b` says `struct S *`.
/// Each fails this.
#[test]
fn a_member_access_on_a_base_that_is_no_struct_is_the_programs() {
    let checked = checked(
        "struct S {\n    int a;\n};\nvoid v(void);\nchar c;\nint f(struct S **pp, struct S *p) {\n    struct S *ap[2];\n    return v().a + f.a + c.a + pp->a + ap->a + p->b;\n}\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "`.` needs a struct, and this is `void`",
            "`.` needs a struct, and this is `int (struct S **, struct S *)`",
            "`.` needs a struct, and this is `char`",
            "`->` needs a pointer to a struct, and this is `struct S **`",
            "`->` needs a pointer to a struct, and this is `struct S *[2]`",
            "no member named `b` in `struct S`",
        ]
    );
}

/// A tag given two definitions, which is `SC0312`, answers every access
/// from its first: what its members are, and from where they are known. So
/// `p->b` is no member wherever it is written, `p->a` is one, and `f`'s
/// access between the two definitions is not called incomplete.
///
/// Mutation: let the second definition be the one `Resolution::definition`
/// answers; `p->a` is reported and `p->b` is not. Mutation: let the second
/// move the step at which the tag closed; `f`'s access is `SC0315`. Either
/// fails this.
#[test]
fn a_tag_defined_twice_answers_from_its_first_definition() {
    let checked = checked(
        "struct V {\n    int a;\n};\nstruct V *p;\nint f(void) {\n    return p->b;\n}\nstruct V {\n    int b;\n};\nint g(void) {\n    return p->b + p->a;\n}\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "redefinition of `struct V`",
            "no member named `b` in `struct V`",
            "no member named `b` in `struct V`",
        ]
    );
}

/// A member access names a member of the struct its base is, found by
/// spelling among the members of the definition the base's tag names, and
/// recorded as that definition and the member's position in it (C17
/// 6.5.2.3 p1). A name the struct does not have is `SC0316`, at the name,
/// with the definition beside it.
///
/// Mutation: type a member access `int` without looking the member up;
/// `s.b` goes silent and `s.c` is `int`. Mutation: record index 0 for every
/// member; `s.c` is recorded as `a`. Mutation: look the name up among the
/// members of the base's own type rather than its tag's definition; `s`,
/// declared before the definition, has none. Mutation: answer
/// `Types::member` with any member recorded rather than `id`'s; `s.a` is
/// recorded as `c`, or `s.b` as a member. Each fails this.
#[test]
fn a_member_access_names_a_member_of_its_struct_or_is_reported() {
    let checked = checked(
        "struct S s;\nstruct S {\n    int a;\n    int *b2;\n    char c;\n};\nint f(void) {\n    return s.c + s.b + s.a;\n}\n",
    );

    assert_eq!(checked.spelling("s.c"), "char");
    let access = |text: &str| {
        checked
            .ast
            .expr_ids()
            .find(|&id| checked.sources.snippet(checked.ast.expr(id).span()) == text)
            .unwrap_or_else(|| panic!("{text:?} is not written"))
    };
    let member = checked
        .types
        .member(access("s.c"))
        .expect("`s.c` names a member");
    assert_eq!(member.index, 2);
    assert_eq!(
        checked.types.member(access("s.a")).map(|m| m.index),
        Some(0)
    );
    assert_eq!(checked.types.member(access("s.b")), None);
    assert!(matches!(
        checked.ast.ty(member.definition),
        Type::Struct { members: Some(members), .. } if members.len() == 3
    ));
    assert_eq!(checked.messages(), ["no member named `b` in `struct S`"]);
    assert_eq!(checked.codes(), ["SC0316"]);
    assert_eq!(
        checked.labels(),
        ["not a member of `struct S`", "`struct S` is defined here"]
    );
}

/// A member access asks whether its struct is complete where the access is
/// written, since an incomplete struct has no members yet (C17 6.7.2.1 p8):
/// `p->a` above `struct S`'s definition is `SC0315`, and below it is typed.
///
/// Mutation: ask completeness where `p`'s type was written, with
/// `Resolution::complete`; the second is refused too. Mutation: drop the
/// question, answering from the tag's definition alone; the first is typed.
/// Either fails this.
#[test]
fn a_member_access_needs_its_struct_complete_where_it_is_written() {
    let checked = checked(
        "struct S *p;\nint f(void) {\n    return p->a;\n}\nstruct S {\n    int a;\n};\nint g(void) {\n    return p->a + 0;\n}\n",
    );

    assert_eq!(
        checked.messages(),
        ["`struct S` is not complete here, and has no members yet"]
    );
    assert_eq!(checked.codes(), ["SC0315"]);
    assert_eq!(checked.spelling("p->a + 0"), "int");
}

/// A chain of member accesses is typed link by link: `s.t.x` reaches a
/// struct defined inside a member, and `s.t.y` is reported once, at `y`,
/// with `s.t.y.z` after it silent rather than a second fault.
///
/// Mutation: look every link up in the outermost base's struct; `s.t.x` is
/// reported. Mutation: report a base that is an untyped member access as
/// this compiler's gap; `s.t.y.z` is a second report. Either fails this.
#[test]
fn a_chain_of_member_accesses_is_typed_link_by_link() {
    let checked = checked(
        "struct S {\n    struct T {\n        int x;\n    } t;\n} s;\nint f(void) {\n    return s.t.x + s.t.y.z;\n}\n",
    );

    assert_eq!(checked.spelling("s.t.x"), "int");
    assert_eq!(checked.messages(), ["no member named `y` in `struct T`"]);
}

/// An array's element is a complete object type (C17 6.7.6.2 p1), and a
/// struct is one only where its definition has closed: `struct S a[2];`
/// above it is refused and below it is not.
///
/// Mutation: answer a struct element complete in `report_array`; `a` goes
/// silent. Mutation: answer it incomplete; `b` is refused too. Either fails
/// this.
#[test]
fn an_array_of_a_struct_needs_the_struct_complete_where_it_is_written() {
    let checked =
        checked("struct S;\nstruct S a[2];\nstruct S {\n    int x;\n};\nstruct S b[2];\n");

    assert_eq!(
        checked.messages(),
        ["an array cannot have `struct S` as its element"]
    );
}

/// A struct with a flexible array member is no member of another struct and
/// no element of an array (C17 6.7.2.1 p3), and a struct whose last member
/// is an array with a length, or which holds a pointer to one, is neither.
///
/// Mutation: drop the member check in `check_members`; `f` goes silent.
/// Mutation: drop the element check in `report_array`; `arr` goes silent.
/// Mutation: answer every struct with an array member as having a flexible
/// one; `harr` is reported. Mutation: answer from the first member rather
/// than the last; both go silent. Each fails this.
#[test]
fn a_struct_with_a_flexible_array_member_is_no_member_and_no_element() {
    let checked = checked(
        "struct F {\n    int n;\n    int a[];\n};\nstruct G {\n    struct F f;\n    int m;\n};\nstruct F arr[2];\nstruct H {\n    struct F *p;\n    int k[3];\n};\nstruct H harr[2];\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "member `f` has type `struct F`, which has a flexible array member",
            "an array cannot have `struct F` as its element, which has a flexible array member",
        ]
    );
    assert_eq!(checked.codes(), ["SC0314", "SC0309"]);
}

/// A struct is an operand of no operator `binary`, `unary` or `increment`
/// asks, and no condition (C17 6.5.3 to 6.5.15, 6.8.4 p1, 6.8.5 p2): each
/// in a program of its own, with the message, the code and the primary
/// label written out. It is assigned, passed and chosen between only beside
/// a struct of its own tag (6.5.16.1 p1, 6.5.2.2 p2, 6.5.15 p3).
///
/// A row is `(statement, message, code, primary label)`, and an empty
/// message is a statement that must stay silent.
///
/// Mutation: let `&&` and `||` take anything but `void`, as they did; their
/// rows go silent. Mutation: answer a struct on the left `false` in
/// `report_operands`; `s * 1` points at `1`. Mutation: let `!`, `~` or `+`
/// take a struct; its row goes silent. Mutation: let `==` or `<` take two
/// structs; its row goes silent. Mutation: answer `Some(true)` for a struct
/// place in `assignable` or `compound_assignable`; `s = 1` or `s += 1` goes
/// silent. Mutation: pass `|_, _| true` for `same_struct` in
/// `conditional`; `c ? s : t` goes silent. Mutation: refuse a struct as an
/// `if` condition only; the `while`, `for` and `?:` rows go silent.
/// Mutation: report the struct arm of `increment` under another code; its
/// row fails. Each fails this.
#[test]
fn a_struct_is_an_operand_of_no_operator_and_is_held_to_its_tag() {
    for (statement, message, code, primary) in [
        (
            "x = s && 1;",
            "`&&` cannot take `struct S` and `int`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = 1 || s;",
            "`||` cannot take `int` and `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = s == u;",
            "`==` cannot take `struct S` and `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = s < u;",
            "`<` cannot take `struct S` and `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = s * 1;",
            "`*` cannot take `struct S` and `int`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = 1 * s;",
            "`*` cannot take `int` and `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = !s;",
            "`!` cannot take `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = ~s;",
            "`~` cannot take `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = +s;",
            "`+` cannot take `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "s++;",
            "`++` cannot take `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "s = 1;",
            "cannot assign `int` to `struct S`",
            "SC0302",
            "this is `int`",
        ),
        (
            "s += 1;",
            "`+=` cannot take `struct S` and `int`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "s = c ? s : t;",
            "`?:` cannot take `struct S` and `struct T`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = c ? s : 1;",
            "`?:` cannot take `struct S` and `int`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "while (s) {}",
            "`while` cannot take `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "for (; s;) {}",
            "`for` cannot take `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "x = s ? 1 : 2;",
            "`?:` cannot take `struct S`",
            "SC0306",
            "this is `struct S`",
        ),
        (
            "h(t);",
            "cannot pass `struct T` to a parameter of type `struct S`",
            "SC0302",
            "this is `struct T`",
        ),
        ("s = c ? s : u;", "", "", ""),
        ("s = u;", "", "", ""),
        ("h(u);", "", "", ""),
    ] {
        let checked = checked(&format!(
            "struct S {{
    int a;
}};
struct T {{
    int b;
}};
void h(struct S s);
int f(struct S s, struct T t, struct S u, int c) {{
    int x;
    {statement}
    return x;
}}
"
        ));

        if message.is_empty() {
            assert_eq!(checked.messages(), Vec::<&str>::new(), "{statement}");
        } else {
            assert_eq!(checked.messages(), [message], "{statement}");
            assert_eq!(checked.codes(), [code], "{statement}");
            assert_eq!(checked.labels().first(), Some(&primary), "{statement}");
        }
    }
}

/// Every step of a pointer to a struct asks whether the struct is complete
/// where the step is written, at each of the places one is spelled: `+` on
/// either side, `-` by an integer and between two pointers, `++`, `[]` on
/// either side, and `+=`. `B` closes after `A`, so its step is numbered
/// apart from the first definition's.
///
/// Mutation: pass `|_| true` for completeness at any one of those places;
/// its row in `f` goes silent, or for `+=` in `compound_assignment`, which
/// says why, loses its note. `p - p` asks both pointees, and either alone
/// still refuses one struct, so the mutation there is to both. Mutation:
/// record every definition as closed at step 0 in `close_definition`; `B`
/// is complete in `f`, and every row goes silent. Either fails this.
#[test]
fn every_step_of_a_struct_pointer_asks_completeness_where_it_is_written() {
    let steps = "    p + 1;\n    1 + p;\n    p - 1;\n    p - p;\n    p++;\n    p[0];\n    0[p];\n    p += 1;\n";
    let checked = checked(&format!(
        "struct A;\nstruct B;\nstruct B *p;\nstruct A {{\n    int a;\n}};\nvoid f(void) {{\n{steps}}}\nstruct B {{\n    int b;\n}};\nvoid g(void) {{\n{steps}}}\n"
    ));

    assert_eq!(
        checked.messages(),
        [
            "`+` cannot take `struct B *` and `int`",
            "`+` cannot take `int` and `struct B *`",
            "`-` cannot take `struct B *` and `int`",
            "`-` cannot take `struct B *` and `struct B *`",
            "`++` cannot take `struct B *`",
            "`[]` cannot take `struct B *` and `int`",
            "`[]` cannot take `int` and `struct B *`",
            "`+=` cannot take `struct B *` and `int`",
        ]
    );
    let note = |clause: &str| {
        format!(
            "a pointer steps by the size of what it points to, and a struct that is not complete here has no size (C17 {clause})"
        )
    };
    assert_eq!(
        checked.notes(),
        [
            note("6.5.6 p2"),
            note("6.5.6 p2"),
            note("6.5.6 p3"),
            note("6.5.6 p3"),
            note("6.5.2.4 p2"),
            note("6.5.2.1 p1"),
            note("6.5.2.1 p1"),
            note("6.5.16.2 p1"),
        ]
    );
}

/// A struct defined in a declaration's specifier is complete in every
/// length its declarator writes, since the specifier is written first: `p
/// + 1` in `arr`'s length steps a complete `struct S *` (C17 6.7.2.1 p8).
///
/// Mutation: drop the step in `Resolver::walk` that walks the specifier's
/// definition first; the step is refused as over an incomplete struct, and
/// this fails.
#[test]
fn a_struct_defined_in_a_specifier_is_complete_in_its_declarators_lengths() {
    let checked = checked(
        "void f(void) {\n    struct S *p;\n    struct S {\n        int a;\n    } arr[(p + 1, 2)];\n}\n",
    );

    assert_eq!(checked.messages(), Vec::<&str>::new());
}

/// A member's array length is a constant, in a block as at file scope,
/// since a member has no variably modified type (C17 6.7.6.2 p2, 6.7.2.1
/// p9); and a definition shared by two declarators has its members asked
/// once.
///
/// Mutation: hold a member to what a block's array is held to; the first
/// row goes silent. Mutation: drop the member's own message; both say what
/// a file-scope array is told. Mutation: walk a definition's members once
/// per declarator in `check_declarators`; `a[0]` is reported twice, and a
/// chain of such definitions is walked in time exponential in its depth.
/// Each fails this.
#[test]
fn a_member_is_held_to_what_c_says_of_a_member_once_per_definition() {
    let checked = checked(
        "void f(int n) {\n    struct S {\n        int a[n];\n    } s;\n}\nint m;\nstruct T {\n    int b[m];\n};\nstruct U {\n    int a[0];\n} x, y;\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "a member of a struct cannot have an array length that is not a constant",
            "the length of an array is greater than zero",
            "a member of a struct cannot have an array length that is not a constant",
        ]
    );
}

/// An lvalue of a struct not complete where it is written is reported
/// wherever its value is read (C17 6.3.2.1 p2), and as unmodifiable where
/// it is assigned to or stepped (6.5.16 p2, 6.5.3.1 p1, with 6.3.2.1 p1).
/// It is not reported as the operand of `&` or the left of `.`, which read
/// nothing, the second already refused for its struct. A subscript is
/// reported only where an untyped operand kept its own check from asking.
/// Below the definition the same statements are not reported, `s`
/// included, though it was declared above it.
///
/// Mutation: skip the pass; every `SC0315` but the member's goes. Mutation:
/// do not exclude `&`'s operand or `.`'s base; `&*p`, `&s` or `(*p).a` is
/// reported. Mutation: word a place as a value, or count only a plain `=`
/// or no `++` as one; the `*p =`, `s =`, `*p +=` or `++*p` row changes.
/// Mutation: ask a subscript with both operands typed too; `p[0]` is
/// reported twice, beside its own report. Mutation: never ask a subscript;
/// `p[i - j]` goes silent. Mutation: ask completeness where the type was
/// written, with `Resolution::complete`; `s` is reported below the
/// definition. Each fails this.
#[test]
fn an_incomplete_struct_is_reported_where_its_value_is_read_or_assigned() {
    let checked = checked(
        "struct S;
struct S s;
struct S t;
void k(struct S v);
int f(struct S *p, struct S *q, int c, int *i, int *j) {
    *p = *q;
    s = t;
    0, *p;
    c ? *p : *q;
    k(s);
    k(p[i - j]);
    *p += *q;
    ++*p;
    p[0];
    (*p).a;
    return &*p != 0 && &s != 0;
}
struct S {
    int a;
};
int g(struct S *p, struct S *q, int c) {
    *p = *q;
    s = t;
    c ? *p : *q;
    k(s);
    return &*p != 0;
}
",
    );

    let value = "`struct S` is not complete here, and its value cannot be read";
    let modified = "`struct S` is not complete here, and cannot be modified";
    assert_eq!(
        checked.messages(),
        [
            "`+=` cannot take `struct S` and `struct S`",
            "`++` cannot take `struct S`",
            "`[]` cannot take `struct S *` and `int`",
            "`struct S` is not complete here, and has no members yet",
            modified,
            value,
            modified,
            value,
            value,
            value,
            value,
            value,
            value,
            modified,
            value,
            modified,
        ]
    );
    // This pass reports last, one note each.
    let notes = checked.notes();
    assert_eq!(
        notes[notes.len() - 12..],
        [
            "C17 6.5.16 p2 and 6.3.2.1 p1",
            "C17 6.3.2.1 p2",
            "C17 6.5.16 p2 and 6.3.2.1 p1",
            "C17 6.3.2.1 p2",
            "C17 6.3.2.1 p2",
            "C17 6.3.2.1 p2",
            "C17 6.3.2.1 p2",
            "C17 6.3.2.1 p2",
            "C17 6.3.2.1 p2",
            "C17 6.5.16 p2 and 6.3.2.1 p1",
            "C17 6.3.2.1 p2",
            "C17 6.5.3.1 p1 and 6.3.2.1 p1",
        ]
    );
}

/// An initializer at file scope is a constant expression (C17 6.7.9 p4): an
/// integer constant, a null pointer constant, an address constant (6.6 p9),
/// or one of those plus or minus an integer constant (6.6 p7). An address
/// constant may be made through `[]`, `.`, `->`, `&` and `*`, and an array
/// or function designated through them is one; `0 && (1, 2)` is a constant,
/// since 6.6 p3 lets an operand that is not evaluated hold a comma. A name
/// of an object, a read through one, a comma, a call, `?:` on addresses, a
/// pointer that is not static, and an address plus or minus a name are not.
/// One without a defined value is told so.
///
/// Mutation: skip the pass; every refused row goes silent. Mutation: accept
/// any `&e` without asking what it designates; `&arr[x]`, `&*s.p` and
/// `&p[1]` go silent. Mutation: let `+` or `-` take any right operand;
/// `&x + x` or `&x - x` goes silent.
/// Mutation: drop either order of `+` or of `[]`; `1 + arr` or `&1[arr]` is
/// reported. Mutation: ask only a name of array type; `s.m`, `a2[1]`,
/// `&a2[1][2]` and `*f` are reported. Mutation: let `->` take a designator
/// rather than an address; `&ps->q` goes silent. Mutation: drop `->`;
/// `&(&sa[1])->q` is reported. Mutation: ask an unevaluated operand only
/// for a value; `0 && (1, 2)` is reported. Mutation: word every one as not
/// constant; `1 / 0`'s message changes. Each fails this.
#[test]
fn an_initializer_at_file_scope_is_held_to_a_constant_expression() {
    let checked = checked(
        "int x;\nint arr[3];\nint a2[2][3];\nint *p;\nstruct S {\n    int *p;\n    int q;\n    int m[4];\n} s;\nstruct S sa[2];\nstruct S *ps;\nint f(void);\nint g = x;\nint h = *s.p;\nint m = (1, 2);\nint c = f();\nint *pe = &arr[x];\nint *pt = 1 ? &x : &x;\nint *pu = &*s.p;\nint *pv = &p[1];\nint *pw = &ps->q;\nint *px = &x + x;\nint *py = &x - x;\nint z = 0 && x;\nint d = 1 / 0;\nint k = 1 + 2;\nint o = 0 && (1, 2);\nint *pa = &x;\nint *pq = 0;\nint *pr = &x + 1;\nint *pm = &x - 1;\nint *pg = 1 + arr;\nint *pb = arr;\nint *pc = &arr[1];\nint *pi = &1[arr];\nint *pd = &s.q;\nint *pj = s.m;\nint *pk = a2[1];\nint *pl = &a2[1][2];\nint *pn = &(&sa[1])->q;\nint (*pf)(void) = f;\nint (*po)(void) = *f;\nint *ph = &*&x;\n",
    );

    assert_eq!(
        checked.messages(),
        [
            "the initializer of `g` is not a constant",
            "the initializer of `h` is not a constant",
            "the initializer of `m` is not a constant",
            "the initializer of `c` is not a constant",
            "the initializer of `pe` is not a constant",
            "the initializer of `pt` is not a constant",
            "the initializer of `pu` is not a constant",
            "the initializer of `pv` is not a constant",
            "the initializer of `pw` is not a constant",
            "the initializer of `px` is not a constant",
            "the initializer of `py` is not a constant",
            "the initializer of `z` is not a constant",
            "the initializer of `d` has no defined value",
        ]
    );
    assert_eq!(checked.codes(), ["SC0317"; 13]);
}

/// An initializer at file scope this stage left untyped, `*arr` being one,
/// is declined as this compiler's gap rather than passed over, since the
/// lowering drops it unread and the program would build. A function given
/// one is `SC0310`'s alone. A chain of `+` as long as the source makes it
/// is walked without running out of stack.
///
/// Mutation: pass an untyped initializer over; `g` goes silent and the run
/// would exit 0. Mutation: ask a function's initializer too; `f2` gets a
/// second report. Mutation: walk `address_constant` by recursion; the
/// chain overflows the stack. Each fails one of these.
#[test]
fn an_initializer_at_file_scope_this_stage_cannot_ask_is_still_refused() {
    let untyped = checked("int arr[3];\nint g = *arr;\n");
    assert_eq!(
        untyped.messages(),
        ["cannot check whether the initializer of `g` is a constant"]
    );
    assert_eq!(untyped.codes(), ["SC0304"]);

    let function = checked("int x;\nint f2(void) = x;\n");
    assert_eq!(function.codes(), ["SC0310"]);

    let chain = checked(&format!("int x;\nint *g = &x{};\n", " + 1".repeat(50_000)));
    assert_eq!(chain.messages(), Vec::<&str>::new());
}
