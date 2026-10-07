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

/// C17 6.5.15's rule for a conditional, as far as this stage answers it.
///
/// Two arms of one type make that type; the rest of p5 needs the usual
/// arithmetic conversions and is not answered.
///
/// Mutation: have the `Expr::Conditional` arm answer `None`. The first two
/// rows lose their type and this fails. Before it existed the whole arm
/// could be deleted with the suite green: the three corpus cases that
/// write `?:` never assign one or return one, so nothing observed its
/// type.
#[test]
fn a_conditional_has_a_type_when_both_its_arms_agree() {
    let checked = checked(
        "int main(void) {
    int *p;
    void *v;
    int x;
    x = 1 ? 2 : 3;
    p = 1 ? p : p;
    1 ? p : v;
    return 0;
}
",
    );

    assert_eq!(checked.messages(), Vec::<&str>::new());

    for (text, spelling) in [
        ("1 ? 2 : 3", "int"),
        ("1 ? p : p", "int *"),
        ("1 ? p : v", "?"),
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
/// `v` points at. Every subscript row has the pointer as its base,
/// because `1[v]` and `g[1]` have no type to keep; `Checker::subscript`
/// says why.
///
/// Mutation: have `increment` answer `None` after its report. The `++`
/// and `--` rows fail. Mutation: have `subscript` answer `None` after its
/// report. The `[]` rows fail. Nothing else in the suite noticed either.
#[test]
fn a_refused_increment_or_subscript_keeps_its_type() {
    for (code, written, ty) in [
        ("v++;", "v++", "void *"),
        ("--v;", "--v", "void *"),
        ("v[1];", "v[1]", "void"),
        ("u[0];", "u[0]", "int[]"),
    ] {
        let checked = checked(&format!(
            "int main(void) {{
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

/// An initializer is held to the rule for a plain `=`, C17 6.7.9 p11, at
/// file scope and in a block, with the words a declaration was written in.
///
/// One program with the silences and the reports together, so that the
/// silences are asserted against a run that does report and cannot pass by
/// checking nothing. The silences are a null pointer constant, and a
/// `void *` both ways, which is an implicit conversion with no cast in the
/// grammar.
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
        ]
    );
    assert_eq!(
        checked.labels(),
        [
            "this is `int`",
            "this holds `int *`",
            "this is `int *`",
            "this holds `char *`",
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
#[test]
fn a_name_that_resolved_to_nothing_is_reported_once() {
    for value in ["nowhere", "nowhere + 1", "1 + nowhere", "-nowhere"] {
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
