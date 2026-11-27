//! From the typed AST to the Safety IR.
//!
//! The first reader of what [`crate::sema`] and [`crate::types`] work out, and
//! the last place a program is still shaped the way it was written. Everything
//! after this reads blocks and places.
//!
//! **What cannot be lowered is reported, and its function is left a
//! declaration.** The alternative is a definition with a hole in it, and
//! nothing in the IR says a hole is there: an analysis walking such a function
//! would conclude about code it never saw. A declaration says exactly what is
//! true, that the body is not here, which is a thing [`Function::declaration`]
//! already exists to say.
//!
//! Everything it refuses is `SC0304`: an expression the frontend could not
//! type, a type the IR cannot hold, a name it cannot reach, a constant it
//! cannot read, and a call with no function to name. One code rather than
//! five, for the reason `types.rs` gives for
//! `MISMATCH`: what differs between them is the message, and a reader
//! filtering on the code wants to know the IR could not be built rather than a
//! list of ways that can happen.
//!
//! `parser.rs` splits its two codes along a different line, and the difference
//! is worth naming: `TOO_DEEP` is separate from `EXPECTED` because a program
//! nested too deeply is well formed and refused, while an unexpected token is
//! a program nobody wrote correctly. Every case here is the first kind, so the
//! split has nothing to divide.

use safec_ir::ir::Rank;
use std::collections::{HashMap, HashSet};

use crate::ast::{
    Ast, BinOp as AstBinOp, Declaration, Expr, ExprId, ForStart, InitDeclarator, Item, Parameters,
    Stmt, StmtId, Storage, Type,
};
use crate::ast::{Nullability, Specifier, TypeId, UnOp as AstUnOp, VaOp, spell_type};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::preprocessor::cut_words;
use crate::sema::{Resolution, TagId};
use crate::types::Types;
use safec_ir::ir::DOES_NOT_RETURN;
use safec_ir::ir::{
    Between, BinOp, Bits, Block, BlockId, Called, Declared, Element, FuncId, Function, Intrinsic,
    Length, Linkage, LocalId, Operand, Operation, Origin, Parameter, Passes, Place, Projection,
    Promise, Rvalue, Shape, Signature, Static, StaticId, StaticInit, StructId, Terminator,
    TranslationUnit, Ty, TyId, UnOp,
};
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

/// Something the frontend accepted and this stage cannot express.
///
/// One code for every shape of it, for the reason `types.rs` gives for
/// `MISMATCH`: what differs between them is the message, and a reader filtering
/// on the code wants "the IR could not be built" rather than a list of ways
/// that can happen.
///
/// The type checker reports a struct under the same code, as valid C this
/// compiler cannot handle yet, and is the stage that does so: none reaches
/// this one. See `types.rs`'s `NOT_YET`.
const LOWERING: Code = Code::new("SC0304");

/// A jump from outside the scope of a variable length array to inside it,
/// which C17 6.8.6.1 p1 and 6.8.4.2 p2 make a constraint violation: the jump
/// passes the declaration that works its length out and makes its storage,
/// and nothing past it has either (#1209).
const INTO_VARIABLE_SCOPE: Code = Code::new("SC0323");

/// Two declarations of one function that disagree about a nullability
/// specifier.
///
/// Refused rather than resolved, because the declaration a caller sees is the
/// one its call is checked against, and a header that leaves the promise out is
/// how a caller in another translation unit goes unchecked while the body
/// believes it. See ADR-0037. `clang` carries it from one declaration to the
/// next in silence.
///
/// Here rather than in `types.rs`, because [`Lowering::declare_one`] is the one
/// place several declarations become one function, and a second answer to which
/// declarations are the same function would be a second place to drift.
const DISAGREEING_ANNOTATION: Code = Code::new("SC0307");

/// The report for two declarations of `name` that disagree about one
/// nullability specifier: what the first wrote and what a later one did.
///
/// One function for a parameter's and a return's, so that the two say the same
/// thing about the same mistake. `first_is` says which earlier declaration is
/// compared, "prototype" or "declaration".
///
/// The primary label goes on a specifier one of the two wrote, because it is
/// the only token either declaration has that says what differs: the later
/// one's where it wrote one, and the first one's otherwise. The other
/// declaration is pointed at by what it wrote, or by its name where it wrote
/// nothing.
fn disagreement(
    sources: &SourceMap,
    name: Span,
    first_name: Span,
    first_is: &str,
    first: Option<Nullability>,
    later: Option<Nullability>,
) -> Diagnostic {
    let (said, other, other_label) = match (first, later) {
        (Some(first), Some(said)) => (
            said,
            first.at,
            format!("the first {first_is} says `{}`", first.specifier.spelling()),
        ),
        (None, Some(said)) => (
            said,
            first_name,
            format!("the first {first_is} says nothing"),
        ),
        (Some(said), None) => (said, name, "this declaration says nothing".to_owned()),
        (None, None) => unreachable!("the two were found to differ"),
    };

    Diagnostic::error(format!(
        "the declarations of `{}` disagree about nullability",
        sources.snippet(name)
    ))
    .with_code(DISAGREEING_ANNOTATION)
    .with_label(Label::primary(
        said.at,
        format!("one declaration says `{}` here", said.specifier.spelling()),
    ))
    .with_label(Label::secondary(other, other_label))
    .with_note(
        "every declaration of a function has to agree about its nullability \
         specifiers, because each of them is a promise to the function's callers",
    )
}

/// What a written specifier promises of the pointer a function returns.
///
/// `_Nonnull` is a promise and `_Nullable` is none, which is what writing
/// nothing is below level 5. Every specifier written out, so that a third is
/// answered for here by `error[E0004]`.
fn written_promise(written: Option<Nullability>) -> Option<Promise> {
    match written {
        Some(Nullability {
            specifier: Specifier::Nonnull,
            at,
        }) => Some(Promise::Declared(at)),
        Some(Nullability {
            specifier: Specifier::Nullable,
            at: _,
        })
        | None => None,
    }
}

/// The terminator that ends a block on a statement's controlling expression:
/// a `Branch` to both arms, or a `Goto` to the one C runs where the expression
/// is a constant.
///
/// C17 6.8.4.1 p2 runs an `if`'s first arm when its controlling expression
/// "compares unequal to 0", and 6.8.5 p4 repeats a loop's body until it
/// "compares equal to 0", so a constant decides the branch before it is
/// reached. Lowered as a `Branch`, `while (1)` had an exit edge C never takes,
/// and a function that promised its result was told it may reach the end of
/// its body past a loop only a `return` leaves (#338). `for (;;)` already
/// lowers to a `Goto`, by 6.8.5.3 p2, and this is the same answer for a
/// condition that is written.
///
/// **Decided here rather than by the analyses**, so that a `Branch` keeps
/// meaning an edge to each arm, on a constant or not. IR built by another
/// frontend, or by hand, is answered as it was, which is conservatively.
///
/// **Only a constant expression that already lowered to a constant**, which
/// `constant` says: whether `types.rs` gave the condition a value. `(x, 1)` lowers to the
/// constant its right operand is, and is not a constant expression: C17 6.6 p3
/// forbids a comma operator in one, and 6.8.5 p6 lets an implementation assume
/// a loop on an expression that is not one terminates, so `while ((x, 1)) {}`
/// is not `while (1) {}` and is left a `Branch`. `1 == 1` and `!0` are
/// constant expressions, given their values by `types.rs`, and are decided
/// as `1` is. The arm not taken is still lowered, into blocks nothing reaches.
fn decided(
    condition: Operand,
    constant: bool,
    then: BlockId,
    otherwise: BlockId,
    origin: Origin,
) -> Terminator {
    match (condition, constant) {
        (Operand::Constant(value), true) => {
            Terminator::Goto(if value != 0 { then } else { otherwise })
        }
        (condition, _) => Terminator::Branch {
            condition,
            then,
            otherwise,
            origin,
        },
    }
}

/// One open scope: which it is, and the locals it has declared so far.
struct Scope {
    id: u32,
    declared: Vec<LocalId>,
}

/// The scopes open at one place in a function, outermost first, each with
/// the locals it had declared by then. What a jump compares to find which
/// storage it ends and which it begins. See ADR-0114.
type Point = Vec<(u32, Vec<LocalId>)>;

/// A labelled statement's block, and the `goto`s waiting for it.
struct Landing {
    /// Where the labelled statement begins.
    block: BlockId,
    /// The scopes open at the label, once it has been lowered.
    at: Option<Point>,
    /// Each `goto` lowered before the label: the block it ended with a jump
    /// to, still to be written, where it was, and its span.
    waiting: Vec<(BlockId, Point, Span)>,
}

/// Where a `break` and a `continue` inside one loop go. C17 6.8.6.2 and
/// 6.8.6.3.
struct Jumps {
    /// Where the loop is left: the block its condition leaves it for, whose
    /// sequence point a `break` passes with nothing pending, since a `break`
    /// is a statement and the one before it ended a full expression.
    broken: BlockId,
    /// Where the next turn begins.
    continued: Continued,
    /// How many scopes were open where the loop began, so that a jump out of
    /// its body ends every scope opened since and none before.
    depth: usize,
}

/// Where a `continue` goes, which for a `for` is a block nobody may need.
enum Continued {
    /// A block that is written whatever happens: the header of a `while`, the
    /// condition of a `do`.
    To(BlockId),
    /// The step of a `for`, reserved by the first `continue` that needs it.
    /// Reserved up front, a loop whose body cannot fall through and holds no
    /// `continue` would leave a block reserved and never written, which
    /// [`Function::blocks`] refuses.
    Step(Option<BlockId>),
    /// A `switch`, which a `continue` goes past to the loop around it
    /// (C17 6.8.6.2 p1).
    Passes,
}

/// Refuse a `sizeof` sized when it runs that is not of a variable length
/// array's own name, such as `sizeof(int[n])` or `sizeof *&a` (#1209). One
/// place, so that both arms say the same. Mutation: report nothing;
/// `a_sizeof_worked_out_when_it_runs_is_refused_where_it_is_not_of_a_name`
/// loses both refusals.
fn refuse_run_time_sizeof(span: Span, diagnostics: &mut DiagnosticSink) -> Option<()> {
    diagnostics.report(
        Diagnostic::error("cannot compile this `sizeof` yet")
            .with_code(LOWERING)
            .with_label(Label::primary(span, "sized when it runs"))
            .with_note("only `sizeof` of a variable length array's own name, or of a type a typedef named, is worked out when it runs so far"),
    );
    None
}

/// End the storage of the copies a call made of its struct arguments, once
/// it has returned. See `Lowering::argument_copies`.
fn end_copies(builder: &mut Builder, copies: &[LocalId], span: Span) {
    for &local in copies {
        builder.element(Element::StorageDead {
            origin: Origin::Generated(span),
            local,
        });
    }
}

/// A bit-field as an access reaches it: the place of its unit, the unit's
/// unsigned type, which bits of it the bit-field takes, and whether it is
/// read as signed. See `Lowering::bit_fields`.
#[derive(Clone, Debug)]
struct BitField {
    unit: Place,
    unit_ty: TyId,
    bits: Bits,
    signed: bool,
}

/// The lowest `bits` bits set, for a mask of up to 64 bits.
fn mask(bits: u64) -> i128 {
    (1i128 << bits) - 1
}

/// What a jump from `from` to `to` ends and begins, in the order
/// [`Lowering::jump`] writes them. See there, and ADR-0114.
///
/// `variable` says which locals are variable length arrays, whose lifetime
/// a backward jump to before their declaration ends (C17 6.2.4 p7): their
/// storage is made each time the declaration is reached, so one not ended
/// on such a jump is made again on top of itself and never given back.
/// Every other local of that scope lives through the whole block (p6) and
/// is left alone. Mutation: end none of them; the run of
/// `a_variable_length_array_is_sized_when_declared` overflows the stack.
fn markers(
    from: &Point,
    to: &Point,
    variable: impl Fn(LocalId) -> bool,
) -> (Vec<LocalId>, Vec<LocalId>) {
    let shared = from
        .iter()
        .zip(to)
        .take_while(|((left, _), (right, _))| left == right)
        .count();
    let left = shared.checked_sub(1).map_or(&[][..], |deepest| {
        let (passed, reached) = (&from[deepest].1, &to[deepest].1);
        passed.get(reached.len()..).unwrap_or(&[])
    });
    let ended = from[shared..]
        .iter()
        .rev()
        .flat_map(|(_, declared)| declared.iter().rev().copied())
        .chain(left.iter().rev().copied().filter(|&local| variable(local)))
        .collect();
    let skipped = shared.checked_sub(1).map_or(&[][..], |deepest| {
        let (passed, reached) = (&from[deepest].1, &to[deepest].1);
        reached.get(passed.len()..).unwrap_or(&[])
    });
    let entered = to[shared..].iter().flat_map(|(_, declared)| declared);
    let begun = skipped.iter().chain(entered).copied().collect();
    (ended, begun)
}

/// End the storage of the locals one scope declared, where it closes.
///
/// Reverse order of declaration, which is the order a C++ destructor would run
/// in and costs nothing to get right while the list is being written. Only
/// where the end is reachable: a scope left only by `return` has its storage
/// ended by the `return`, which ends every scope it leaves, so nothing is
/// left to end here.
///
/// `span` is the statement the scope belongs to, a compound or a `for`, and
/// the marker is `Generated` at its last byte. Nobody wrote "end this
/// storage": the `}` or the end of the loop is what it exists because of,
/// which is what `Generated` means and what stops a diagnostic quoting a
/// block of source back as if a user had asked for this. The lowering only
/// ever sees a tree that parsed without a word said about it, so that byte
/// is the statement's own.
fn end_storage(builder: &mut Builder, declared: &[LocalId], span: Span) {
    if !builder.reachable() {
        return;
    }
    for &local in declared.iter().rev() {
        builder.element(Element::StorageDead {
            origin: Origin::Generated(Span::new(span.file(), span.end() - 1, span.end())),
            local,
        });
    }
}

/// Build the IR of one translation unit.
///
/// Every function that can be lowered is, whatever the ones beside it did: a
/// program is not one thing that fails, and a caller of a function this stage
/// refused still resolves, because the refusal leaves a declaration behind.
///
/// `nonnull_returns_by_default` is level 5's default, that a pointer is not null
/// unless it is written `_Nullable` (ADR-0050). **The level is resolved here
/// and not carried into the IR**: what reaches the IR is the promise it made,
/// [`Promise::Defaulted`], so nothing that reads an IR asks what level a run
/// is at, which is ADR-0011's boundary.
pub fn lower(
    sources: &SourceMap,
    ast: &Ast,
    resolution: &Resolution,
    types: &Types,
    target: Target,
    nonnull_returns_by_default: bool,
    diagnostics: &mut DiagnosticSink,
) -> TranslationUnit {
    let mut lowering = Lowering {
        sources,
        ast,
        nonnull_returns_by_default,
        resolution,
        types,
        // The one thing this stage learns about the machine, and it only
        // passes it on: what a type is worth is asked of the unit, below this.
        unit: TranslationUnit::new(target),
        locals: HashMap::new(),
        scopes: Vec::new(),
        next_scope: 0,
        landings: HashMap::new(),
        stacked: None,
        labeled: false,
        targets: Vec::new(),
        functions: HashMap::new(),
        prototypes: HashMap::new(),
        return_nullability: HashMap::new(),
        structs: HashMap::new(),
        written: HashMap::new(),
        laying_out: HashSet::new(),
        refused_structs: HashSet::new(),
        memcpy: None,
        memset: None,
        vla_types: HashMap::new(),
        writes_below: writes_below(ast),
        typedef_lengths: HashMap::new(),
        typedef_locals: Vec::new(),
        argument_copies: Vec::new(),
        refused: HashSet::new(),
        statics: HashMap::new(),
        file_statics: HashMap::new(),
        block_statics: HashMap::new(),
        internal: HashSet::new(),
        strings: HashMap::new(),
        static_levels: HashMap::new(),
        pending: HashMap::new(),
        top_level: false,
        discarding: None,
        root: None,
    };

    lowering.declare(diagnostics);
    lowering.define(diagnostics);
    lowering.lay_out_stepped(diagnostics);
    lowering.unit
}

/// One translation unit, being lowered.
struct Lowering<'a> {
    sources: &'a SourceMap,
    ast: &'a Ast,
    /// Level 5's default, which [`lower`] says why this stage resolves.
    nonnull_returns_by_default: bool,
    resolution: &'a Resolution,
    types: &'a Types,
    unit: TranslationUnit,
    /// The local a declared name means, keyed by the span of that name.
    ///
    /// A [`crate::sema::Binding`] says a name and a type and not which function
    /// it belongs to, and a resolution is keyed on the use site, so a
    /// declaration cannot be asked which binding it made. A name's span can:
    /// it is where the name was declared, so it is one per declaration, and a
    /// use reaches it through the binding it resolved to.
    locals: HashMap<Span, LocalId>,
    /// The IR type of each variable length array local of the function being
    /// lowered, whose length is a local of its own (#1209): what a use of its
    /// name is typed as. **Emptied where each function begins**, since a
    /// `LocalId` means a local of one function only, as `Builder::bit_fields`
    /// learned (#1206 review).
    vla_types: HashMap<LocalId, TyId>,
    /// Whether each expression, or anything below it, assigns, increments or
    /// decrements, by [`ExprId::index`]. See [`writes_below`].
    writes_below: Vec<bool>,
    /// The local each variable length array type a typedef declared has its
    /// length worked out into, and where: what an object or a `sizeof` of
    /// the type reads (#1294).
    ///
    /// **Keyed by the length expression**, not by the array's type node: an
    /// alias, `typedef T U;`, hands on T's own node, and a qualified one,
    /// `const T`, a copy of it with the same length, so a node key worked
    /// T's length out again at the alias and took over T's entry, a wrong
    /// size and a jump into T's scope accepted. The expression is what C17
    /// 6.7.8 p3 evaluates, and every share and copy keeps it. Emptied where
    /// each function begins, as `vla_types` is.
    typedef_lengths: HashMap<ExprId, (LocalId, Span)>,
    /// Every local a typedef's length was worked out into in this function,
    /// for what a jump asks: never taken from, so that an entry a later
    /// typedef could replace in `typedef_lengths` is still found.
    typedef_locals: Vec<(LocalId, Span)>,
    /// The scopes that are open, innermost last, each holding the locals it
    /// declared directly: a compound statement's, and a `for`'s whose first
    /// clause is a declaration (C17 6.8.5 p5).
    ///
    /// The function's body is the first, so a scope narrower than the function
    /// is one at index 1 or beyond. Only those get storage markers: a local
    /// declared in the body itself lives exactly as long as the frame, which
    /// every consumer already knows, and ADR-0012 argues why that is enough.
    scopes: Vec<Scope>,
    /// The id the next scope opened gets, so that two points of a function
    /// can be compared by which scopes they have open. See [`Point`].
    next_scope: u32,
    /// Where each labelled statement lowered so far, or named by a `goto`
    /// before it was, begins. See ADR-0114.
    landings: HashMap<StmtId, Landing>,
    /// The block of the label just lowered, while its statement is another
    /// label, which takes it rather than reserving its own. See
    /// [`Lowering::land`].
    stacked: Option<BlockId>,
    /// Whether the function being lowered has had a label lowered yet, which
    /// is whether a declaration lowered now can be reached again by a
    /// backward `goto`. See the `Declaration` arm.
    labeled: bool,
    /// Where a `break` and a `continue` go, innermost loop last.
    ///
    /// Pushed by each loop around its body and popped on every path out of
    /// it, lowered or not, as [`Lowering::scopes`] is: a target left behind
    /// by a body that failed would send a `break` in the next function into a
    /// block of this one.
    targets: Vec<Jumps>,
    /// Which function a name at file scope became, keyed by the name itself.
    ///
    /// Not by span, the way a local is: a prototype and the definition that
    /// follows it are two declarations of one function, at two spans, and a
    /// call resolves to whichever the resolver had in scope. Keyed by span,
    /// `int add(int, int);` and the `add` below it become two functions, and a
    /// call written between them reaches the one with no body. Keyed by the
    /// text, they are one, which is what C means by them and what
    /// `sema.rs::lookup` already compares.
    functions: HashMap<String, FuncId>,
    /// The first prototype each file-scope name was declared with: where its
    /// name was, and the nullability specifier each of its parameters wrote.
    ///
    /// Not [`Lowering::functions`]' entry, which is the first *declaration*:
    /// `void g();` declares no parameters (C17 6.7.6.3 p14), so every later
    /// declaration would agree with it, and `void g(); void g(int *p);` above a
    /// `_Nonnull` definition passed in silence. That was found by review.
    /// See [`DISAGREEING_ANNOTATION`].
    prototypes: HashMap<String, (Span, Vec<Option<Nullability>>)>,
    /// The first declaration each file-scope function name had: where its name
    /// was, and the nullability specifier on the pointer it returns.
    ///
    /// The first declaration and not the first prototype, unlike
    /// [`Lowering::prototypes`]: `void g();` declares no parameters and does
    /// declare what `g` returns, so it is a declaration of the return to agree
    /// with like any other.
    return_nullability: HashMap<String, (Span, Option<Nullability>)>,
    /// The IR struct each struct tag became, so that one struct type is one
    /// `Ty::Struct` however many times it is written. See [`Lowering::ty`].
    structs: HashMap<TagId, StructId>,
    /// Where each IR struct was first written and the C type that wrote it,
    /// so that one only a pointer reaches can be laid out once the unit is
    /// lowered. See [`Lowering::lay_out_stepped`].
    written: HashMap<StructId, (Span, TypeId)>,
    /// The tags whose members are being laid out now. See
    /// [`Lowering::struct_of`].
    laying_out: HashSet<TagId>,
    /// The tags whose layout was refused, said once. See
    /// [`Lowering::struct_of`].
    refused_structs: HashSet<TagId>,
    /// The `memcpy` a struct copy calls, made once. See
    /// [`Lowering::copy_struct`].
    memcpy: Option<FuncId>,
    /// The `memset` an array initialized from a shorter string literal
    /// calls, made once. See [`Lowering::store_string`].
    memset: Option<FuncId>,
    /// The copies made of the struct arguments of the calls being lowered, in
    /// the order they were made. A call takes its own off the top when it
    /// finishes, since every call nested in its arguments has finished and
    /// taken its own first, and ends their storage after it returns.
    argument_copies: Vec<LocalId>,
    /// The names whose signature this stage could not read.
    ///
    /// Reported once, where the declaration is. A call to one of them is not
    /// reported again: the caller wrote an ordinary call and the fault is in a
    /// declaration somewhere else.
    refused: HashSet<String>,
    /// The object with static storage duration each declaration names, by the
    /// span of the name it declares, which is what a use resolves to. Every
    /// file-scope declaration of one object, and every block-scope `extern`
    /// naming it, maps to the same one. See ADR-0115.
    statics: HashMap<Span, StaticId>,
    /// The file-scope objects by spelling, which is how a block-scope
    /// `extern` finds the one it names (C17 6.2.2 p4).
    file_statics: HashMap<String, StaticId>,
    /// How many block-scope `static` objects each spelling has had in each
    /// function, so that each gets a name of its own.
    block_statics: HashMap<String, u32>,
    /// The functions some file-scope declaration of which says `static`,
    /// which C17 6.2.2 p3 gives internal linkage.
    internal: HashSet<String>,
    /// The static each string literal is, by the literal: one object however
    /// many times the expression is evaluated (C17 6.4.5 p6 gives it static
    /// storage duration). See [`Lowering::string`].
    strings: HashMap<ExprId, StaticId>,
    /// Per static, the levels at which a value stored in it points at
    /// `const`, so that the temporary [`Lowering::reach_static`] reaches it
    /// through carries them. A string literal holds no pointer and has none.
    static_levels: HashMap<StaticId, u32>,
    /// Where a `&&`, `||` or `?:` puts its answer, and where control rejoins.
    ///
    /// Filled when the first operand has been evaluated and read when the last
    /// one has. Keyed by the expression, because the two moments are two turns
    /// of the loop in [`Lowering::value`] rather than two lines of one
    /// function.
    pending: HashMap<ExprId, Pending>,
    /// Whether a sequence point reached from here sequences the whole of the
    /// full expression being lowered.
    ///
    /// C17 6.5 p3 leaves the operands of most operators unsequenced, so a
    /// sequence point inside one of them says nothing about the others:
    /// `*p + (free(p), 0)` has a comma, and 6.5.17 p2 sequences the free
    /// before the `0`, but neither is ordered against the read of `*p`.
    /// Recording that comma as an [`Element::Sequenced`] would tell every
    /// analysis that the free happens first, which C has not said.
    ///
    /// True at the root of a full expression, and kept only through the
    /// operands C sequences. See ADR-0022, and [`sequences`] for the list.
    top_level: bool,
    /// The root of the full expression [`Lowering::effect`] is lowering,
    /// whose value is discarded. An assignment there reads nothing back:
    /// C17 6.5.16 p3 gives the expression a value and asks no read of the
    /// left operand, and the read-back of `*p = 1` read `p` after the store
    /// may have written it (#1137).
    discarding: Option<ExprId>,
    /// The full expression being lowered, which is the expression
    /// [`Lowering::value`] was last asked for, and `None` while a parameter's
    /// length is, which is not one.
    ///
    /// Not [`Lowering::top_level`], which stays true through operands C
    /// sequences and so through the right operand of a comma. Read by the call
    /// that decides whether it has a continuation.
    root: Option<ExprId>,
}

/// A branch a value is waiting on.
struct Pending {
    /// Where every arm writes its answer, or nothing for a `?:` whose type is
    /// `void`, whose arms have no value to write. A `&&` or `||` always has
    /// one.
    answer: Option<LocalId>,
    /// Where the arms come back together.
    join: BlockId,
    /// Which block the `else` of a `?:` starts at.
    otherwise: Option<BlockId>,
}

/// One function, being built.
struct Builder {
    function: Function,
    /// The block being written into, or none where control cannot arrive.
    ///
    /// `None` after a terminator, which is what makes a `return` in the middle
    /// of a body need no special case: the statements after it open a block of
    /// their own that nothing jumps to. Leaving a block reserved and unfilled
    /// is what [`Function::blocks`] panics about, so the block is opened when
    /// something is written into it rather than when the last one ended.
    block: Option<BlockId>,
    /// What has been written into the block since it was opened.
    elements: Vec<Element>,
    /// The temporaries a bit-field is taken into where it is accessed, and
    /// where its bits are (#1206): a read reads the temporary, and a write
    /// to it is put back into the unit by `Lowering::write_back`. Keyed by
    /// the temporary, which is made fresh for each access.
    ///
    /// **Here, with the function's locals**, since a `LocalId` means a
    /// local of one function only: kept on the whole unit, a temporary of
    /// one function's bit-field was a plain local of the next, and an
    /// assignment to it there was put back into memory it never named
    /// (#1206 review). Mutation: keep one table for the unit;
    /// `a_bit_field_is_read_and_written_within_its_unit` in `tests/run.rs`
    /// fails at `unrelated`.
    bit_fields: HashMap<LocalId, BitField>,
}

impl Builder {
    /// The block being written into, opening one if there is none.
    fn open(&mut self) -> BlockId {
        match self.block {
            Some(block) => block,
            None => {
                let block = self.function.reserve_block();
                self.block = Some(block);
                block
            }
        }
    }

    /// Write an operation into the current block.
    fn push(&mut self, operation: Operation) {
        self.element(Element::Assign(operation));
    }

    /// Write any element into the current block.
    fn element(&mut self, element: Element) {
        self.open();
        self.elements.push(element);
    }

    /// Say that nothing before here can happen after anything following it.
    ///
    /// `Generated`, because nobody writes an element: the `;` or the `,` or
    /// the `&&` is what this exists because of.
    ///
    /// `span` is the expression the point falls **after** and not that
    /// operator's own, which the tree does not carry. See [`Element::Sequenced`]
    /// for what that costs a reader of the artifact.
    fn sequenced(&mut self, span: Span) {
        self.element(Element::Sequenced {
            origin: Origin::Generated(span),
        });
    }

    /// Say that a value nobody wanted was evaluated, where the evaluation is
    /// the only thing that happened.
    ///
    /// C17 6.8.3 p2 evaluates an expression statement as a void expression and
    /// 6.3.2.2 discards what it yields, and three other places do the same: a
    /// `for` initialiser, a `for` step, and the left operand of a comma under
    /// 6.5.17 p2. A cast to `void` is a fourth and cannot be written, because
    /// the parser does not read a cast; whoever adds one arrives here.
    ///
    /// **Only a place reached through a projection.** Anything that needed
    /// computing left the operation that computed it, and a bare name is left
    /// alone because no check here would read an element saying it was
    /// evaluated, not because evaluating one is always defined: 6.3.2.1 p2's
    /// last sentence makes reading an uninitialised object undefined where its
    /// address was never taken, and that belongs to an axis with no check.
    /// What remains is the shape that was silent: `*p;` after a free reported
    /// nothing at all, because the place went into an operand nobody read
    /// rather than into an element. [`Element::Evaluate`] carries the rest.
    fn discarded(&mut self, value: Operand, at: Span) {
        let Operand::Copy(place) = value else {
            return;
        };
        match place.shape() {
            Shape::Local => return,
            Shape::Deref(_) | Shape::Indexed => {}
        }
        self.element(Element::Evaluate {
            place,
            origin: Origin::Written(at),
        });
    }

    /// End the current block, and leave none open.
    fn end(&mut self, terminator: Terminator) {
        let block = self.open();
        let elements = std::mem::take(&mut self.elements);
        self.function.fill_block(
            block,
            Block {
                elements,
                terminator,
            },
        );
        self.block = None;
    }

    /// Begin writing into a block whose id was reserved earlier.
    ///
    /// # Panics
    ///
    /// If a block is still open. Two open blocks would mean operations written
    /// into whichever was current, which is the bug this shape exists to make
    /// impossible.
    fn switch(&mut self, block: BlockId) {
        assert!(self.block.is_none(), "a block was left open");
        self.block = Some(block);
    }

    /// Begin a block a branch leads to, with the sequence point the expression
    /// that decided ended at.
    ///
    /// **After the branch and not before it, because the branch is what reads
    /// the controlling expression.** `if (*p)` needs no temporary, so the
    /// dereference is carried by [`Terminator::Branch`] itself; a marker
    /// written before the terminator then sits on the wrong side of the read it
    /// is about, and nothing says the read happens before the body. C17 6.8 p4 is
    /// the clause: it lists a controlling expression among the full expressions
    /// and puts a sequence point at the end of one, and the end of an
    /// expression is after the evaluation the terminator performs. Cited alone,
    /// because the paragraphs that introduce the term do no work here and a
    /// citation that is not load-bearing is one nobody checks. See ADR-0023.
    ///
    /// Every arm, because both of them follow it: an `if` with no `else` still
    /// has the edge that skips the body, and a loop's exit is as much after the
    /// condition as its body is.
    ///
    /// `&&`, `||` and `?:` reach the same position through
    /// [`Lowering::split`] and [`Lowering::second`], which ask
    /// [`Lowering::top_level`] first because C leaves theirs unordered against
    /// whatever encloses them. A statement's controlling expression is a full
    /// expression whatever it is written inside, so there is nothing to ask.
    ///
    /// `None` is a `for` with no condition, which 6.8.5.3 p2 replaces by a
    /// constant: there is no expression, so there is no point at the end of
    /// one. It is spelled here rather than at that one caller so that what a
    /// branch's arm begins with is answered in one place.
    ///
    /// [`Lowering::top_level`]: Lowering::top_level
    fn enter(&mut self, block: BlockId, asked: Option<Span>) {
        self.switch(block);
        if let Some(asked) = asked {
            self.sequenced(asked);
        }
    }

    /// Whether control can still arrive at what comes next.
    fn reachable(&self) -> bool {
        self.block.is_some()
    }
}

/// What is still to be done to one expression.
///
/// The walk is a stack rather than a recursion because the tree is not bounded
/// by the parser's own nesting limit: a chain folded by a loop, which is how
/// every left-associative operator is read, adds a level per operator.
/// `driver/dumps.rs`'s `dump_expr` is the walker that paid for it first.
enum Task {
    /// Push the value of this expression.
    Value(ExprId),
    /// Take the value this call left, the temporary a struct was returned
    /// into, as the place the struct is (#1201).
    Placed(ExprId),
    /// The struct argument whose place is on the stack is copied, and the
    /// copy's address is the argument's value (#1201).
    Copied(ExprId),
    /// Push the place this expression names.
    Place(ExprId),
    /// The operands this node reads are on the stacks; produce its value.
    Finish(ExprId),
    /// The same, for a node being read as a place.
    FinishPlace(ExprId),
    /// The first operand of a `&&`, `||` or `?:` is done; branch on it.
    Split(ExprId),
    /// The left operand of a comma is done; say so before the right runs.
    ///
    /// Between the two rather than at the end, because what this builds has to
    /// land where the left operand ran. A comma finishes after its right
    /// operand, and a right operand can change what the left one said: `*p, p
    /// = q;` had the element placed after `p` was overwritten, which made a
    /// proved use of a freed value silent, and `*p, free(p);` had it placed
    /// after the free, which made defined C an error.
    Discard(ExprId),
    /// The `then` arm of a `?:` is done; start the `else`.
    Second(ExprId),
    /// The last arm is done; come back together.
    Merge(ExprId),
    /// Every argument of this call has been evaluated; say so before it runs.
    ///
    /// Between the operands and the node, the way [`Self::Discard`] is for a
    /// comma, and for the same reason: an element built where a node finishes
    /// lands after everything the node contains, and what this one says is
    /// about what came before it. The span is the call's, because that is what
    /// the point falls before.
    ///
    /// Pushed only where the flag reaches the call, which is why the flag is
    /// read rather than the task always being pushed: see
    /// [`Lowering::begin_value`] and [`Lowering::descend`].
    ///
    /// [`Lowering::begin_value`]: Lowering::begin_value
    Arguments(Span),
    /// Every operand of this node has been lowered; say which of them
    /// conflict over a local, before the node is built.
    ///
    /// Pushed by a call and by every operator that does not sequence: an
    /// assignment, a binary operator, a subscript. For a call, before
    /// [`Self::Arguments`] rather than where the call finishes, because
    /// `nullability::null_at_terminators` reads a block whose last element is
    /// [`Element::ArgumentsEvaluated`] as one whose arguments are ordered,
    /// and an element after it turned that answer off for the whole block.
    /// Pushed whatever encloses the node, which is the difference from that
    /// task. See ADR-0052.
    Conflicts(ExprId),
    /// Put [`Lowering::top_level`] back to what it was before this node.
    ///
    /// Pushed before a node's own tasks, so it is popped after all of them.
    /// A field rather than a value threaded through every task because the
    /// question is about where the walk is, and only three of the tasks ask
    /// it.
    Restore(bool),
}

/// Whether C sequences the operands of this node against each other.
///
/// The operators C17 Annex C lists: the comma operator (6.5.17 p2), `&&`
/// (6.5.13 p4), `||` (6.5.14 p4) and the conditional operator (6.5.15 p4).
/// Annex C's first entry is the sequence point between a call's arguments and
/// the call itself (6.5.2.2 p10's first sentence), and it is not this question:
/// a call's arguments are unsequenced against *each other*, which is why
/// `Expr::Call` answers false here.
///
/// **That one is an [`Element::ArgumentsEvaluated`], emitted by
/// [`Lowering::begin_value`]'s call arm.** ADR-0022 expressed it by position
/// instead: the argument operations are in the block before the call
/// terminator, which is enough for a consumer that looks backwards from the
/// call and nothing at all for ADR-0023's, which carries a read forwards. That
/// consumer reported `void f(int *p) { free(p + *p); }`, which C defines. See
/// ADR-0026, which also says why the element concludes less than an
/// [`Element::Sequenced`] does.
///
/// [`Lowering::begin_value`]: Lowering::begin_value
///
/// **Everything else answers false, including a node with one operand.** A
/// sequence point inside `-(free(p), *p)` does order those two, because there
/// is nothing else in the expression for them to be unordered against, and
/// answering false there costs a proof. What it buys is that this function is
/// Annex C's list and nothing beside it: a rule that says `false` too often
/// makes a warning out of an error, and one that says `true` once too often
/// hands an analysis a proof C does not license.
fn sequences(expr: &Expr) -> bool {
    match expr {
        Expr::Comma { .. } | Expr::Conditional { .. } => true,
        Expr::Binary { op, .. } => matches!(op, AstBinOp::LogAnd | AstBinOp::LogOr),
        Expr::Number { .. }
        | Expr::Enumerator { .. }
        | Expr::Character { .. }
        | Expr::Length { .. }
        | Expr::String { .. }
        | Expr::Identifier { .. }
        | Expr::Unary { .. }
        | Expr::Member { .. }
        | Expr::Assign { .. }
        | Expr::Call { .. }
        | Expr::Subscript { .. }
        | Expr::SizeofType { .. }
        | Expr::SizeofExpr { .. }
        | Expr::Cast { .. }
        | Expr::VaOp { .. }
        | Expr::VaArg { .. }
        | Expr::Error { .. } => false,
    }
}

/// One file-scope object as [`Lowering::file_objects`] gathers it, over
/// every declaration of it.
struct FileObject {
    name: String,
    ty: TyId,
    /// Whether its first declaration said `static`.
    internal: bool,
    /// Whether the program may not modify it, read off its first declaration.
    /// See [`Lowering::read_only`].
    read_only: bool,
    /// The levels at which a value stored in it points at `const`, read off
    /// its first declaration. See [`Lowering::const_levels`].
    const_levels: u32,
    /// Whether a declaration defines it, tentatively or not.
    defined: bool,
    init: StaticInit,
    /// The span of the name in each declaration of it.
    declared: Vec<Span>,
}

impl Lowering<'_> {
    /// Give every function a [`FuncId`] before any body is built.
    ///
    /// A call names its callee by id, and a callee can be defined after its
    /// caller or be the caller itself, so the ids come first and the bodies
    /// second. Until a body arrives the function is a declaration, which is
    /// what it truthfully is.
    fn declare(&mut self, diagnostics: &mut DiagnosticSink) {
        let mut objects: Vec<FileObject> = Vec::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        for item in self.ast.items() {
            match item {
                Item::Function(function) => {
                    let (name, ty, written) =
                        (function.name, function.ty, function.return_nullability);
                    if function.storage == Some(Storage::Static) {
                        self.internal.insert(self.sources.snippet(name).to_owned());
                    }
                    self.declare_one(name, ty, written, true, diagnostics);
                }
                Item::Declaration {
                    declarators,
                    specified,
                    storage,
                    ..
                } => {
                    self.file_objects(
                        declarators,
                        *storage,
                        (&mut objects, &mut by_name),
                        diagnostics,
                    );
                    // **A declaration that declares nothing still defines**:
                    // `struct S { int (*fp)(...); };` writes a prototype that
                    // no declarator's type carries, and a later `struct S s;`
                    // names the tag without its members. So its specifiers are
                    // walked where there is no declarator to walk them through
                    // (#448). Found by review.
                    // One set per declaration, because every declarator of
                    // `struct S { ... } s1, s2;` shares the struct's node, and
                    // walking it once per declarator reported one fault twice.
                    let mut walked = HashSet::new();
                    if declarators.is_empty() {
                        let _ = self.prototypes_lose_nothing(*specified, &mut walked, diagnostics);
                    }
                    for declarator in declarators {
                        // An object is [`Lowering::file_objects`]'s, above,
                        // and becomes a static; what is asked here is of a
                        // function, which `declare_one` passes over an object.
                        let declaration = &declarator.declaration;
                        if let Some(name) = declaration.name {
                            let (ty, written) = (declaration.ty, declaration.nullability);
                            self.declare_one(name, ty, written, false, diagnostics);
                        }
                        // **Asked here because nothing else asks it.** An
                        // object's type is lowered only where the object is
                        // used, and an unused one is never, so a prototype it
                        // holds went past the check a function's own prototype
                        // gets (#448). A function's own prototype is
                        // `signature`'s, so a function type is not walked.
                        if !matches!(self.ast.ty(declaration.ty), Type::Function { .. }) {
                            let _ = self.prototypes_lose_nothing(
                                declaration.ty,
                                &mut walked,
                                diagnostics,
                            );
                        }
                    }
                }
                Item::Error { .. } => {}
            }
        }

        for object in objects {
            // C17 6.9.2 p2: a tentative definition and no definition is a
            // definition with zero, which `StaticInit::Zero` already says;
            // only a unit that defines it nowhere leaves it to another.
            let linkage = match (object.defined, object.internal) {
                (false, _) => Linkage::Declared,
                (true, true) => Linkage::Internal,
                (true, false) => Linkage::External,
            };
            let id = self.unit.push_static(Static {
                name: object.name.clone(),
                ty: object.ty,
                linkage,
                init: object.init,
                read_only: object.read_only,
            });
            self.static_levels.insert(id, object.const_levels);
            self.file_statics.insert(object.name, id);
            for span in object.declared {
                self.statics.insert(span, id);
            }
        }
    }

    /// Every object a file-scope declaration declares, gathered into
    /// `objects` by spelling, since `int g; int g = 1;` is one object
    /// declared twice (C17 6.9.2 p2). `by_name` is each spelling's index
    /// there, a map rather than a scan per declarator, which the
    /// `definitions` map in `sema.rs` says a scan cost.
    ///
    /// The first declaration's linkage stands (6.2.2 p4): `static` gives
    /// internal linkage, and a later one without it agrees rather than
    /// changing it. A declaration with an initializer, or without `extern`,
    /// defines it, the second tentatively (6.9.2 p1, p2).
    ///
    /// **An object whose type this compiler cannot hold is passed over in
    /// silence**, its type asked into a sink nobody reads: a use of it is
    /// refused where it is written, as every global's was before this, and an
    /// unused one costs nothing. One whose initializer is not a number is
    /// refused here, since dropping it would build a program that starts
    /// with the wrong value.
    fn file_objects(
        &mut self,
        declarators: &[InitDeclarator],
        storage: Option<Storage>,
        (objects, by_name): (&mut Vec<FileObject>, &mut HashMap<String, usize>),
        diagnostics: &mut DiagnosticSink,
    ) {
        for declarator in declarators {
            let declaration = &declarator.declaration;
            let Some(name) = declaration.name else {
                continue;
            };
            if matches!(self.ast.ty(declaration.ty), Type::Function { .. }) {
                if storage == Some(Storage::Static) {
                    self.internal.insert(self.sources.snippet(name).to_owned());
                }
                continue;
            }
            let Some(ty) = self.ty(name, declaration.ty, &mut DiagnosticSink::new()) else {
                continue;
            };
            let init = match declarator.init {
                Some(init) => match self.static_init(init, ty, diagnostics) {
                    Some(init) => Some(init),
                    None => continue,
                },
                None => None,
            };
            let spelled = self.sources.snippet(name);
            let index = match by_name.get(spelled) {
                Some(&index) => index,
                None => {
                    by_name.insert(spelled.to_owned(), objects.len());
                    objects.push(FileObject {
                        name: spelled.to_owned(),
                        ty,
                        read_only: self.read_only(declaration.ty),
                        const_levels: self.const_levels(declaration.ty),
                        internal: storage == Some(Storage::Static),
                        defined: false,
                        init: StaticInit::Zero,
                        declared: Vec::new(),
                    });
                    objects.len() - 1
                }
            };
            let object = &mut objects[index];
            object.defined |= storage != Some(Storage::Extern) || init.is_some();
            if let Some(init) = init {
                object.init = init;
            }
            object.declared.push(name);
        }
    }

    /// What an object with static storage duration starts with: the value of
    /// its initializer, as its type holds it (C17 6.7.9 p10 by way of
    /// 6.5.16.1 p2), or `None` where it has none this can write.
    ///
    /// The type checker has refused one that is not a constant (6.7.9 p4).
    /// What is left without a value is an address constant, `int *p = &g;`,
    /// which C allows and a static here cannot hold yet; or one already
    /// reported, which is not reported twice.
    fn static_init(
        &mut self,
        init: ExprId,
        ty: TyId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<StaticInit> {
        // An array of character type from a string literal (C17 6.7.9 p14):
        // its bytes, cut or padded with zeros to the array's length, which
        // the type check has held to at least the literal's characters.
        if let (Expr::String { .. }, Ty::Array { length, .. }) =
            (self.ast.expr(init), self.unit.ty(ty))
        {
            let Length::Constant(length) = length else {
                unreachable!("the type check refuses a variable length array's initializer")
            };
            let mut bytes = self.string_bytes(init);
            bytes.resize(usize::try_from(length).expect("an array's length fits"), 0);
            return Some(StaticInit::Bytes(bytes));
        }
        if let Some(value) = self.types.value(init) {
            let value = self
                .unit
                .integer(ty)
                .map_or(value, |int| int.convert(value));
            return Some(StaticInit::Int(value));
        }
        // A null pointer however it is cast, `(int *)0` or `(void *)0`:
        // C17 6.3.2.3 p3 and p4 make each a null pointer. Mutation: answer
        // none; `a_null_pointer_cast_initializes_a_static` is refused.
        let mut id = init;
        while let Expr::Cast { operand, .. } = self.ast.expr(id) {
            id = *operand;
        }
        if id != init && self.types.value(id) == Some(0) {
            return Some(StaticInit::Int(0));
        }
        if !diagnostics.has_errors() {
            diagnostics.report(
                Diagnostic::error("cannot compile this initializer yet")
                    .with_code(LOWERING)
                    .with_label(Label::primary(
                        self.ast.expr(init).span(),
                        "an address, which an object with static storage duration cannot start with so far",
                    ))
                    .with_note("an initializer that is a number or a null pointer is all this compiler holds"),
            );
        }
        None
    }

    /// The object with static storage duration the identifier `id` names, if
    /// it names one.
    fn static_named(&self, id: ExprId) -> Option<StaticId> {
        self.resolution
            .resolved(id)
            .map(|binding| self.resolution.binding(binding).name)
            .and_then(|name| self.statics.get(&name).copied())
    }

    /// The static a string literal is, made the first time it is asked for:
    /// an array of `char` holding each literal's characters, escapes read,
    /// then a terminating zero (C17 6.4.5 p6), adjacent literals joined
    /// (5.1.1.2 phase 6). Internal and named with a `.`, which no C
    /// identifier holds, so it is no object the program can name.
    ///
    /// Mutation: leave out the zero; `a_string_literal_is_its_bytes_and_a_zero`
    /// answers another length. Mutation: read only the first literal of a
    /// run; the same test answers 1 where `"a" "b"` is 2.
    fn string(&mut self, id: ExprId) -> StaticId {
        if let Some(&object) = self.strings.get(&id) {
            return object;
        }
        let bytes = self.string_bytes(id);
        let element = self.unit.push_type(Ty::CHAR);
        let ty = self.unit.push_type(Ty::Array {
            element,
            length: Length::Constant(bytes.len() as u64),
        });
        let object = self.unit.push_static(Static {
            name: format!(".str.{}", self.strings.len()),
            ty,
            linkage: Linkage::Internal,
            init: StaticInit::Bytes(bytes),
            // C17 6.4.5 p7: a literal may not be modified, whatever its type
            // says. See ADR-0120.
            read_only: true,
        });
        self.strings.insert(id, object);
        object
    }

    /// The bytes of the string literal `id`: each literal's characters,
    /// escapes read, adjacent literals joined, then a terminating zero.
    fn string_bytes(&self, id: ExprId) -> Vec<u8> {
        let Expr::String { parts, .. } = self.ast.expr(id) else {
            unreachable!("a string literal is asked for only of one")
        };
        let mut bytes = Vec::new();
        for &part in parts {
            let text = self.sources.snippet(part);
            let body = &text[1..text.len() - 1];
            let values = crate::literal::unescape(body)
                .expect("the parser keeps only a literal every escape of which it reads");
            bytes.extend(values.into_iter().map(|value| {
                u8::try_from(value).expect("`unescape` refuses a value no `char` holds")
            }));
        }
        bytes.push(0);
        bytes
    }

    /// Initialize the local array `local` of character type from the string
    /// literal `init` (C17 6.7.9 p14): a `memset` to zero where the array is
    /// longer than the literal, then a store per byte the array holds, each
    /// built as `s[i] = c` is. A `char` array holds no pointer, so the
    /// `memset` is read as the write it is. See ADR-0137.
    ///
    /// Mutation: leave out the `memset`; the trailing bytes of
    /// `a_char_array_longer_than_its_string_literal_is_zero_filled` are what
    /// the stack held. Mutation: store one byte fewer;
    /// `a_char_array_takes_its_length_and_bytes_from_a_string_literal`
    /// reads another value.
    fn store_string(
        &mut self,
        builder: &mut Builder,
        local: LocalId,
        init: ExprId,
        at: Span,
    ) -> Option<()> {
        let ty = builder.function.local(local);
        let Ty::Array {
            element,
            length: Length::Constant(length),
        } = self.unit.ty(ty)
        else {
            unreachable!("the type check initializes only a sized array from a string literal")
        };
        let length = usize::try_from(length).expect("an array's length fits");
        let mut bytes = self.string_bytes(init);
        let pointer = self.unit.push_type(Ty::Pointer(element));
        let first = builder.function.push_local(pointer);
        builder.push(Operation {
            place: Place::local(first),
            value: Rvalue::Address(Place::new(
                local,
                vec![Projection::Index(Operand::Constant(0))],
            )),
            origin: Origin::Generated(at),
        });
        if length > bytes.len() {
            let memset = self.memset(at);
            let then = builder.function.reserve_block();
            builder.end(Terminator::Call {
                callee: Called::Direct(memset),
                arguments: vec![
                    Operand::Copy(Place::local(first)),
                    Operand::Constant(0),
                    Operand::Constant(i128::try_from(length).expect("an array's length fits")),
                ],
                destination: None,
                then: Some(then),
                origin: Origin::Written(at),
            });
            builder.switch(then);
            // Every byte past the literal's is the `memset`'s.
        }
        bytes.truncate(length);
        let integer = self.unit.integer(element);
        for (index, byte) in bytes.into_iter().enumerate() {
            let value = i128::from(byte);
            let value = integer.map_or(value, |int| int.convert(value));
            let into = if index == 0 {
                first
            } else {
                let into = builder.function.push_local(pointer);
                builder.push(Operation {
                    place: Place::local(into),
                    value: Rvalue::Binary {
                        op: BinOp::Add,
                        lhs: Operand::Copy(Place::local(first)),
                        rhs: Operand::Constant(i128::try_from(index).expect("an index fits")),
                    },
                    origin: Origin::Generated(at),
                });
                into
            };
            builder.push(Operation {
                place: Place::local(into).deref(),
                value: Rvalue::Use(Operand::Constant(value)),
                origin: Origin::Written(at),
            });
        }
        Some(())
    }

    /// The `memset` [`Lowering::store_string`] calls: the program's own
    /// where it has one, as [`Lowering::memcpy`] is, and otherwise one this
    /// compiler makes.
    fn memset(&mut self, at: Span) -> FuncId {
        if let Some(id) = self.memset {
            return id;
        }
        let id = match self.functions.get("memset") {
            Some(&id) => id,
            None => {
                let void = self.unit.push_type(Ty::Void);
                let pointer = self.unit.push_type(Ty::Pointer(void));
                let int = self.unit.push_type(Ty::INT);
                let (rank, sign) = self.unit.target().size_type();
                let size = self.unit.push_type(Ty::Integer { rank, sign });
                self.unit.push_function(Function::builtin(
                    "memset",
                    at,
                    pointer,
                    [pointer, int, size],
                ))
            }
        };
        self.memset = Some(id);
        id
    }

    /// Whether an object of `ty` may not be modified: `const` at its
    /// innermost element (C17 6.7.3 p6), so `const int a[2]` is and `int
    /// *const p`'s pointee is no business of `p`'s. See ADR-0120.
    fn read_only(&self, ty: TypeId) -> bool {
        let mut ty = ty;
        while let Type::Array { element, .. } = self.ast.ty(ty) {
            ty = *element;
        }
        self.ast.is_const(ty)
    }

    /// The place a use of a static is: `*t`, where `t` is a temporary holding
    /// its address. See ADR-0115.
    fn reach_static(&mut self, builder: &mut Builder, object: StaticId, span: Span) -> Place {
        let ty = self.unit.static_object(object).ty;
        let pointer = self.unit.push_type(Ty::Pointer(ty));
        let address = builder.function.push_local(pointer);
        // **As a pointer to it would be declared**: `const` at its first
        // level where the static is, and below where what it holds is, so a
        // store of a literal into `const char *G` is no write the static's
        // readers may make. Found by the review of #1134. See ADR-0121.
        // Mutation: mark nothing; `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
        // `into_a_const_static` is reported.
        // A pointer to an array static is read as the pointer to its
        // element is, which the static's own levels already are; to any
        // other, one level above what the static holds.
        let held = self.static_levels.get(&object).copied().unwrap_or(0);
        let levels = match self.unit.ty(ty) {
            Ty::Array { .. } => held,
            _ => held << 1 | u32::from(self.unit.static_object(object).read_only),
        };
        builder.function.mark_const_levels(address, levels);
        builder.push(Operation {
            place: Place::local(address),
            value: Rvalue::Static(object),
            origin: Origin::Generated(span),
        });
        Place::new(address, vec![Projection::Deref])
    }

    /// A block-scope declaration that names a storage class: a `static`
    /// object, which is a static of its own whose name no other function's
    /// shares, or an `extern` one, which is the file-scope object of that
    /// name (C17 6.2.2 p4), or one declared for another unit to define.
    ///
    /// Nothing runs where it is written. Its initializer is its starting
    /// value, given before the program begins (5.1.2 p1), so the declaration
    /// is no assignment, and no storage begins or ends with the block.
    fn block_static(
        &mut self,
        builder: &Builder,
        declarator: &InitDeclarator,
        storage: Storage,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let declaration = &declarator.declaration;
        let Some(name) = declaration.name else {
            return Some(());
        };
        let ty = self.ty(declaration.span, declaration.ty, diagnostics)?;
        let spelled = self.sources.snippet(name).to_owned();
        let id = match storage {
            Storage::Static => {
                let init = match declarator.init {
                    Some(init) => self.static_init(init, ty, diagnostics)?,
                    None => StaticInit::Zero,
                };
                // `.` is in no C identifier, so no file-scope name and no
                // other function's static is spelled the same.
                let function = self.sources.snippet(builder.function.name);
                let base = format!("{function}.{spelled}");
                let count = self.block_statics.entry(base.clone()).or_insert(0);
                let unique = match *count {
                    0 => base,
                    n => format!("{base}.{n}"),
                };
                *count += 1;
                let id = self.unit.push_static(Static {
                    name: unique,
                    ty,
                    linkage: Linkage::Internal,
                    init,
                    read_only: self.read_only(declaration.ty),
                });
                self.static_levels
                    .insert(id, self.const_levels(declaration.ty));
                id
            }
            Storage::Extern => {
                let id = match self.file_statics.get(&spelled) {
                    Some(&id) => id,
                    None => {
                        let id = self.unit.push_static(Static {
                            name: spelled.clone(),
                            ty,
                            linkage: Linkage::Declared,
                            init: StaticInit::Zero,
                            read_only: self.read_only(declaration.ty),
                        });
                        self.static_levels
                            .insert(id, self.const_levels(declaration.ty));
                        self.file_statics.insert(spelled.clone(), id);
                        id
                    }
                };
                // C17 6.2.7 p2 makes two declarations of one object with
                // incompatible types undefined, and the type checker compares
                // only declarations in one scope. A read through the wrong
                // type would be built in silence, so it is refused.
                if self.unit.static_object(id).ty != ty {
                    diagnostics.report(
                        Diagnostic::error("cannot compile this declaration yet")
                            .with_code(LOWERING)
                            .with_label(Label::primary(
                                name,
                                format!("`{spelled}` is declared with another type outside this block"),
                            ))
                            .with_note("C17 6.2.7 p2 makes two declarations of one object with incompatible types undefined"),
                    );
                    return None;
                }
                id
            }
        };
        self.statics.insert(name, id);
        Some(())
    }

    /// One name out of [`Lowering::declare`]'s walk.
    ///
    /// Its own function because one declaration may declare several names and
    /// each of them is its own function or its own object, so this runs once
    /// per declarator rather than once per item.
    ///
    /// `definition` says whether a body was written. Only an [`Item::Function`]
    /// carries one, and it decides the diagnostic below.
    ///
    /// `written` is the nullability specifier on the pointer it returns, which
    /// the parser allows only on a function declared at file scope.
    fn declare_one(
        &mut self,
        name: Span,
        ty: TypeId,
        written: Option<Nullability>,
        definition: bool,
        diagnostics: &mut DiagnosticSink,
    ) {
        let Type::Function {
            returns,
            parameters,
            ..
        } = self.ast.ty(ty)
        else {
            // A declaration of an object is the ordinary case here and is
            // answered where it is used. A *definition* of one is not: C17
            // 6.9.1 p2 requires the identifier in a function definition to
            // have a function type, `int (*f)(int) { ... }` does not, and
            // nothing before this stage checks it. Dropping it in silence
            // would leave a translation unit missing a function that the
            // file plainly contains, and the artifact saying `declared`
            // about a body it can see.
            if definition {
                diagnostics.report(
                        Diagnostic::error("this defines something that is not a function")
                            .with_code(LOWERING)
                            .with_label(Label::primary(
                                name,
                                format!("this declares `{}`", spell_type(self.sources, self.ast, ty)),
                            ))
                            .with_note(
                                "C17 6.9.1 p2 requires the identifier in a function definition to have a function type",
                            ),
                    );
                self.refused.insert(self.sources.snippet(name).to_owned());
            }
            return;
        };
        let (returns, parameters) = (*returns, parameters.clone());
        let Some((returns, lowered)) = self.signature(name, returns, &parameters, diagnostics)
        else {
            // The signature was reported and there is no honest function to
            // put here: inventing one would tell a caller a return type
            // this compiler could not read. What is remembered instead is
            // the name, so that a call to it says nothing more. The user
            // has been told once, about the declaration, and a second
            // diagnostic pointing at an ordinary call would be blaming code
            // that is fine.
            self.refused.insert(self.sources.snippet(name).to_owned());
            return;
        };

        // Before the function is recorded, because `lowered` moves into it.
        // `()` says nothing about the parameters, so it has nothing to agree
        // or disagree with, which is why only a prototype is asked.
        if let Parameters::Prototype(parameters) = &parameters {
            let specifiers = parameters.iter().map(|parameter| parameter.nullability);
            self.agree(name, specifiers.collect(), diagnostics);
        }
        self.agree_on_return(name, written, diagnostics);

        // A name declared twice is one function. The first declaration is
        // the one whose span the IR carries, which is where a reader of a
        // diagnostic about the callee is pointed. What it returns and takes
        // is the standing declaration's, the one the type checker checks a
        // call against (see `Resolution::standing`): built from the first,
        // `int f(); int f(int *_Nonnull a);` gave `f` no parameter, and
        // `f(0)` passed a null pointer to a `_Nonnull` one in silence.
        if !self.functions.contains_key(self.sources.snippet(name)) {
            let Some((returns, lowered)) = self.standing_signature(name, (returns, lowered)) else {
                // The standing declaration's signature cannot be read. It is
                // a declaration of this file too, so it is reported, and its
                // name refused, where it is walked; nothing is done here.
                return;
            };
            let mut declared = Function::declaration_with_parameters(name, returns, lowered);
            if !self.standing_prototyped(name, &parameters) {
                declared = declared.unprototyped();
            }
            if self.standing_variadic(name) {
                declared = declared.variadic();
            }
            // **And at which levels each parameter points at `const`**, as a
            // definition's are, so a call to a body this unit does not have
            // asks what it hands below the first level by the same type
            // (#1134). See ADR-0121.
            if let Parameters::Prototype(written) = &parameters {
                let locals: Vec<_> = declared.parameters().collect();
                for (parameter, local) in written.iter().zip(locals) {
                    declared.mark_const_levels(local, self.const_levels(parameter.ty));
                }
            }
            // A written promise is believed of a function this unit does not
            // define, since nobody here can ask its body: the boundary
            // ADR-0050 accepts, as ADR-0037 accepts it of a parameter.
            if let Some(promise) = written_promise(written) {
                declared = declared.promising(promise);
            }
            let id = self.unit.push_function(declared);
            self.functions
                .insert(self.sources.snippet(name).to_owned(), id);
        }
    }

    /// Whether `name`'s standing declaration declares its parameters, or
    /// `own`, the list in hand, where there is no standing function.
    ///
    /// **The standing one, as the signature is**: `int f(); int f(int);`
    /// is one function with a prototype, and a pointer to it is converted
    /// as one. Mutation: answer `true`; `unprototyped_designator` in
    /// `a_function_pointer_s_type_is_asked_where_it_is_converted_or_called`
    /// is reported as converted.
    fn standing_prototyped(&self, name: Span, own: &Parameters) -> bool {
        let standing = self.resolution.declared(self.sources.snippet(name));
        match standing.map(|ty| self.ast.ty(ty)) {
            Some(Type::Function { parameters, .. }) => {
                matches!(parameters, Parameters::Prototype(_))
            }
            _ => matches!(own, Parameters::Prototype(_)),
        }
    }

    /// Whether `name`'s standing declaration ends its parameters in `, ...`.
    /// Two declarations that disagree on it are a conflict the frontend
    /// refuses (C17 6.7.6.3 p15), so the standing one answers for both.
    ///
    /// Mutation: answer `false`; `printf` is declared without its `...` and
    /// `a_variadic_function_is_called_with_its_arguments_promoted` in
    /// `tests/run.rs` is refused by the backend.
    fn standing_variadic(&self, name: Span) -> bool {
        let standing = self.resolution.declared(self.sources.snippet(name));
        matches!(
            standing.map(|ty| self.ast.ty(ty)),
            Some(Type::Function { variadic: true, .. })
        )
    }

    /// The signature of `name`'s standing declaration, or `own`, the one in
    /// hand, where the standing declaration is not a function or is this one.
    ///
    /// Read into a sink of its own: the standing declaration is a declaration
    /// in this file, and `declare` reports what is wrong with it when it walks
    /// it, so a second report from here would say the same thing twice.
    fn standing_signature(
        &mut self,
        name: Span,
        own: (TyId, Vec<Parameter>),
    ) -> Option<(TyId, Vec<Parameter>)> {
        let standing = self.resolution.declared(self.sources.snippet(name));
        let Some(Type::Function {
            returns,
            parameters,
            ..
        }) = standing.map(|ty| self.ast.ty(ty))
        else {
            return Some(own);
        };
        let (returns, parameters) = (*returns, parameters.clone());
        self.signature(name, returns, &parameters, &mut DiagnosticSink::new())
    }

    /// Refuse a prototype of a function that disagrees with the first prototype
    /// of it about a parameter's nullability specifier, or record this one as
    /// the first. See [`DISAGREEING_ANNOTATION`].
    ///
    /// Position by position, over as many parameters as both have. A
    /// prototype with a different count is a different disagreement: two
    /// prototypes of different counts are incompatible, which the frontend
    /// reports as conflicting types (`SC0313`), and a program it refuses is
    /// not lowered. A prototype against `()` can differ in count and agree,
    /// and is not compared here, as the `()` has nothing to compare.
    ///
    /// **Three answers, compared as written**: `_Nonnull`, `_Nullable`, and
    /// none. The last two lower alike, which is why the specifiers are compared
    /// here rather than what the IR carries, and they are still a disagreement:
    /// at level 5 they would not be, where a parameter of a function with
    /// internal linkage is non-null unless written `_Nullable` (ADR-0050), and
    /// where a specifier is read does not depend on the level.
    fn agree(
        &mut self,
        name: Span,
        later: Vec<Option<Nullability>>,
        diagnostics: &mut DiagnosticSink,
    ) {
        let key = self.sources.snippet(name);
        let Some((first_name, first)) = self.prototypes.get(key) else {
            self.prototypes.insert(key.to_owned(), (name, later));
            return;
        };
        let specifier = |written: &Option<Nullability>| written.map(|written| written.specifier);
        let differs = first
            .iter()
            .zip(&later)
            .find(|(first, later)| specifier(first) != specifier(later));
        let Some((first, later)) = differs else {
            return;
        };
        let report = disagreement(self.sources, name, *first_name, "prototype", *first, *later);
        diagnostics.report(report);
    }

    /// Refuse a declaration of a function that disagrees with the first one
    /// about the nullability specifier on the pointer it returns, or record
    /// this one as the first. See [`DISAGREEING_ANNOTATION`].
    ///
    /// The same three answers [`Lowering::agree`] compares, for the same
    /// reason, and against the first declaration rather than the first
    /// prototype, for the reason [`Lowering::return_nullability`] gives.
    fn agree_on_return(
        &mut self,
        name: Span,
        later: Option<Nullability>,
        diagnostics: &mut DiagnosticSink,
    ) {
        let key = self.sources.snippet(name);
        let Some((first_name, first)) = self.return_nullability.get(key) else {
            self.return_nullability
                .insert(key.to_owned(), (name, later));
            return;
        };
        let specifier = |written: Option<Nullability>| written.map(|written| written.specifier);
        if specifier(*first) == specifier(later) {
            return;
        }
        let report = disagreement(
            self.sources,
            name,
            *first_name,
            "declaration",
            *first,
            later,
        );
        diagnostics.report(report);
    }

    /// The declaration a call is checked against, and the locals a body starts
    /// with.
    ///
    /// Local 0 is the return place and the parameters follow it, which is what
    /// [`Function::with_parameters`] lays out. Each carries the `_Nonnull` its
    /// declaration wrote, which is what a call is checked against.
    ///
    /// **`_Nullable` makes no promise**, which is what an unannotated parameter
    /// is below level 5, and at level 5 too for a function with external
    /// linkage, whose callers in other translation units nobody checks
    /// (ADR-0050). Every function here has external linkage, because the
    /// parser reads no storage class, so no parameter is non-null unless it
    /// was written `_Nonnull`.
    ///
    /// An empty parameter list is lowered as no parameters: C17 6.7.6.3 p14
    /// makes `()` say nothing about the count rather than say there are none,
    /// and the IR has no way to spell "not said". Nothing here reads it,
    /// because a call carries the arguments it passes, and `types.rs` is where
    /// the count is checked.
    fn signature(
        &mut self,
        name: Span,
        returns: TypeId,
        parameters: &Parameters,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<(TyId, Vec<Parameter>)> {
        // **No list crosses a call** (#1214): C adjusts a parameter of an
        // array type to a pointer (6.7.6.3 p7), so where `__builtin_va_list`
        // is an array `clang` passes the caller's list by address and the
        // callee reads it on, while the IR's array would be copied. Passed
        // so, a `vprintf`-style function read a copy and its caller's list
        // never moved. Refused rather than laid out per target, as passing
        // or returning one is refused at the call. Mutation: lower it as
        // any type; `a_list_is_no_parameter_and_no_result_yet` builds.
        let crosses_a_list = matches!(self.ast.ty(returns), Type::VaList)
            || matches!(parameters, Parameters::Prototype(parameters)
                if parameters.iter().any(|parameter| matches!(self.ast.ty(parameter.ty), Type::VaList)));
        if crosses_a_list {
            diagnostics.report(
                Diagnostic::error("cannot compile a `__builtin_va_list` passed to or returned from a function yet")
                    .with_code(LOWERING)
                    .with_label(Label::primary(name, "declared here"))
                    .with_note("pass a pointer to the list instead"),
            );
            return None;
        }
        let mut returns = self.ty(name, returns, diagnostics)?;
        let mut lowered = Vec::new();
        // **A struct returned by value goes where the caller says**, through
        // a pointer passed first, and the function returns nothing (#1201).
        // See `Passes`.
        if matches!(self.unit.ty(returns), Ty::Struct(_)) {
            // **Never null**: every caller passes where it wants the result,
            // so the pointer is promised, which no source wrote and no caller
            // can break. Unpromised, every member written through it was
            // doubted as through a pointer that may be null.
            lowered.push(Parameter {
                ty: self.unit.push_type(Ty::Pointer(returns)),
                nonnull: Some(Promise::Made(name)),
                name: None,
                reads_only: false,
                passes: Passes::StructReturn,
            });
            returns = self.unit.push_type(Ty::Void);
        }
        if let Parameters::Prototype(parameters) = parameters {
            for parameter in parameters {
                let at = parameter.name.unwrap_or(parameter.span);
                // C17 6.7.6.3 p12 lets a declaration that is not a definition
                // have a parameter of incomplete type, `void` included, and
                // `types.rs` accepts one for that reason; a definition with one
                // never gets here. But no IR type holds nothing, and left to
                // the backend this was `SC0801`, which says a program is
                // beyond the backend when what it is beyond is this lowering.
                if matches!(self.ast.ty(parameter.ty), Type::Void) {
                    diagnostics.report(
                        Diagnostic::error("cannot compile a parameter of type `void` yet")
                            .with_code(LOWERING)
                            .with_label(Label::primary(at, "declared here"))
                            .with_note(
                                "C17 6.7.6.3 p12 allows one in a declaration that is not a definition, and nothing can be passed to it",
                            ),
                    );
                    return None;
                }
                if !self.adjusted_without_loss(parameter.written) {
                    self.refuse_parameter(parameter, diagnostics);
                    return None;
                }
                let ty = self.ty(at, parameter.ty, diagnostics)?;
                // **A struct passed by value is the address of the caller's
                // copy** (#1201). See `Passes`.
                if matches!(self.unit.ty(ty), Ty::Struct(_)) {
                    // Never null either, for the same reason: every caller
                    // passes its copy's address. Mutation: promise nothing;
                    // `a_struct_is_assigned_passed_and_returned_by_value`'s
                    // `read_a_member` reports `SC0403`.
                    lowered.push(Parameter {
                        ty: self.unit.push_type(Ty::Pointer(ty)),
                        nonnull: Some(Promise::Made(at)),
                        name: parameter.name,
                        reads_only: false,
                        passes: Passes::StructCopy,
                    });
                    continue;
                }
                lowered.push(Parameter {
                    reads_only: matches!(self.ast.ty(parameter.ty),
                        Type::Pointer(pointee) if self.read_only(*pointee)),
                    ty,
                    nonnull: match parameter.nullability {
                        Some(Nullability {
                            specifier: Specifier::Nonnull,
                            at,
                        }) => Some(Promise::Declared(at)),
                        Some(Nullability {
                            specifier: Specifier::Nullable,
                            at: _,
                        })
                        | None => None,
                    },
                    name: parameter.name,
                    passes: Passes::Value,
                });
            }
        }

        Some((returns, lowered))
    }

    /// Whether the expression `id` has a struct type.
    fn is_struct(&self, id: ExprId) -> bool {
        self.types
            .of(id)
            .is_some_and(|ty| matches!(self.ast.ty(ty), Type::Struct { .. }))
    }

    /// The address of the struct at `place`, as an operand: the pointer it is
    /// reached through, `&*p` being `p` (C17 6.5.3.2 p3), or a temporary
    /// holding `&s` for a struct with no pointer in front of it.
    fn struct_address_of(&mut self, builder: &mut Builder, place: Place, at: Span) -> Operand {
        if let Some(pointer) = place.without_last_deref() {
            return Operand::Copy(pointer);
        }
        let ty = self
            .unit
            .place_ty(&builder.function, &place)
            .expect("a place this lowering built has a type");
        let pointer = self.unit.push_type(Ty::Pointer(ty));
        let address = builder.function.push_local(pointer);
        builder.push(Operation {
            place: Place::local(address),
            value: Rvalue::Address(place),
            origin: Origin::Generated(at),
        });
        Operand::Copy(Place::local(address))
    }

    /// Copy the struct at `from` into `into`: a call to `memcpy` of the
    /// struct's size, made by this compiler whatever the source declares
    /// (#1201). **A call the memory check reads as the library copy it is**,
    /// slot by slot and carrying what the source holds, so a struct copy is
    /// answered by the door that already answers `memcpy`, and the backend
    /// writes the copy LLVM writes for one.
    fn copy_struct(
        &mut self,
        builder: &mut Builder,
        into: Place,
        from: Place,
        at: Span,
    ) -> Option<()> {
        let ty = self
            .unit
            .place_ty(&builder.function, &into)
            .expect("a place this lowering built has a type");
        let size = self.unit.size_of(ty)?;
        let memcpy = self.memcpy(at);
        let into = self.struct_address_of(builder, into, at);
        let from = self.struct_address_of(builder, from, at);
        let then = builder.function.reserve_block();
        builder.end(Terminator::Call {
            callee: Called::Direct(memcpy),
            arguments: vec![into, from, Operand::Constant(i128::from(size))],
            destination: None,
            then: Some(then),
            origin: Origin::Written(at),
        });
        builder.switch(then);
        Some(())
    }

    /// The `memcpy` a struct copy calls: the program's own, declared or
    /// defined, where it has one, and otherwise one this compiler makes, so
    /// that a module never has it twice.
    ///
    /// **The program's even where it defines it**, wherever in the file:
    /// what a program defining a library name means is C17 7.1.3 p2's to
    /// say, and it is undefined, which the safety check reports as `SC0319`;
    /// a freestanding program defines `memcpy` as a matter of course, and a
    /// struct copy calling it is what `clang` builds. Asking whether it is
    /// defined here asked before a definition later in the file was lowered,
    /// so the answer depended on the order. Mutation: refuse a defined one;
    /// `a_struct_copy_calls_the_programs_own_memcpy` is refused.
    fn memcpy(&mut self, at: Span) -> FuncId {
        if let Some(id) = self.memcpy {
            return id;
        }
        let id = match self.functions.get("memcpy") {
            Some(&id) => id,
            None => {
                let void = self.unit.push_type(Ty::Void);
                let pointer = self.unit.push_type(Ty::Pointer(void));
                let (rank, sign) = self.unit.target().size_type();
                let size = self.unit.push_type(Ty::Integer { rank, sign });
                self.unit.push_function(Function::builtin(
                    "memcpy",
                    at,
                    pointer,
                    [pointer, pointer, size],
                ))
            }
        };
        self.memcpy = Some(id);
        id
    }

    /// Refuse a struct used whole other than as an object copied, passed or
    /// returned: the value of an assignment, a `?:` or a comma, or a struct
    /// read whole where a scalar would be. Each of those is lowered by
    /// copying from the place a struct lives (#1201), and these have none.
    /// A whole read built here would be a value the memory check and the
    /// backend have no meaning for: the backend refused it as `SC0801`, and
    /// the check read it as nothing anybody decided. Refused here, under the
    /// code for what this compiler does not do yet.
    ///
    /// Said of a union as a union (#1203), the expression `id` saying which.
    fn struct_value_not_yet(&self, id: ExprId, at: Span, diagnostics: &mut DiagnosticSink) {
        let keyword = match self.types.of(id).map(|ty| self.ast.ty(ty)) {
            Some(Type::Struct { union: true, .. }) => "union",
            _ => "struct",
        };
        diagnostics.report(
            Diagnostic::error(format!("cannot compile a {keyword} used as a value yet"))
                .with_code(LOWERING)
                .with_label(Label::primary(at, format!("a {keyword}, whole")))
                .with_note("a struct is copied, passed and returned where it is an object; the value of an assignment, a `?:` or a comma that is a struct is not lowered yet"),
        );
    }

    /// The span from `from`'s start to `to`'s end, through the one join
    /// across instances, or nothing, with the function refused.
    ///
    /// A span the lowering builds over parts of nodes the parser built, a
    /// definition's lengths or a declarator's store, is one the parser never
    /// joined, so it can cut an included file where every node the parser
    /// made held it whole: `int` then an `#include` of `*a` then `= 0;` starts
    /// the store of `a` at `a`, partway through the header. Refused rather
    /// than panicked on, since the program is C. See ADR-0054.
    fn joined(&self, from: Span, to: Span, diagnostics: &mut DiagnosticSink) -> Option<Span> {
        match self.sources.join(from, to) {
            Ok(span) => Some(span),
            Err(cut) => {
                // No "included here": the parser already accepted the
                // construct, and the reader is told where the cut is.
                let (what, label, _) = cut_words(self.sources, cut);
                diagnostics.report(
                    Diagnostic::error(format!("cannot compile {what} where it is yet"))
                        .with_code(LOWERING)
                        .with_label(Label::primary(cut.span, label))
                        .with_note("safec reads an included file or a macro's expansion only where everything it builds over it holds all of it or none of it"),
                );
                None
            }
        }
    }

    /// [`Self::joined`] where a cut is not an error, because the caller has a
    /// span to fall back to: `None` where it would cut, reporting nothing.
    fn try_joined(&self, from: Span, to: Span) -> Option<Span> {
        self.sources.join(from, to).ok()
    }

    /// A declarator's store span: from its `name` through the end of
    /// `declared`, its [`Declaration::span`], where that joins, and the
    /// init-declarator's own span, `own`, where it would cut a header the
    /// init-declarator holds whole.
    ///
    /// `int` then `#include` of `*a` then `= 0;` starts at the name `a`,
    /// after the header's `*`, and `*a = 0` holds the header from its first
    /// token. Only there, so that every other store keeps the position it
    /// had (#555). `own` holds this declarator's tokens and no other's. A
    /// sibling's read could not be taken for this store's operand even
    /// through a wider span, since each initializer is a full expression
    /// (C17 6.8 p4), sequenced before the next declarator, and that clears
    /// what is pending, so the narrow span is about where the store is said
    /// to be. Where both cut, refused as before.
    ///
    /// Mutation: fall back to the span from the declaration's specifiers.
    /// `a_sibling_declarators_read_beside_a_store_from_a_header_is_reported`
    /// fails on the store's printed position; its read of `q` is still
    /// reported, measured.
    ///
    /// [`Declaration::span`]: crate::ast::Declaration::span
    fn store_span(
        &self,
        name: Span,
        declared: Span,
        own: Option<Span>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Span> {
        match (self.try_joined(name, declared), own) {
            (Some(span), _) | (None, Some(span)) => Some(span),
            (None, None) => self.joined(name, declared, diagnostics),
        }
    }

    /// Refuse a parameter declared as an array this stage cannot take,
    /// naming the part of its length nothing typed where that is the reason.
    ///
    /// One refusal for both doors, each asking its own question first: a
    /// function's own prototype, through [`Self::signature`], which lowers
    /// the parameter and so asks [`Self::adjusted_without_loss`]; and a
    /// prototype inside the type of a file-scope object, through
    /// [`Self::prototypes_lose_nothing`], which lowers nothing and so asks
    /// only [`Self::lengths_typed_throughout`] (#448).
    fn refuse_parameter(&self, parameter: &Declaration, diagnostics: &mut DiagnosticSink) {
        let at = parameter.name.unwrap_or(parameter.span);
        // The note claims nothing about the program, because a length nobody
        // typed can hide a fault: `int a[*(p - q)]` is not C, and the type
        // check could not ask while `p - q` had no type, which it has since
        // #638; no untyped length is measured now (#1298).
        let mut refusal =
            Diagnostic::error("cannot compile a parameter declared as this array yet")
                .with_code(LOWERING)
                .with_label(Label::primary(at, "declared here"));
        // Which part of the length has no type, where that is the reason, so
        // the reader is not left to find it.
        if let Type::Array {
            length: Some(length),
            ..
        } = self.ast.ty(parameter.written)
        {
            if self.types.value(*length).is_none() {
                if let Some(untyped) = self.first_untyped(*length) {
                    refusal = refusal.with_label(Label::secondary(
                        self.ast.expr(untyped).span(),
                        "nothing worked out what type this has",
                    ));
                }
            }
        }
        diagnostics.report(refusal.with_note(
            "this is a gap in this compiler: whether this parameter is valid C is not checked",
        ));
    }

    /// Whether every length in a parameter's written array type that is not
    /// a constant was typed all the way down.
    ///
    /// The one question a prototype inside a file-scope object's type is
    /// asked. Not [`Self::adjusted_without_loss`]'s whole answer, whose rule
    /// against an inner variable length is about what the IR can hold, and
    /// nothing lowers an object's type: `int (*fp)(int n, int a[][n]);` is C
    /// (C17 6.7.6.2 p2 and p5) and builds. What is left is #446's reason, a
    /// part nobody typed can hide a fault nothing will check, which holds at
    /// any depth since 6.7.6.2 p5 runs none of them. Found by review.
    ///
    /// **Through pointers as well as arrays**, since `int (*a)[*(p - q)]` is
    /// the same length behind a pointer and 6.7.6.2 p5 runs it no more than
    /// the other: asking only the array chain at the top left it to build.
    /// Found by review. It stops at a function type, which
    /// [`Self::prototypes_lose_nothing`] walks as a prototype of its own.
    fn lengths_typed_throughout(&self, written: TypeId) -> bool {
        let mut current = written;
        loop {
            match self.ast.ty(current) {
                Type::Array {
                    element, length, ..
                } => {
                    if let Some(length) = *length {
                        if self.types.value(length).is_none() && !self.typed_throughout(length) {
                            return false;
                        }
                    }
                    current = *element;
                }
                Type::Pointer(pointee) => current = *pointee,
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Void
                | Type::VaList
                | Type::Function { .. }
                | Type::Struct { .. } => {
                    return true;
                }
            }
        }
    }

    /// Every prototype a type holds, each parameter asked
    /// [`Self::lengths_typed_throughout`], and `None` at the first refusal.
    ///
    /// For the type of a file-scope object, which nothing else lowers while
    /// nothing uses it, so `int (*fp)(int *p, int *q, int n, int a[(*(p - q)
    /// = 0, n)]);` built with the fault a function's own prototype is refused
    /// for (#448). Through pointers, array elements, struct members, return
    /// types and each parameter's written type, so a prototype nested
    /// anywhere in it is asked too. A worklist, as the other walks of a type
    /// here are.
    ///
    /// `walked` is every type this declaration has walked already, so a node
    /// two declarators share is asked once.
    fn prototypes_lose_nothing(
        &self,
        ty: TypeId,
        walked: &mut HashSet<TypeId>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let mut work = vec![ty];
        while let Some(ty) = work.pop() {
            if !walked.insert(ty) {
                continue;
            }
            match self.ast.ty(ty) {
                Type::Pointer(pointee) => work.push(*pointee),
                Type::Array { element, .. } => work.push(*element),
                Type::Struct { members, .. } => {
                    work.extend(members.iter().flatten().map(|member| member.written));
                }
                Type::Function {
                    returns,
                    parameters,
                    ..
                } => {
                    if let Parameters::Prototype(parameters) = parameters {
                        for parameter in parameters {
                            if !self.lengths_typed_throughout(parameter.written) {
                                self.refuse_parameter(parameter, diagnostics);
                                return None;
                            }
                            work.push(parameter.written);
                        }
                    }
                    work.push(*returns);
                }
                // Written out rather than `_`, so a type added later is
                // asked whether it can hold a prototype.
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Void
                | Type::VaList => {}
            }
        }
        Some(())
    }

    /// Whether a parameter written as `written` loses nothing by being the
    /// pointer C17 6.7.6.3 p7 adjusts it to.
    ///
    /// The parser adjusts every array parameter, and `Declaration::ty` is
    /// what this stage lowers. What the adjustment throws away is the
    /// array's own declarator: its length, which 6.9.1 p10 evaluates on entry
    /// to the function where it is not a constant, so `int a[(free(p), 1)]`
    /// frees `p` and `int a[*p = 1]` writes through it. The outermost length
    /// is the one the adjustment drops, and [`Lowering::lengths`] evaluates
    /// it in a definition, so it may be any expression the type check typed
    /// all the way down. One with any part left untyped is refused, since
    /// nothing would ever check it in a prototype, where 6.7.6.2 p5 treats
    /// it as `*` and nothing runs it: the driver does not lower a tree the
    /// type check reported about, so a part untyped here was left so without
    /// a report, and a fault can sit behind it, as `*g2 = 0` with `g2` an
    /// array of arrays does in `int a[(*g2 = 0, n)]`, where the comma is
    /// typed by its right operand. See [`Lowering::typed_throughout`].
    /// Every inner length stays in the adjusted pointer's type, which this
    /// IR cannot hold unless it is a positive number. The element and a
    /// constant length are `types.rs`'s to refuse first, under C17 6.7.6.2
    /// p1, and this is a defence for them. A prototype inside a file-scope
    /// object's type is asked too, through [`Self::prototypes_lose_nothing`].
    fn adjusted_without_loss(&self, written: TypeId) -> bool {
        let mut current = written;
        let mut array = false;
        while let Type::Array {
            element,
            length,
            written: _,
        } = self.ast.ty(current)
        {
            if let Some(length) = *length {
                let accepted = match self.types.value(length) {
                    Some(value) => value > 0,
                    None => !array && self.typed_throughout(length),
                };
                if !accepted {
                    return false;
                }
            }
            array = true;
            current = *element;
        }
        !array || !matches!(self.ast.ty(current), Type::Void | Type::Function { .. })
    }

    /// Whether `root` and every expression under it has a type.
    fn typed_throughout(&self, root: ExprId) -> bool {
        self.first_untyped(root).is_none()
    }

    /// The first expression at or under `root` with no type, which the
    /// refusal of a parameter's length points at.
    ///
    /// A loop over [`Expr::extend_children`] rather than a recursion, since a
    /// length is as deep as its source makes it.
    fn first_untyped(&self, root: ExprId) -> Option<ExprId> {
        let mut work = vec![root];
        while let Some(id) = work.pop() {
            if self.types.of(id).is_none() {
                return Some(id);
            }
            self.ast.expr(id).extend_children(&mut work);
        }
        None
    }

    /// Whether an expression is an integer constant expression, which is
    /// whether `types.rs` gave it a value (C17 6.6 p6).
    ///
    /// Never one with a comma operator in it, which 6.6 p3 forbids and which
    /// lowers to a constant anyway: `(x, 1)` has no value there, so
    /// [`decided`] leaves `while ((x, 1)) {}` a `Branch`.
    fn constant_expression(&self, expr: ExprId) -> bool {
        self.types.value(expr).is_some()
    }

    /// Lower every function that has a body.
    fn define(&mut self, diagnostics: &mut DiagnosticSink) {
        for index in 0..self.ast.items().len() {
            let Item::Function(function) = &self.ast.items()[index] else {
                continue;
            };
            let (name, ty, body, written) = (
                function.name,
                function.ty,
                function.body,
                function.return_nullability,
            );
            let hatch = function
                .attribute
                .is_some_and(|attribute| self.resolution.is_hatch(attribute));

            let Some(id) = self.functions.get(self.sources.snippet(name)).copied() else {
                continue;
            };
            let Some(mut built) = self.body(name, ty, body, hatch, written, diagnostics) else {
                continue;
            };
            if self.internal.contains(self.sources.snippet(name)) {
                built = built.with_internal_linkage();
            }

            self.unit.fill_function(id, built);
        }
    }

    /// One function's blocks, or nothing where something could not be lowered.
    ///
    /// `hatch` is whether `sema::resolve` accepted the attribute written before
    /// it as a hatch. See ADR-0038. `written` is the nullability specifier on
    /// the pointer it returns.
    fn body(
        &mut self,
        name: Span,
        ty: TypeId,
        body: StmtId,
        hatch: bool,
        written: Option<Nullability>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Function> {
        let Type::Function {
            returns,
            parameters,
            variadic,
        } = self.ast.ty(ty)
        else {
            return None;
        };
        let variadic = *variadic;
        let (returns, parameters) = (*returns, parameters.clone());
        let returns_reads_only =
            matches!(self.ast.ty(returns), Type::Pointer(pointee) if self.read_only(*pointee));
        let returns_const_levels = self.const_levels(returns);
        let (returns, lowered) = self.signature(name, returns, &parameters, diagnostics)?;
        let mut function = Function::with_parameters(name, returns, lowered);
        if !self.standing_prototyped(name, &parameters) {
            function = function.unprototyped();
        }
        // **A body that reads past its parameters**, through
        // `__builtin_va_start` (#1214). See ADR-0126.
        if variadic {
            function = function.variadic();
        }
        if returns_reads_only {
            function.mark_returns_reads_only();
        }
        let return_place = function.return_place();
        function.mark_const_levels(return_place, returns_const_levels);
        // The written parameters, after the pointer a struct result goes
        // through where there is one.
        let hidden = usize::from(function.returns_a_struct());
        if let Parameters::Prototype(declared) = &parameters {
            for (index, (parameter, local)) in declared
                .iter()
                .zip(function.parameters().skip(hidden).collect::<Vec<_>>())
                .enumerate()
            {
                if function.passes(index + hidden) == Passes::StructCopy {
                    function.mark_const_levels(local, self.pointing_at_levels(parameter.ty));
                    continue;
                }
                if self.read_only(parameter.ty) {
                    function.mark_read_only(local);
                }
                function.mark_const_levels(local, self.const_levels(parameter.ty));
            }
        }
        if hatch {
            function = function.hatched();
        }
        // The definition carries its own promise, since it replaces the
        // declaration callers were lowered against, and `agree_on_return` has
        // made every declaration say the same.
        //
        // **Level 5's default is made here, on a body, and nowhere else.** A
        // function this unit only declares returns `_Nullable` where it says
        // neither, since nobody here can ask its body (ADR-0050). A definition
        // this stage refused stays the declaration `declare_one` made, which
        // carries what was written and no default, and the build has already
        // failed on `SC0304` beside it.
        //
        // **Not on a hatch.** What a hatch's body could not prove is listed
        // rather than reported (ADR-0038), so a default promise on one would
        // be believed by every caller and asked by nobody: a promise no one
        // wrote. A hatch promises what it writes, and its callers assume the
        // worst of the rest.
        let promise = match written {
            Some(_) => written_promise(written),
            None => (self.nonnull_returns_by_default
                && !hatch
                && matches!(self.unit.ty(returns), Ty::Pointer(_)))
            .then_some(Promise::Defaulted(name)),
        };
        if let Some(promise) = promise {
            function = function.promising(promise);
        }

        // A parameter is a local before the body's first statement, and its
        // name is how the body reaches it.
        let mut struct_parameters = Vec::new();
        if let Parameters::Prototype(parameters) = &parameters {
            for (index, (parameter, local)) in parameters
                .iter()
                .zip(function.parameters().skip(hidden))
                .enumerate()
            {
                if function.passes(index + hidden) == Passes::StructCopy {
                    struct_parameters.push((parameter.name, parameter.ty, local));
                } else if let Some(at) = parameter.name {
                    self.locals.insert(at, local);
                }
            }
        }

        let mut builder = Builder {
            function,
            block: None,
            elements: Vec::new(),
            bit_fields: HashMap::new(),
        };
        builder.open();
        self.labeled = false;

        // **A struct passed by value is copied again, into a local of the
        // callee's own, and that local is the parameter's name** (#1201).
        // C17 6.9.1 p9 makes a parameter an object of the callee with
        // automatic storage, and the pointer the IR passes names the caller's
        // copy, which every check reads as the caller's: `free(&s)` was a free
        // of a pointer the caller handed over, silent where C makes it
        // undefined, and `s.p = q` a store the caller could read. As a local,
        // `s` is everything a local is to every check, with nothing for any of
        // them to learn. What it costs is a copy at run time.
        //
        // Mutation: name the pointer's target `*p` instead of the copy;
        // `a_struct_is_assigned_passed_and_returned_by_value`'s `freed_own`
        // loses its `SC0401`.
        self.argument_copies.clear();
        self.vla_types.clear();
        self.typedef_lengths.clear();
        self.typedef_locals.clear();
        for (written, ty, pointer) in struct_parameters {
            let at = written.unwrap_or(name);
            let lowered = self.ty(at, ty, diagnostics)?;
            let copy = builder.function.push_local(lowered);
            let levels = self.const_levels(ty);
            builder.function.mark_const_levels(copy, levels);
            self.copy_struct(
                &mut builder,
                Place::local(copy),
                Place::local(pointer).deref(),
                at,
            )?;
            if let Some(written) = written {
                self.locals.insert(written, copy);
            }
        }
        self.lengths(&mut builder, &parameters, diagnostics)?;

        // C17 5.1.2.2.3 p1: reaching the `}` that terminates `main` returns
        // zero. Written at the top rather than beside the fall-off below,
        // because a `goto` can reach that `}` from anywhere and because this is
        // what `clang` does: the return place holds zero from the first
        // instruction and a `return` of its own overwrites it.
        //
        // By name, which is what C17 5.1.2.2.1 p1 calls the entry point. The IR
        // says nothing about which function that is, so it is the C frontend's
        // to know, which is what the note this replaces pointed here for. A
        // `main` returning anything but `int` is not one 5.1.2.2.1 describes,
        // and writing a zero into its return place would be writing into a
        // `void`.
        if self.sources.snippet(name) == "main" && self.unit.ty(returns) == Ty::INT {
            builder.push(Operation {
                place: Place::local(builder.function.return_place()),
                value: Rvalue::Use(Operand::Constant(0)),
                origin: Origin::Generated(name),
            });
        }

        self.stmt(&mut builder, body, diagnostics)?;

        // Falling off the end returns whatever the return place holds, which
        // C17 6.9.1 p12 makes undefined to read. `main` is the exception and
        // has already written its zero above.
        if builder.reachable() {
            builder.end(Terminator::Return);
        }

        // See `Function::thread_rebranches`. After the whole body, so that
        // every block a branch may lead into is filled.
        let mut function = builder.function;
        function.thread_rebranches();
        Some(function)
    }

    /// C17 6.9.1 p10: on entry to a definition, the length of each parameter
    /// whose outermost array length is not a constant is evaluated, and its
    /// value discarded, since the parameter is the adjusted pointer.
    ///
    /// **One unsequenced group, and one sequence point after it.** A
    /// parameter's declarator is part of the function's, so it is not a full
    /// declarator and its end is no sequence point (6.7.6 p3); nothing orders
    /// one parameter's length against another's, and 6.9.1 p10 names no
    /// order. Each is walked as a call's argument is, with
    /// [`Lowering::top_level`] false, so a sequence point inside one, a
    /// comma's, is not recorded as ordering it against the rest (ADR-0022),
    /// and `int b[*p], int a[(free(p), 1)]` is not read as the read coming
    /// first. Each value is held, as an argument is until its call, and read
    /// only once every length is lowered. The end of the function's
    /// declarator is a full declarator's end, so one [`Element::Sequenced`]
    /// follows them all and orders them before the body. Lowered in the
    /// order written, which is an order C allows and the one the backend
    /// emits; the missing markers and the held values are what stop an
    /// analysis concluding from it.
    ///
    /// **And asked for conflicts against each other**, as a call's arguments
    /// are, because nothing else sets two lengths side by side: each is
    /// walked on its own, so `int a[g(p, 1)], int b[(p = 0, 1)]` exited 0
    /// where the same pair in a body is `SC0409` (#529). See ADR-0052.
    fn lengths(
        &mut self,
        builder: &mut Builder,
        parameters: &Parameters,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let Parameters::Prototype(parameters) = parameters else {
            return Some(());
        };
        // Every length's value is held and discarded only once all of them
        // are lowered, as a call's arguments are read only at the call. A
        // value discarded where its length finishes is read there, before
        // the next length is lowered, and `int b[*p], int a[(p = 0, 1)]`
        // then checks `*p` against the `p` before the write C may run first.
        let mut held = Vec::new();
        let mut walked = Vec::new();
        let mut declared = Vec::new();
        for parameter in parameters {
            let Type::Array {
                length: Some(length),
                ..
            } = self.ast.ty(parameter.written)
            else {
                continue;
            };
            let length = *length;
            if self.types.value(length).is_some() {
                continue;
            }
            let span = self.ast.expr(length).span();
            let value = self.walk(builder, length, false, diagnostics)?;
            held.push((value, span));
            walked.push(length);
            declared.push(parameter.span);
        }
        // From the first such parameter to the last, spans the parser joined,
        // rather than from the first length's node to the last's: a length's
        // node leaves out the brackets and any parenthesis it was written in,
        // so `int b[*q]` included as a parameter starts its length at `*q`,
        // partway through the header, and refused C that cuts nothing. Found
        // by review.
        //
        // **And over the whole parameter list where that would cut a
        // header**: `int f(` then `#include` of `int n, int b[n]` then `, int
        // a[n])` starts the first length's parameter partway through the
        // header, and the list holds it whole. Only there, so that every other
        // span over lengths stays what it was (#555). The list adds parameters
        // and no reads, since a parameter's only expressions are its lengths.
        //
        // Mutation: drop the fallback.
        // `a_parameter_with_a_length_after_one_without_from_a_header_is_read_as_written_out`
        // is refused as `SC0304`.
        if let (Some(&first), Some(&last)) = (declared.first(), declared.last()) {
            let list = parameters
                .first()
                .zip(parameters.last())
                .and_then(|(first, last)| self.try_joined(first.span, last.span));
            let at = match (self.try_joined(first, last), list) {
                (Some(at), _) | (None, Some(at)) => at,
                (None, None) => self.joined(first, last, diagnostics)?,
            };
            let each: Vec<_> = walked
                .iter()
                .map(|&length| self.accesses(length, false))
                .collect();
            let conflicts = self.conflicts(
                &builder.function,
                &each,
                Between::Lengths,
                at,
                &mut Vec::new(),
            );
            for element in conflicts {
                builder.element(element);
            }
        }
        let last = held.last().map(|(_, span)| *span);
        for (value, span) in held {
            if let Some(value) = value {
                builder.discarded(value, span);
            }
        }
        if let Some(span) = last {
            builder.sequenced(span);
        }
        Some(())
    }

    /// The IR struct the struct type `ty` is, laid out from its definition's
    /// members the first time its tag is met.
    ///
    /// **Declared before its members are lowered**, so that a member pointing
    /// at the struct itself, `struct L { struct L *next; }`, is a pointer to
    /// the struct being laid out (C17 6.7.2.3 p8): a pointer needs no layout
    /// of what it points at. A struct nothing defines is declared and never
    /// laid out, which a pointer to it is all it can be, the type check
    /// refusing an object of one. A member of struct type is laid out
    /// first, at its alignment, its offsets adding up under the outer's.
    /// Mutation: cache the id only after the members are lowered;
    /// `a_nested_struct_and_an_array_of_structs_are_read_by_member` in
    /// `tests/run.rs` overflows the stack.
    ///
    /// **Laid out only where `lay_out` asks**, which a struct behind a
    /// pointer does not, so a pointer chain is declared and not walked.
    /// **Refused where a member is refused**, a struct with `const` storage
    /// in it, and one larger than the target can address, under the code for
    /// what this compiler does not do yet, at the declaration that needed
    /// it, and left declared and incomplete: a use that needs its layout asks
    /// again and is refused again.
    fn struct_of(
        &mut self,
        at: Span,
        ty: TypeId,
        lay_out: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<StructId> {
        let tag = self.resolution.tag(self.resolution.bound(self.ast, ty));
        let id = match self.structs.get(&tag) {
            Some(&id) => id,
            None => {
                // **A union is a struct declared a union** (#1203), and laid
                // out by its rule. See `Ty::Struct`.
                let id = if matches!(self.ast.ty(ty), Type::Struct { union: true, .. }) {
                    self.unit.declare_union()
                } else {
                    self.unit.declare_struct()
                };
                self.structs.insert(tag, id);
                self.written.insert(id, (at, ty));
                id
            }
        };
        if !lay_out || self.unit.layout_of(id).is_some() {
            return Some(id);
        }
        // Refused already, which was said where it was.
        if self.refused_structs.contains(&tag) {
            return None;
        }
        let not_yet = |diagnostics: &mut DiagnosticSink, why: &str| {
            diagnostics.report(
                Diagnostic::error(format!(
                    "cannot compile something of type `{}` yet",
                    spell_type(self.sources, self.ast, ty)
                ))
                .with_code(LOWERING)
                .with_label(Label::primary(at, "declared here"))
                .with_note(why.to_owned()),
            );
        };
        let Some(definition) = self.resolution.definition(tag) else {
            return Some(id);
        };
        let Type::Struct {
            members: Some(members),
            ..
        } = self.ast.ty(definition)
        else {
            unreachable!("a tag's definition is a struct with members");
        };
        // A struct that holds itself by value is incomplete inside itself,
        // which the type check refuses (C17 6.7.2.1 p3); one met again here
        // is a gap that left, said rather than recursed into.
        if !self.laying_out.insert(tag) {
            not_yet(diagnostics, "a struct that holds itself is not laid out");
            return None;
        }
        let mut lowered = Vec::with_capacity(members.len());
        let mut refused = false;
        for (index, member) in members.iter().enumerate() {
            // **A `const` member is refused**, since the memory check keeps
            // what is defined `const` per object, a static's or a local's
            // (ADR-0120), and a member is a slot of an object that is not:
            // `int *m = (int *)&v; m[1] = 1;` wrote a `const` member with no
            // report (#1200 review). Mutation: lay one out;
            // `a_struct_this_compiler_cannot_lay_out_yet_is_refused`'s
            // `Fixed` is lowered.
            if self.holds_const_storage(member.ty) {
                not_yet(
                    diagnostics,
                    "a struct with a `const` member is not laid out yet",
                );
                refused = true;
                break;
            }
            // **A flexible array member is an array of none** (C17 6.7.2.1
            // p18, #1208): the last member, `T d[]`, takes no bytes of the
            // struct, and its elements are what was allocated past it. The
            // type check holds it to the last place and to a complete
            // element. See `Ty::Array`.
            let lowered_ty = match self.ast.ty(member.ty) {
                Type::Array {
                    element,
                    length: None,
                    ..
                } if index + 1 == members.len() => {
                    self.ty(member.span, *element, diagnostics).map(|element| {
                        self.unit.push_type(Ty::Array {
                            element,
                            length: Length::Constant(0),
                        })
                    })
                }
                _ => self.ty(member.span, member.ty, diagnostics),
            };
            match lowered_ty {
                Some(ty) => lowered.push(Declared {
                    ty,
                    // The type check held every width to a constant that
                    // fits, and a run it reported on stops before here.
                    width: member.width.map(|width| {
                        self.types
                            .value(width)
                            .and_then(|value| u64::try_from(value).ok())
                            .expect("the type check gives every bit-field a width that fits")
                    }),
                    named: member.name.is_some(),
                }),
                None => {
                    refused = true;
                    break;
                }
            }
        }
        self.laying_out.remove(&tag);
        // Only where nothing was refused: a layout of the members lowered
        // before a refused one would be a struct laid out short.
        if !refused && self.unit.complete_record(id, &lowered).is_none() {
            not_yet(
                diagnostics,
                "a struct too large to count in bytes is not laid out",
            );
            refused = true;
        }
        // **No larger than the target can address**, as `types.rs` holds an
        // array to: three members that each fit made a struct none could,
        // and its `alloca` built a program that does not start (#1200
        // review). Mutation: skip this;
        // `a_struct_this_compiler_cannot_lay_out_yet_is_refused`'s `Huge`
        // is lowered.
        let limit = 1u64 << self.unit.target().pointer_bits().min(61);
        if !refused && self.unit.struct_layout(id).size >= limit {
            not_yet(
                diagnostics,
                "a struct larger than this target can address is not laid out",
            );
            refused = true;
        }
        // **Left declared, and not laid out, where it is refused**: a pointer
        // to it is still a pointer, and a use that needs its layout is
        // refused, said once at the first, as one struct reached by two uses
        // was said twice at one caret (#1202 review).
        if refused {
            self.refused_structs.insert(tag);
            return None;
        }
        Some(id)
    }

    /// Whether a member of type `ty` is storage defined `const`: the member
    /// itself, or the elements of an array, however deep. **The elements
    /// too** (#1202 review): `const int a[2]` is an array of `const int`,
    /// which `Ast::is_const` of the array type does not say, and the member
    /// was laid out and written through a cast in silence. Mutation: ask the
    /// member's own type alone; `a_struct_this_compiler_cannot_lay_out_yet_is_refused`'s
    /// `Rows` is lowered.
    fn holds_const_storage(&self, ty: TypeId) -> bool {
        let mut ty = ty;
        loop {
            if self.ast.is_const(ty) {
                return true;
            }
            match self.ast.ty(ty) {
                Type::Array { element, .. } => ty = *element,
                Type::Integer { .. }
                | Type::Void
                | Type::VaList
                | Type::Pointer(_)
                | Type::Function { .. }
                | Type::Struct { .. } => return false,
            }
        }
    }

    /// The length an array type was written with, where the type check gave
    /// it a value that is positive and fits a `u64`: what `ir::Ty::Array`
    /// holds. `None` for a variable length, and for one that is zero or
    /// negative, which C17 6.7.6.2 p1 forbids and `types.rs` refuses first.
    fn positive_length(&self, length: ExprId) -> Option<u64> {
        self.types
            .value(length)
            .and_then(|value| u64::try_from(value).ok())
            .filter(|&value| value > 0)
    }

    /// The IR type of a type the frontend wrote.
    ///
    /// A loop rather than a recursion, following `ast::spell_type`: the chain
    /// is bounded by the parser, and a walk over a type is the shape that stops
    /// being bounded first when something else builds one.
    fn ty(&mut self, at: Span, id: TypeId, diagnostics: &mut DiagnosticSink) -> Option<TyId> {
        // Each derivation of the written type, outermost first, so that the IR
        // type is built back up from the base in the order C nests them:
        // `int (*)[4]` is a pointer to an array and `int *[4]` an array of
        // pointers. `None` is a pointer and `Some(length)` an array.
        let mut derivations = Vec::new();
        let mut current = id;

        let base = loop {
            match self.ast.ty(current) {
                Type::Integer { rank, sign } => {
                    break Ty::Integer {
                        rank: *rank,
                        sign: *sign,
                    };
                }
                Type::Void => break Ty::Void,
                // **An array of integers as wide as the list is aligned**,
                // as many as fill the target's size: the size and alignment
                // `clang` gives it, and nothing any reader takes for a
                // pointer, whatever the target makes it (#1214). See
                // ADR-0126.
                Type::VaList => {
                    let (size, align) = self.unit.target().va_list();
                    derivations.push(Some(size / align));
                    break Ty::Integer {
                        rank: if align == 8 {
                            Rank::LongLong
                        } else {
                            Rank::Int
                        },
                        sign: safec_ir::ir::Sign::Unsigned,
                    };
                }
                Type::Pointer(pointee) => {
                    derivations.push(None);
                    current = *pointee;
                }
                // **An array whose length the type check gave a positive
                // value**, which is what C17 6.7.6.2 p1 asks of a constant
                // length. Not a pointer: saying it is would tell the memory
                // analysis that one object is another. A length with no value
                // is a variable length, which `Lowering::variable_length`
                // holds only for the declared object's own array, so one
                // reached here, nested or behind a pointer, is refused.
                Type::Array {
                    element,
                    length: Some(length),
                    written: _,
                } if self.positive_length(*length).is_some() => {
                    derivations.push(self.positive_length(*length));
                    current = *element;
                }
                // **A struct is laid out once, by the IR**, from its members'
                // types, and every type that names its tag is that one
                // `Ty::Struct`. See ADR-0124.
                Type::Struct { .. } => {
                    // **Laid out only where it is not behind a pointer**,
                    // which needs no layout of what it points at: `struct
                    // A { struct B *b; }` with `struct B { struct A a; }`
                    // laid `B` out from inside `A`'s layout, met `A`
                    // unfinished and refused a struct that is fine (#1202
                    // review). Mutation: lay it out behind a pointer too;
                    // `a_nested_struct_and_an_array_of_structs_are_read_by_member`
                    // in `tests/run.rs` does not build.
                    let behind_a_pointer = derivations.iter().any(Option::is_none);
                    break Ty::Struct(self.struct_of(
                        at,
                        current,
                        !behind_a_pointer,
                        diagnostics,
                    )?);
                }
                // **A function behind a pointer is its signature** (#1212),
                // laid out as a declaration's parameters are, so a call
                // through the pointer passes what a direct call would. A
                // function type anywhere else is no object's type, and is
                // refused below as before. See ADR-0125.
                Type::Function {
                    returns,
                    parameters,
                    variadic,
                } if matches!(derivations.last(), Some(None)) => {
                    let variadic = *variadic;
                    let (returns, parameters) = (*returns, parameters.clone());
                    let prototyped = matches!(parameters, Parameters::Prototype(_));
                    let (returns, lowered) =
                        self.signature(at, returns, &parameters, diagnostics)?;
                    let signature = Signature {
                        returns,
                        parameters: lowered
                            .iter()
                            .map(|parameter| (parameter.ty, parameter.passes))
                            .collect(),
                        prototyped,
                        variadic,
                    };
                    break Ty::Function(self.unit.push_signature(signature));
                }
                // #74, which is the issue for what the IR cannot say, is about
                // storage duration rather than about these.
                Type::Array { .. } | Type::Function { .. } => {
                    diagnostics.report(
                        Diagnostic::error(format!(
                            "cannot compile something of type `{}` yet",
                            spell_type(self.sources, self.ast, id)
                        ))
                        .with_code(LOWERING)
                        .with_label(Label::primary(at, "declared here"))
                        .with_note("the integer types, `void`, structs, pointers to them, arrays of them with a constant length, and a variable length array of them declared as an object are all this compiler holds so far"),
                    );
                    return None;
                }
            }
        };

        let mut ty = self.unit.push_type(base);
        for derivation in derivations.into_iter().rev() {
            ty = self.unit.push_type(match derivation {
                None => Ty::Pointer(ty),
                Some(length) => Ty::Array {
                    element: ty,
                    length: Length::Constant(length),
                },
            });
        }

        Some(ty)
    }

    /// What the frontend said this expression's type is, or a report that it
    /// said nothing.
    ///
    /// Asked of every expression as it is reached, and not only of the ones
    /// that need a temporary to hold them. `x[p - q]` where `x` is an `int`
    /// needs none: it is a place, and lowering it without asking would have
    /// built a projection into an object with no elements while the type
    /// checker had already answered that it does not know what this is.
    fn typed(&mut self, id: ExprId, diagnostics: &mut DiagnosticSink) -> Option<TypeId> {
        let span = self.ast.expr(id).span();
        let Some(ty) = self.types.of(id) else {
            // The match below is the list of expressions `types.rs` reports
            // about itself: a constant has no type exactly when the frontend
            // could not read its spelling, and said so at this span with
            // `SC0106` or `SC0305`. A second report here would put two carets
            // on one problem, and its note, that whether the expression is
            // valid C was not checked, would contradict the report that just
            // checked `123abc`. Every expression *not* in the list is a gap
            // nobody has reported yet, which is what the message says. The
            // note does not say the program is valid, because some of those
            // are not: `(p - q) < p` breaks C17 6.5.8 p2, and the type check
            // could not ask while `p - q` had no type, which it has since
            // #638; no untyped expression is measured now (#1298).
            //
            // The driver does not lower a tree the type check reported
            // about, so no run reaches the list today. It is kept as the
            // defence it was: were that gate to open, a constant's error
            // would not gain a second caret and a note that contradicts it.
            if !matches!(
                self.ast.expr(id),
                Expr::Number { .. } | Expr::Character { .. } | Expr::Length { .. }
            ) {
                diagnostics.report(
                    Diagnostic::error("cannot compile an expression whose type is not known")
                        .with_code(LOWERING)
                        .with_label(Label::primary(
                            span,
                            "nothing worked out what type this has",
                        ))
                        .with_note(
                            "this is a gap in this compiler: whether this expression is valid C is not checked",
                        ),
                );
            }
            return None;
        };

        Some(ty)
    }

    /// The operand an additive operation yields when it moves the pointer
    /// nowhere.
    ///
    /// **Only where the result is a pointer**, which is the whole of why this
    /// belongs here. The fold exists because `E[0]` and `*E` are one C
    /// expression, and that is a statement about pointers; an integer `c + 0`
    /// is not two spellings of anything, so folding it would be an
    /// optimisation this compiler does not do, and it would drop a promotion
    /// the IR shows the reader. It would not drop a value: measured, the
    /// backend converts every operand where it is consumed, so `c + 0` folded
    /// and unfolded emit the same answer. See ADR-0021.
    ///
    /// C17 6.5.6 p8 is the paragraph: adding an integer to a pointer yields a
    /// pointer to the element that far along, `(P)+N` and `N+(P)` alike, so at
    /// zero it is a pointer to the same element. That is Semantics and it is
    /// what makes this a fold rather than a guess. The Constraints beside it
    /// answer a different question and are cited where that question is asked:
    /// p3 allows the pointer only on the left of a `-`, which is why `0 - E`
    /// is not here, and `types.rs` cites the same paragraph where it declines
    /// to give `1 - p` a type. Reading one of those for the other is a habit
    /// rather than an accident: the same mistake was caught in 6.5.3.2, where
    /// a Semantics paragraph was read as a Constraint.
    fn unmoved(ty: Ty, op: BinOp, lhs: &Operand, rhs: &Operand) -> Option<Operand> {
        if !matches!(ty, Ty::Pointer(_)) {
            return None;
        }

        // Which operand is the pointer is read off which side holds the
        // literal, and that is sound only because an `Operand::Constant` is
        // always an integer: a constant expression is the one thing that
        // builds one, and `types.rs` gives a value only to an expression of
        // integer type. A cast or a string literal would break that, and `((int *)0)[n]`
        // would fold to `n`, which is a wrong **value** rather than a wrong
        // report. Neither is implemented; when one is, this decides by the
        // operand's type instead.
        match op {
            BinOp::Add if matches!(rhs, Operand::Constant(0)) => Some(lhs.clone()),
            BinOp::Add if matches!(lhs, Operand::Constant(0)) => Some(rhs.clone()),
            BinOp::Sub if matches!(rhs, Operand::Constant(0)) => Some(lhs.clone()),
            _ => None,
        }
    }

    /// The IR type of an expression, or a report that it has none.
    fn ty_of(&mut self, id: ExprId, diagnostics: &mut DiagnosticSink) -> Option<TyId> {
        let ty = self.typed(id, diagnostics)?;
        // **A name of a variable length array is the array its local is**
        // (#1209): its length is the local it was worked out into where it
        // was declared, which the C type, `T[n]`, does not say. Only a
        // length with no value, so a constant array is never answered from
        // the table, whatever it holds.
        if let (
            Expr::Identifier { .. },
            Type::Array {
                length: Some(length),
                ..
            },
        ) = (self.ast.expr(id), self.ast.ty(ty))
        {
            if self.types.value(*length).is_some() {
                return self.ty(self.ast.expr(id).span(), ty, diagnostics);
            }
            if let Some(&local) = self
                .resolution
                .resolved(id)
                .and_then(|binding| self.locals.get(&self.resolution.binding(binding).name))
            {
                if let Some(builder_ty) = self.vla_types.get(&local) {
                    return Some(*builder_ty);
                }
            }
        }
        // **An access to a flexible array member is an array of none**, as
        // its struct was laid out (#1208): `T[]` is no type the IR holds
        // anywhere else, and is not one here, where the access names it.
        if let (
            Some(_),
            Type::Array {
                element,
                length: None,
                ..
            },
        ) = (self.types.member(id), self.ast.ty(ty))
        {
            let element = self.ty(self.ast.expr(id).span(), *element, diagnostics)?;
            return Some(self.unit.push_type(Ty::Array {
                element,
                length: Length::Constant(0),
            }));
        }
        self.ty(self.ast.expr(id).span(), ty, diagnostics)
    }

    /// A statement, and everything under it.
    ///
    /// Recursion is what the statements a walker meets are bounded by:
    /// `parser::MAX_NESTING` counts a nested statement and not a folded
    /// operator, which is why `driver/dumps.rs`'s `dump_stmt` recurses where
    /// its `dump_expr` does not.
    fn stmt(
        &mut self,
        builder: &mut Builder,
        id: StmtId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        match self.ast.stmt(id) {
            Stmt::Compound { body, span } => {
                let (body, span) = (body.clone(), *span);
                self.open_scope();

                // Not `?`: the scope has to be closed whether or not a
                // statement inside it could be lowered, or the next compound
                // at this depth would inherit an entry that is still open.
                let mut lowered = Some(());
                for statement in body {
                    if self.stmt(builder, statement, diagnostics).is_none() {
                        lowered = None;
                        break;
                    }
                }

                // The function's own body collects nothing, because the
                // `Declaration` arm only records a local when a scope narrower
                // than the body is open, so there is nothing to close for it
                // and no need to ask whether this is that one.
                let declared = self.scopes.pop().expect("the scope this arm pushed");
                end_storage(builder, &declared.declared, span);
                lowered?;
            }
            Stmt::Return { value, span } => {
                let (value, span) = (*value, *span);
                // **A struct is returned into where the caller said**, the
                // pointer passed first, and the function returns nothing
                // (#1201).
                if let Some(value) = value.filter(|_| builder.function.returns_a_struct()) {
                    let from = self.place_of(builder, value, diagnostics)?;
                    let into = builder
                        .function
                        .parameters()
                        .next()
                        .expect("a struct result's pointer is the first parameter");
                    self.copy_struct(builder, Place::local(into).deref(), from, span)?;
                    builder.sequenced(self.ast.expr(value).span());
                    self.leave_scopes(builder, 0, span);
                    builder.end(Terminator::Return);
                    return Some(());
                }
                if let Some(value) = value {
                    let operand = self.value(builder, value, diagnostics)?;
                    let place = Place::local(builder.function.return_place());
                    builder.push(Operation {
                        place,
                        value: Rvalue::Use(operand),
                        origin: Origin::Written(span),
                    });
                    builder.sequenced(self.ast.expr(value).span());
                }
                // Every scope this leaves ends here, after the value is read
                // (6.8.6.4 p3) and in the order the `Compound` arm closes one
                // in, innermost first. After, and not only for C's sake: the
                // memory check asks about a returned pointer at the write into
                // the return place, and a local cleared before that write
                // reaches no allocation, so a freed one returned out of a
                // nested scope would go unreported. `Generated` for the
                // `Compound` arm's reason, and spanned by the `return` rather
                // than by each `}`, because control never reaches those braces
                // and this statement is why the storage ends. The function's
                // own body is the first scope, and holds only its variable
                // length arrays (#1209), whose storage a `return` gives back
                // like any other's, so no depth test is needed. See ADR-0012.
                self.leave_scopes(builder, 0, span);
                builder.end(Terminator::Return);
            }
            Stmt::Declaration {
                declarators,
                storage: Some(storage),
                ..
            } => {
                for declarator in declarators {
                    self.block_static(builder, declarator, *storage, diagnostics)?;
                }
            }
            Stmt::Declaration {
                declarators,
                typedefs,
                specified,
                ..
            } => {
                let specified = *specified;
                for &ty in typedefs {
                    self.typedef_lengths_of(builder, ty, diagnostics)?;
                }
                for declarator in declarators {
                    let (name, ty, span) = (
                        declarator.declaration.name,
                        declarator.declaration.ty,
                        declarator.declaration.span,
                    );
                    // A declaration with no name declares nothing to write to, and
                    // one with no initializer writes nothing: a local is made and
                    // left alone until something assigns to it.
                    if let Some(name) = name {
                        let read_only = self.read_only(ty);
                        let const_levels = self.const_levels(ty);
                        let variable =
                            self.variable_length(builder, ty, specified, span, diagnostics)?;
                        let ty = match variable {
                            Some(ty) => ty,
                            None => self.ty(span, ty, diagnostics)?,
                        };
                        let local = builder.function.push_named_local(ty, name);
                        if variable.is_some() {
                            self.vla_types.insert(local, ty);
                        }
                        if read_only {
                            builder.function.mark_read_only(local);
                        }
                        builder.function.mark_const_levels(local, const_levels);
                        self.locals.insert(name, local);

                        // Only a scope narrower than the function's own body. The
                        // first entry is the body, and ADR-0012 says why a local
                        // that lives as long as the frame needs no marker.
                        //
                        // **Except where a label comes before it**: a backward
                        // `goto` can then reach the declaration a second time,
                        // and C17 6.2.4 p6 makes the value indeterminate each
                        // time it is reached. Without the marker the value of
                        // the last pass was carried round and read as if it
                        // were still there, a free of it built in silence.
                        // Marked, and not added to the body's scope, so that
                        // no jump and no `return` ends a frame's local. Found
                        // by review. See ADR-0114.
                        //
                        // **A variable length array is both, even in the
                        // body** (#1209). Its storage is made at this marker,
                        // as large as its length then is, which the backend
                        // does here; and it is in its scope's list, the
                        // body's too, so that a jump past its declaration is
                        // found and refused and one back before it ends it
                        // (`markers`). Mutations: leave the marker to
                        // `narrower`, and the run of
                        // `a_variable_length_array_is_sized_when_declared`
                        // fails; leave the list to it, and
                        // `a_jump_into_the_scope_of_a_variable_length_array_is_refused`
                        // accepts the jump in the body.
                        let narrower = self.scopes.len() > 1;
                        if narrower || variable.is_some() {
                            let scope = self.scopes.last_mut().expect("a scope is open");
                            scope.declared.push(local);
                        }
                        if narrower || self.labeled || variable.is_some() {
                            // Generated for the same reason as the closing half:
                            // the declaration is what this exists because of, and
                            // is not itself an instruction to begin storage that
                            // somebody wrote.
                            builder.element(Element::StorageLive {
                                local,
                                origin: Origin::Generated(span),
                            });
                        }

                        // After `self.locals` has the name, so that the
                        // initializer of `int a = a;` finds the very thing it is
                        // initializing. C17 6.2.1 p7 opens a name's scope at the
                        // end of its declarator, which is before the `=`, and
                        // `clang` accepts it with a warning rather than an error.
                        // After `StorageLive` as well, because a write to storage
                        // that has not begun is what ADR-0012's pair exists to
                        // make findable.
                        //
                        // `Written` and not `Generated`: the source asked for
                        // this store.
                        //
                        // Its span runs from the name to the end of the
                        // declarator, which for `int a = 1, b = 2;` is `b = 2`
                        // rather than the whole line. `Declaration::span`
                        // deliberately starts every declarator of one
                        // declaration at the specifiers they share, so using
                        // it here would report both of those stores at the
                        // `int` in front of them, pointing a later diagnostic
                        // at the declarator before the one it is about. An
                        // ordinary `a = 1;` already carries a span that starts
                        // at the place being written, so this is what the same
                        // store gets written the other way.
                        if let Some(init) = declarator.init {
                            let at = self.store_span(name, span, declarator.span, diagnostics)?;
                            if matches!(self.ast.expr(init), Expr::String { .. })
                                && matches!(
                                    self.ast.ty(declarator.declaration.ty),
                                    Type::Array { .. }
                                )
                            {
                                self.store_string(builder, local, init, at)?;
                                builder.sequenced(self.ast.expr(init).span());
                                continue;
                            }
                            // A struct is copied from where it is, as an
                            // assignment copies one (#1201).
                            if self.is_struct(init) {
                                let from = self.place_of(builder, init, diagnostics)?;
                                self.copy_struct(builder, Place::local(local), from, at)?;
                                builder.sequenced(self.ast.expr(init).span());
                                continue;
                            }
                            let operand = self.value(builder, init, diagnostics)?;
                            builder.push(Operation {
                                place: Place::local(local),
                                value: Rvalue::Use(operand),
                                origin: Origin::Written(at),
                            });
                            // One per declarator that has an initializer:
                            // `int a = f(), b = g();` is two full expressions
                            // and the `;` is not what separates them.
                            builder.sequenced(self.ast.expr(init).span());
                        }
                    }
                }
            }
            Stmt::Expression { value, .. } => {
                if let Some(value) = *value {
                    self.effect(builder, value, diagnostics)?;
                    builder.sequenced(self.ast.expr(value).span());
                }
            }
            Stmt::If {
                condition,
                then,
                otherwise,
                ..
            } => {
                let (condition, then, otherwise) = (*condition, *then, *otherwise);
                // Before `condition` becomes the operand: `Terminator::Branch`
                // asks for where the expression that decides is written, and
                // one line down there is no expression left to ask.
                let asked = self.ast.expr(condition).span();
                let decides = self.decides(condition);
                let constant = self.constant_expression(condition);
                let condition = self.value(builder, condition, diagnostics)?;
                let taken = builder.function.reserve_block();
                let skipped = builder.function.reserve_block();
                let join = builder.function.reserve_block();
                builder.end(decided(
                    condition,
                    constant,
                    taken,
                    skipped,
                    Origin::Written(decides),
                ));

                builder.enter(taken, Some(asked));
                self.stmt(builder, then, diagnostics)?;
                if builder.reachable() {
                    builder.end(Terminator::Goto(join));
                }

                // An `if` with no `else` still has an edge that skips the body,
                // and it is the same edge as an empty `else`.
                builder.enter(skipped, Some(asked));
                if let Some(otherwise) = otherwise {
                    self.stmt(builder, otherwise, diagnostics)?;
                }
                if builder.reachable() {
                    builder.end(Terminator::Goto(join));
                }

                builder.switch(join);
            }
            Stmt::Break { span } => {
                let span = *span;
                let target = self
                    .targets
                    .last()
                    .expect("the parser reads a `break` only in a loop");
                let (to, depth) = (target.broken, target.depth);
                self.leave_scopes(builder, depth, span);
                builder.end(Terminator::Goto(to));
            }
            Stmt::Continue { span } => {
                let span = *span;
                let target = self
                    .targets
                    .iter_mut()
                    .rev()
                    .find(|target| !matches!(target.continued, Continued::Passes))
                    .expect("the parser reads a `continue` only in a loop");
                let depth = target.depth;
                let to = match &mut target.continued {
                    Continued::To(to) => *to,
                    Continued::Step(Some(step)) => *step,
                    Continued::Step(reserved @ None) => {
                        *reserved.insert(builder.function.reserve_block())
                    }
                    Continued::Passes => unreachable!("passed over above"),
                };
                self.leave_scopes(builder, depth, span);
                builder.end(Terminator::Goto(to));
            }
            Stmt::Goto { span, .. } => {
                let span = *span;
                let labeled = self.resolution.label(id);
                let from = self.point();
                let landing = self.landing(builder, labeled);
                match &landing.at {
                    // Backward: what the label has open is known, so the
                    // markers go in this block.
                    Some(to) => {
                        let (to, block) = (to.clone(), landing.block);
                        self.jump(builder, &from, &to, span, span, diagnostics)?;
                        builder.end(Terminator::Goto(block));
                    }
                    // Forward: what the label has declared is not, so this
                    // ends in a block the label writes when it is reached.
                    None => {
                        let pad = builder.function.reserve_block();
                        landing.waiting.push((pad, from, span));
                        builder.end(Terminator::Goto(pad));
                    }
                }
            }
            Stmt::Labeled { body, .. } => {
                let body = *body;
                self.labeled = true;
                self.land(builder, id, body, diagnostics)?;
            }
            Stmt::Switch {
                condition, body, ..
            } => {
                let (condition, body) = (*condition, *body);
                self.switch(builder, id, condition, body, diagnostics)?;
            }
            // A label of the `switch` around it: a block its dispatch jumps
            // to, the way a `goto` jumps to a label. See `Lowering::switch`.
            Stmt::Case { body, .. } | Stmt::Default { body, .. } => {
                let body = *body;
                self.land(builder, id, body, diagnostics)?;
            }
            Stmt::While {
                condition, body, ..
            } => {
                let (condition, body) = (*condition, *body);
                let header = builder.function.reserve_block();
                builder.end(Terminator::Goto(header));

                // The condition is asked again on every turn, so it is written
                // into the header rather than before it: the back edge at the
                // end of the body arrives here, above the branch.
                builder.switch(header);
                let asked = self.ast.expr(condition).span();
                let decides = self.decides(condition);
                let constant = self.constant_expression(condition);
                let condition = self.value(builder, condition, diagnostics)?;
                let inside = builder.function.reserve_block();
                let after = builder.function.reserve_block();
                builder.end(decided(
                    condition,
                    constant,
                    inside,
                    after,
                    Origin::Written(decides),
                ));

                builder.enter(inside, Some(asked));
                let target = Jumps {
                    broken: after,
                    continued: Continued::To(header),
                    depth: self.scopes.len(),
                };
                self.looped(builder, body, target, diagnostics).0?;
                if builder.reachable() {
                    builder.end(Terminator::Goto(header));
                }

                builder.enter(after, Some(asked));
            }
            Stmt::Do {
                body, condition, ..
            } => {
                let (body, condition) = (*body, *condition);
                let top = builder.function.reserve_block();
                builder.end(Terminator::Goto(top));

                builder.switch(top);
                // Before the body, which a `continue` or a `break` in it
                // reaches.
                let test = builder.function.reserve_block();
                let after = builder.function.reserve_block();
                let target = Jumps {
                    broken: after,
                    continued: Continued::To(test),
                    depth: self.scopes.len(),
                };
                self.looped(builder, body, target, diagnostics).0?;
                if builder.reachable() {
                    builder.end(Terminator::Goto(test));
                }

                // Asked after each turn, in a block of its own that nothing
                // reaches when the body cannot fall through.
                builder.switch(test);
                let asked = self.ast.expr(condition).span();
                let decides = self.decides(condition);
                let constant = self.constant_expression(condition);
                let condition = self.value(builder, condition, diagnostics)?;
                let again = builder.function.reserve_block();
                builder.end(decided(
                    condition,
                    constant,
                    again,
                    after,
                    Origin::Written(decides),
                ));

                // **Not straight back to `top`**: the condition's sequence
                // point belongs on the edge that evaluated it (ADR-0023), and
                // the first turn reaches `top` without evaluating anything.
                // A marker there would end a full expression that has not
                // begun.
                builder.enter(again, Some(asked));
                builder.end(Terminator::Goto(top));

                builder.enter(after, Some(asked));
            }
            Stmt::For {
                start,
                condition,
                step,
                body,
                span,
            } => {
                let (start, condition, step, body, span) =
                    (*start, *condition, *step, *body, *span);
                // C17 6.8.5 p5 scopes a declaration here to the whole loop, so
                // its storage begins before the first turn and ends where the
                // loop is left, once, rather than each turn as the body's
                // does. Not `?`, for the reason `Stmt::Compound` gives: the
                // scope is closed whether or not the loop could be lowered.
                let declares = matches!(start, Some(ForStart::Declaration(_)));
                if declares {
                    self.open_scope();
                }
                let lowered = self.for_loop(builder, start, condition, step, body, diagnostics);
                if declares {
                    let declared = self.scopes.pop().expect("the scope this arm pushed");
                    end_storage(builder, &declared.declared, span);
                }
                lowered?;
            }
            // The driver hands this stage a tree nothing reported about, so a
            // node the parser gave up on cannot be here. Reporting it would be
            // a second diagnostic about the first one's problem, which is what
            // the per-input gate exists to stop.
            Stmt::Error { .. } => {}
        }

        Some(())
    }

    /// A `switch`, C17 6.8.4.2: the controlling value worked out once, the
    /// body, and then the dispatch, a chain of comparisons with each label's
    /// value in the order the labels are written, each going on its own edge
    /// to its label as a `goto` would (ADR-0114). No `case` matching goes to
    /// the `default`, or past the body where there is none (p5).
    ///
    /// A chain of `Branch` and not a terminator of its own, by the owner's
    /// decision on #616: every analysis, the interpreter and the backend read
    /// it already. Its cost is a block per label, which reaches the memory
    /// check's bound sooner.
    ///
    /// The dispatch is written after the body because only then is what each
    /// label has open known; the block before the body ends with a jump to
    /// it. Every comparison is at the promoted type of the controlling
    /// expression (p5), [`Types::controlling`].
    ///
    /// A `null` test written as the controlling expression refines a pointer
    /// only through the first comparison: the nullability check walks back
    /// from a `Branch` across `Goto`s alone, and every later comparison is
    /// reached through one. That is a doubt it could have spared, not one it
    /// misses.
    fn switch(
        &mut self,
        builder: &mut Builder,
        id: StmtId,
        condition: ExprId,
        body: StmtId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let asked = self.ast.expr(condition).span();
        let value = self.value(builder, condition, diagnostics)?;
        // At the promoted controlling type (p5), which each `case` value is
        // converted to as the comparison reads it.
        let controlling = self
            .types
            .controlling(condition)
            .expect("the type check refuses a `switch` on no integer");
        let at = self.ty(asked, controlling, diagnostics)?;
        let held = builder.function.push_local(at);
        builder.push(Operation {
            place: Place::local(held),
            value: Rvalue::Use(value),
            origin: Origin::Generated(asked),
        });
        builder.sequenced(asked);
        let from = self.point();
        let dispatch = builder.function.reserve_block();
        builder.end(Terminator::Goto(dispatch));

        let after = builder.function.reserve_block();
        let target = Jumps {
            broken: after,
            continued: Continued::Passes,
            depth: self.scopes.len(),
        };
        self.looped(builder, body, target, diagnostics).0?;
        if builder.reachable() {
            builder.end(Terminator::Goto(after));
        }

        // **One local for every comparison, and a pad only where the edge
        // has markers to write.** A local and a block per `case` made a
        // `switch` of 250 take ninety seconds in a debug build and one of
        // 260 past the memory check's bound, measured by review: what that
        // check holds grows with blocks times locals squared.
        builder.switch(dispatch);
        let equal = builder.function.push_local(at);
        let mut default = None;
        let mut pads = Vec::new();
        for &label in self.resolution.labels_of_switch(id) {
            let landing = &self.landings[&label];
            let to = landing
                .at
                .clone()
                .expect("every label of a lowered body was reached");
            let (ended, begun) = markers(&from, &to, |local| self.vla_types.contains_key(&local));
            let pad = if ended.is_empty() && begun.is_empty() {
                landing.block
            } else {
                let pad = builder.function.reserve_block();
                pads.push((pad, landing.block, to, label));
                pad
            };
            match self.ast.stmt(label) {
                Stmt::Case { value, .. } => {
                    let number = self
                        .types
                        .value(*value)
                        .expect("the type check refuses a `case` value that is no constant");
                    builder.push(Operation {
                        place: Place::local(equal),
                        value: Rvalue::Binary {
                            op: BinOp::Eq,
                            lhs: Operand::Copy(Place::local(held)),
                            rhs: Operand::Constant(number),
                        },
                        origin: Origin::Generated(self.ast.expr(*value).span()),
                    });
                    let next = builder.function.reserve_block();
                    // **At the `case` value, not the controlling expression**:
                    // what decides this edge is whether the value is this
                    // one, and a report that names the edge, "when this is
                    // true", has to point where that reads true. Found by
                    // review.
                    builder.end(Terminator::Branch {
                        condition: Operand::Copy(Place::local(equal)),
                        then: pad,
                        otherwise: next,
                        origin: Origin::Written(self.ast.expr(*value).span()),
                    });
                    builder.switch(next);
                }
                _ => default = Some(pad),
            }
        }
        builder.end(Terminator::Goto(default.unwrap_or(after)));

        for (pad, block, to, label) in pads {
            builder.switch(pad);
            // The caret on the label alone, `case 1` or `default`, rather
            // than on the statement it labels as well.
            let span = self.ast.stmt(label).span();
            let caret = match self.ast.stmt(label) {
                Stmt::Case { value, .. } => Span::new(
                    span.file(),
                    span.start(),
                    self.ast.expr(*value).span().end(),
                ),
                _ => Span::new(
                    span.file(),
                    span.start(),
                    span.start() + "default".len() as u32,
                ),
            };
            self.jump(builder, &from, &to, span, caret, diagnostics)?;
            builder.end(Terminator::Goto(block));
        }

        builder.switch(after);
        Some(())
    }

    /// A loop's body, lowered with `target` the innermost, and what became of
    /// the target's `continue` block.
    ///
    /// The target is popped whether or not the body could be lowered, for the
    /// reason [`Lowering::targets`] gives, so a caller applies `?` only to
    /// what this returns.
    fn looped(
        &mut self,
        builder: &mut Builder,
        body: StmtId,
        target: Jumps,
        diagnostics: &mut DiagnosticSink,
    ) -> (Option<()>, Continued) {
        self.targets.push(target);
        let lowered = self.stmt(builder, body, diagnostics);
        let target = self.targets.pop().expect("the target this pushed");
        (lowered, target.continued)
    }

    /// A label, a `case` or a `default`, and the statement it labels: the
    /// label's block, reached by falling through and by every jump to it,
    /// with each `goto` that came first written its edge now (ADR-0114).
    ///
    /// **Labels written one after another share one block**, whatever kind
    /// each is. They name one place, and a block apiece made `again: case 1:`
    /// with a `goto again` a loop entered at two blocks, the dispatch's and
    /// the label's, which the memory check does not read. Found by review.
    /// So a label whose statement is another label leaves its block for that
    /// one to take, in [`Lowering::stacked`], and does not open it; a block a
    /// `goto` reserved for the inner one before then is written as a jump to
    /// the shared one, and nothing reaches it.
    fn land(
        &mut self,
        builder: &mut Builder,
        id: StmtId,
        body: StmtId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let shared = self.stacked.take();
        let landing = self.landing(builder, id);
        let mut orphan = None;
        let block = match shared {
            Some(block) => {
                if landing.block != block {
                    orphan = Some(std::mem::replace(&mut landing.block, block));
                }
                block
            }
            None => landing.block,
        };
        if shared.is_none() && builder.reachable() {
            builder.end(Terminator::Goto(block));
        }
        let to = self.point();
        let landing = self.landings.get_mut(&id).expect("reserved above");
        landing.at = Some(to.clone());
        for (pad, from, span) in std::mem::take(&mut landing.waiting) {
            builder.switch(pad);
            self.jump(builder, &from, &to, span, span, diagnostics)?;
            builder.end(Terminator::Goto(block));
        }
        if let Some(orphan) = orphan {
            builder.switch(orphan);
            builder.end(Terminator::Goto(block));
        }
        match self.ast.stmt(body) {
            Stmt::Labeled { .. } | Stmt::Case { .. } | Stmt::Default { .. } => {
                self.stacked = Some(block);
            }
            _ => builder.switch(block),
        }
        self.stmt(builder, body, diagnostics)
    }

    /// Open a scope with an id no other scope of this unit has.
    fn open_scope(&mut self) {
        self.scopes.push(Scope {
            id: self.next_scope,
            declared: Vec::new(),
        });
        self.next_scope += 1;
    }

    /// The scopes open here, with what each has declared so far.
    fn point(&self) -> Point {
        self.scopes
            .iter()
            .map(|scope| (scope.id, scope.declared.clone()))
            .collect()
    }

    /// The landing of `labeled`, its block reserved the first time a `goto`
    /// or the label itself asks.
    fn landing(&mut self, builder: &mut Builder, labeled: StmtId) -> &mut Landing {
        self.landings.entry(labeled).or_insert_with(|| Landing {
            block: builder.function.reserve_block(),
            at: None,
            waiting: Vec::new(),
        })
    }

    /// The storage markers of a jump from `from` to `to`, written into the
    /// block being built, which the caller then ends with the jump. See
    /// ADR-0114.
    ///
    /// The scopes both have open, matched by id, are the ones the jump stays
    /// in. It ends every local of a scope only `from` has, innermost first in
    /// reverse order of declaration, as a closing brace would. It begins what
    /// the deepest of the shared scopes has declared at `to` and had not at
    /// `from`, which a forward jump skips, and every local of a scope only
    /// `to` has, outermost first in order of declaration, which a jump into a
    /// block skips. A backward jump within one scope begins nothing: what it
    /// passes is alive already, and a declaration it reaches again is what
    /// makes the value indeterminate.
    ///
    /// **A jump that would begin a variable length array is refused**
    /// (#1209): it passes the declaration that works the length out, so the
    /// storage it begins would be sized by a local nothing wrote, and C17
    /// 6.8.6.1 p1 and 6.8.4.2 p2 make it a constraint violation. Mutation:
    /// report nothing; `a_jump_into_the_scope_of_a_variable_length_array_is_refused`
    /// is accepted. `caret` is where that is said, the jump or the label a
    /// `switch` jumps to, and `span` what the markers are generated by.
    fn jump(
        &self,
        builder: &mut Builder,
        from: &Point,
        to: &Point,
        span: Span,
        caret: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let (ended, begun) = markers(from, to, |local| self.vla_types.contains_key(&local));
        // A typedef's length local is a variably modified identifier's
        // too, and is not ended on a jump back: it holds no storage.
        let typedef_at = |local: LocalId| {
            self.typedef_locals
                .iter()
                .find(|&&(held, _)| held == local)
                .map(|&(_, at)| at)
        };
        if let Some(&skipped) = begun
            .iter()
            .find(|&&local| self.vla_types.contains_key(&local) || typedef_at(local).is_some())
        {
            let mut diagnostic = Diagnostic::error(
                "this jumps into the scope of a variable length array",
            )
            .with_code(INTO_VARIABLE_SCOPE)
            .with_label(Label::primary(caret, "this jumps past its declaration"))
            .with_note("C17 6.8.6.1 p1 and 6.8.4.2 p2: no jump may enter the scope of a variable length array from outside it");
            if let Some(name) = builder.function.name_of(skipped).or(typedef_at(skipped)) {
                diagnostic =
                    diagnostic.with_label(Label::secondary(name, "its length is worked out here"));
            }
            diagnostics.report(diagnostic);
            return None;
        }
        for local in ended {
            builder.element(Element::StorageDead {
                origin: Origin::Generated(span),
                local,
            });
        }
        for local in begun {
            builder.element(Element::StorageLive {
                local,
                origin: Origin::Generated(span),
            });
        }
        Some(())
    }

    /// End the storage of every local in a scope deeper than `depth`, where
    /// control leaves them all at once: a `return`, a `break` or a
    /// `continue`.
    ///
    /// Innermost first, in the order the `Compound` arm closes one, and
    /// `Generated` at `span`, the statement that leaves: control never
    /// reaches those braces, and this statement is why the storage ends. The
    /// body of the function is the first scope and holds only its variable
    /// length arrays (#1209), which a `return` ends like any other, so the
    /// depth of 0 a `return` passes needs no test. See ADR-0012.
    fn leave_scopes(&self, builder: &mut Builder, depth: usize, span: Span) {
        for scope in self.scopes[depth..].iter().rev() {
            for &local in scope.declared.iter().rev() {
                builder.element(Element::StorageDead {
                    origin: Origin::Generated(span),
                    local,
                });
            }
        }
    }

    /// The blocks of a `for` from its first clause to where it is left, with
    /// `builder` positioned in that last block. C17 6.8.5.3.
    ///
    /// A declaration start is lowered as the statement it is, inside the scope
    /// [`Lowering::stmt`] opened for it, which is what gives its locals their
    /// `StorageLive`.
    fn for_loop(
        &mut self,
        builder: &mut Builder,
        start: Option<ForStart>,
        condition: Option<ExprId>,
        step: Option<ExprId>,
        body: StmtId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        match start {
            Some(ForStart::Expression(initialiser)) => {
                self.effect(builder, initialiser, diagnostics)?;
                builder.sequenced(self.ast.expr(initialiser).span());
            }
            Some(ForStart::Declaration(declaration)) => {
                self.stmt(builder, declaration, diagnostics)?;
            }
            None => {}
        }

        let header = builder.function.reserve_block();
        builder.end(Terminator::Goto(header));
        builder.switch(header);

        let inside = builder.function.reserve_block();
        let after = builder.function.reserve_block();
        // Read before the match so that both blocks below can ask for
        // it: an absent condition is an absent sequence point, which
        // is what `Builder::enter` answers `None` for.
        let asked = condition.map(|condition| self.ast.expr(condition).span());
        match condition {
            Some(condition) => {
                let decides = self.decides(condition);
                let constant = self.constant_expression(condition);
                let condition = self.value(builder, condition, diagnostics)?;
                builder.end(decided(
                    condition,
                    constant,
                    inside,
                    after,
                    Origin::Written(decides),
                ));
            }
            // 6.8.5.3 p2: an absent condition is replaced by a non-zero
            // constant, so the loop has no exit edge of its own.
            None => builder.end(Terminator::Goto(inside)),
        }

        builder.enter(inside, asked);
        let target = Jumps {
            broken: after,
            continued: Continued::Step(None),
            depth: self.scopes.len(),
        };
        let (lowered, continued) = self.looped(builder, body, target, diagnostics);
        lowered?;
        // The step runs after a turn that fell through and after one a
        // `continue` ended (C17 6.8.5.3 p1, 6.8.6.2 p2). Where a `continue`
        // reserved a block for it, both arrive there; where none did, it is
        // written where the body ended, so a loop with no `continue` lowers
        // to what it did before there was one.
        let reserved = match continued {
            Continued::Step(reserved) => reserved,
            Continued::To(_) | Continued::Passes => {
                unreachable!("a `for` continues to its step")
            }
        };
        if let Some(stepped) = reserved {
            if builder.reachable() {
                builder.end(Terminator::Goto(stepped));
            }
            builder.switch(stepped);
        }
        if builder.reachable() {
            if let Some(step) = step {
                self.effect(builder, step, diagnostics)?;
                builder.sequenced(self.ast.expr(step).span());
            }
        }
        if builder.reachable() {
            builder.end(Terminator::Goto(header));
        }

        builder.enter(after, asked);
        Some(())
    }

    /// The value of a full expression that has one.
    ///
    /// Every caller reads what this answers, and C17 6.3.2.2 p1 forbids using
    /// the "(nonexistent) value of a void expression", which `types.rs`
    /// refuses and the driver does not lower past. So a `void` root here is a
    /// gap in that gate, and the `expect` is what names it.
    fn value(
        &mut self,
        builder: &mut Builder,
        root: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Operand> {
        Some(
            self.walk(builder, root, true, diagnostics)?
                .expect("a value for the root, which is not `void`"),
        )
    }

    /// A full expression evaluated for its effects, whose value is discarded.
    ///
    /// An expression statement and a `for`'s first and third clauses. A value
    /// it has is handed to [`Builder::discarded`]; a `void` one has none.
    fn effect(
        &mut self,
        builder: &mut Builder,
        root: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        self.discarding = Some(root);
        let walked = self.walk(builder, root, true, diagnostics);
        self.discarding = None;
        if let Some(value) = walked? {
            builder.discarded(value, self.ast.expr(root).span());
        }
        Some(())
    }

    /// The value of an expression, with an explicit stack rather than a
    /// recursion: `None` where it could not be lowered, and `Some(None)` where
    /// it is `void` and so has no value to answer.
    ///
    /// See [`Task`] for why. Values and places come back on two stacks, and a
    /// node pops exactly what it reads, so the stacks are empty of its operands
    /// by the time it pushes its own. A call to a `void` function and a `void`
    /// `?:` push nothing, and the three places either can reach, an expression
    /// statement, a comma's left operand and a `?:`'s arms, ask
    /// [`Lowering::pushes`] rather than the type: `*p` of a `void *` is
    /// `void` too, and pushes the place it reads. A consumer that pops where
    /// nothing was pushed panics naming the stack it expected, and a value
    /// nobody popped panics here, rather than a read going missing.
    ///
    /// `full` is whether `root` is a full expression. Every caller but one
    /// passes `true`: Annex C's list and `lower_stmt`'s arms are the same
    /// eight places. [`Lowering::lengths`] passes `false`, because a
    /// parameter's length
    /// is part of a declarator (C17 6.8 p4) and unsequenced against the
    /// others, and the walk then starts as it would under an operator C
    /// leaves unsequenced: no sequence point inside is recorded, and no call
    /// inside ends its block for good.
    fn walk(
        &mut self,
        builder: &mut Builder,
        root: ExprId,
        full: bool,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Option<Operand>> {
        let (value, _) = self.walk_from(builder, root, full, Task::Value(root), diagnostics)?;
        Some(value)
    }

    /// The place a full expression names, for what reads a struct whole: a
    /// struct copied, which is copied from where it is rather than read into
    /// a value (#1201). [`Lowering::walk`], started from the place.
    fn place_of(
        &mut self,
        builder: &mut Builder,
        root: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Place> {
        let (_, place) = self.walk_from(builder, root, true, Task::Place(root), diagnostics)?;
        Some(place.expect("a place for a root asked for one"))
    }

    /// [`Lowering::walk`] and [`Lowering::place_of`], from `start`.
    fn walk_from(
        &mut self,
        builder: &mut Builder,
        root: ExprId,
        full: bool,
        start: Task,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<(Option<Operand>, Option<Place>)> {
        // Set here rather than restored, so that an expression abandoned
        // half-way by a type error cannot leave the next one reading a flag
        // the walk that set it never finished with.
        self.top_level = full;
        self.root = full.then_some(root);

        let mut tasks = vec![start];
        let mut values: Vec<Operand> = Vec::new();
        let mut places: Vec<Place> = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Value(id) => {
                    self.begin_value(id, &mut tasks, &mut values, diagnostics)?;
                }
                Task::Place(id) => {
                    self.begin_place(builder, id, &mut tasks, &mut places, diagnostics)?;
                }
                Task::Finish(id) => {
                    self.finish_value(builder, id, &mut values, &mut places, diagnostics)?;
                }
                Task::FinishPlace(id) => {
                    self.finish_place(builder, id, &mut values, &mut places, diagnostics)?;
                }
                Task::Split(id) => {
                    self.split(builder, id, &mut tasks, &mut values, diagnostics)?;
                }
                Task::Discard(id) => self.discard(builder, id, &mut values),
                Task::Second(id) => self.second(builder, id, &mut tasks, &mut values),
                Task::Merge(id) => self.merge(builder, id, &mut values),
                Task::Arguments(span) => builder.element(Element::ArgumentsEvaluated {
                    origin: Origin::Generated(span),
                }),
                Task::Conflicts(id) => {
                    for element in self.unsequenced(&builder.function, id) {
                        builder.element(element);
                    }
                }
                Task::Restore(top_level) => self.top_level = top_level,
                Task::Copied(id) => {
                    let from = places.pop().expect("a struct argument");
                    let span = self.ast.expr(id).span();
                    let ty = self.ty_of(id, diagnostics)?;
                    let copy = builder.function.push_local(ty);
                    // **The copy is the callee's parameter, and lives only
                    // until the call returns** (C17 6.2.4 p2, 6.9.1 p9): its
                    // storage begins here and the call ends it, so a pointer
                    // into it kept past the call is one into storage that
                    // has ended, as it is to C.
                    builder.element(Element::StorageLive {
                        local: copy,
                        origin: Origin::Generated(span),
                    });
                    self.argument_copies.push(copy);
                    self.copy_struct(builder, Place::local(copy), from, span)?;
                    let pointer = self.unit.push_type(Ty::Pointer(ty));
                    let address = builder.function.push_local(pointer);
                    builder.push(Operation {
                        place: Place::local(address),
                        value: Rvalue::Address(Place::local(copy)),
                        origin: Origin::Generated(span),
                    });
                    values.push(Operand::Copy(Place::local(address)));
                }
                Task::Placed(id) => {
                    let Some(Operand::Copy(place)) = values.pop() else {
                        unreachable!(
                            "a struct a call returns is the place it was returned into: {id:?}"
                        );
                    };
                    places.push(place);
                }
            }
        }

        let value = values.pop();
        let place = places.pop();
        assert!(
            values.is_empty() && places.is_empty(),
            "every value and place an expression pushed was read: {values:?} {places:?}"
        );
        Some((value, place))
    }

    /// Whether lowering `id` leaves a value on the walk's stack.
    ///
    /// A call to a `void` function, a `void` `?:` and a cast to `void` leave
    /// none, and a comma
    /// leaves what its right operand leaves. An assignment whose value
    /// [`Lowering::effect`] discards leaves none (#1137). Everything else leaves one,
    /// `void` included: `*p` of a `void *` is a read of a place, and a
    /// consumer that dropped it for its type would drop the read, which is
    /// what the memory and nullability checks ask about.
    fn pushes(&self, mut id: ExprId) -> bool {
        // A loop rather than a recursion, because a comma's right operand
        // can be a comma as deep as the parser lets parentheses go.
        loop {
            match self.ast.expr(id) {
                Expr::Call { .. } | Expr::Conditional { .. } | Expr::Cast { .. } => {
                    return !self.is_void(id);
                }
                // **No value, as a call to a function returning `void` has
                // none** (#1214). Mutation: answer `true`;
                // `a_va_start_or_va_end_is_a_void_expression_anywhere` panics.
                Expr::VaOp { .. } => return false,
                Expr::Comma { rhs, .. } => id = *rhs,
                _ => return true,
            }
        }
    }

    /// Where the expression whose value `id` is was written: for a comma, its
    /// right operand, followed down the chain, and otherwise `id` itself.
    ///
    /// C17 6.5.17 p2 evaluates a comma's left operand as a void expression and
    /// gives the whole the right operand's value, so in `if (c, *p)` only `*p`
    /// decides and only `*p` is underlined. A loop for [`Self::pushes`]'s
    /// reason. What a branch or a recorded `&&` operand points at is this;
    /// where the sequence point falls is still the whole expression's.
    fn decides(&self, mut id: ExprId) -> Span {
        while let Expr::Comma { rhs, .. } = self.ast.expr(id) {
            id = *rhs;
        }
        self.ast.expr(id).span()
    }

    /// Whether `id` has type `void`, and so no value: C17 6.3.2.2 p1.
    ///
    /// An expression nothing typed answers `false`, and is refused where its
    /// type is asked for rather than here.
    fn is_void(&self, id: ExprId) -> bool {
        self.types
            .of(id)
            .is_some_and(|ty| matches!(self.ast.ty(ty), Type::Void))
    }

    /// Step below a node C leaves unsequenced, if this is one.
    ///
    /// A sequence point below such a node orders that node's own parts and
    /// nothing beside them, so [`Lowering::top_level`] goes false for its
    /// operands. The task is pushed before the arm's own, so it is popped
    /// after every one of them: that is what makes this cover the operands
    /// rather than whatever follows the node.
    ///
    /// **Two callers, and one of them cannot reach it with the flag set.**
    /// Every task that asks for a place is pushed by a node that does not
    /// sequence: `&`, `++`, `--`, an assignment, a subscript, a dereference.
    /// Each of those has already run this and set the flag false, so a place
    /// walk is always entered below one. `begin_place` does meet a comma or a
    /// conditional, through `&(a, b)` and its like, and refuses them as naming
    /// no place; it is not that they never arrive. Measured: deleting the call
    /// from there breaks no named test.
    ///
    /// It is called anyway, because one rule asked in two places is one
    /// function rather than two copies, and two copies of one rule have drifted
    /// apart here before. A C++ adapter with a sequencing
    /// operator that yields an lvalue makes the call live without anybody
    /// having to notice.
    ///
    /// **Not at every such node: only where two of its operands may act**
    /// ([`Self::at_most_one_acts`]). What a marker below one operand could
    /// misorder is something another operand puts in the block, and a
    /// constant or a local read by value puts nothing there. So `x = (free(p),
    /// *p);` keeps the comma's marker and is the proof `int x = (free(p),
    /// *p);` is, while `*r + ((free(p), 1) && *p)` drops it (#178). See
    /// ADR-0022.
    fn descend(&mut self, id: ExprId, tasks: &mut Vec<Task>) {
        if self.top_level && !sequences(self.ast.expr(id)) && !self.at_most_one_acts(id) {
            tasks.push(Task::Restore(true));
            self.top_level = false;
        }
    }

    /// Whether at most one operand of this node may act: read through a
    /// pointer, call, or write.
    ///
    /// That operand is the only one that can hold a marker or anything a
    /// marker orders, so the node keeps [`Lowering::top_level`] for it, and
    /// the flag reaches a marker only through nodes that each passed this.
    /// **The direction to be wrong in is counting too many**: an operand
    /// answered inert that does act is a read the marker orders before a free
    /// C leaves it unordered against, which is a silence. See ADR-0022.
    fn at_most_one_acts(&self, id: ExprId) -> bool {
        let acting = match self.ast.expr(id) {
            // **A compound assignment reads its place**, C17 6.5.16.2 p3, so
            // `*p += e` reads through `p` unsequenced against `e`, where `*p = e`
            // only stores there after both. Counted as the plain store, `*p +=
            // (exit(1), 0);` kept the comma's marker ahead of that read, and a
            // check that ends a path at the marker after `exit` was silent
            // about it (#590).
            Expr::Assign {
                op, place, value, ..
            } => {
                usize::from(op.is_some() || !self.inert_place(*place))
                    + usize::from(!self.inert(*value))
            }
            Expr::Binary { lhs, rhs, .. } => {
                usize::from(!self.inert(*lhs)) + usize::from(!self.inert(*rhs))
            }
            Expr::Subscript { base, index, .. } => {
                usize::from(!self.inert(*base)) + usize::from(!self.inert(*index))
            }
            Expr::Call {
                callee, arguments, ..
            } => {
                usize::from(!self.inert(*callee))
                    + arguments
                        .iter()
                        .filter(|&&argument| !self.inert(argument))
                        .count()
            }
            // One operand, so nothing beside it.
            Expr::Unary { .. } | Expr::Cast { .. } | Expr::VaOp { .. } | Expr::VaArg { .. } => 1,
            // Written out rather than `_`, so a kind added later is asked.
            // These sequence, have no operands, or are refused.
            Expr::Number { .. }
            | Expr::Enumerator { .. }
            | Expr::Character { .. }
            | Expr::Length { .. }
            | Expr::String { .. }
            | Expr::Identifier { .. }
            | Expr::Member { .. }
            | Expr::Conditional { .. }
            | Expr::Comma { .. }
            | Expr::SizeofType { .. }
            | Expr::SizeofExpr { .. }
            | Expr::Error { .. } => 2,
        };
        acting <= 1
    }

    /// Whether evaluating this puts nothing in the block a marker could
    /// misorder: constants, locals read by value, `&` of a local, and
    /// arithmetic over those.
    ///
    /// A local read by value is not a read through a pointer, which is all the
    /// memory check carries forwards to a free, and its value changes only by
    /// a write, which is not inert. A dereference, a subscript, a call, an
    /// assignment, an increment and a member are none of these.
    ///
    /// **That last argument has a limit**: the operand that does act may be
    /// the write. `*q = (free(q), q = r, 0)` reads `q` for the place and writes
    /// it in the value with nothing ordering the two, which C17 6.5 p2 makes
    /// undefined. This argument does not answer that; [`Self::unsequenced`]
    /// does, by reporting the conflict as `SC0409` (#525, ADR-0052).
    fn inert(&self, id: ExprId) -> bool {
        match self.ast.expr(id) {
            Expr::Number { .. }
            | Expr::Enumerator { .. }
            | Expr::Character { .. }
            | Expr::Length { .. }
            | Expr::String { .. }
            | Expr::Identifier { .. } => true,
            Expr::Unary { op, operand, .. } => match op {
                AstUnOp::AddrOf => matches!(self.ast.expr(*operand), Expr::Identifier { .. }),
                AstUnOp::Plus | AstUnOp::Minus | AstUnOp::Not | AstUnOp::BitNot => {
                    self.inert(*operand)
                }
                AstUnOp::Deref
                | AstUnOp::PreInc
                | AstUnOp::PreDec
                | AstUnOp::PostInc
                | AstUnOp::PostDec => false,
            },
            Expr::Binary { lhs, rhs, .. } | Expr::Comma { lhs, rhs, .. } => {
                self.inert(*lhs) && self.inert(*rhs)
            }
            Expr::Conditional {
                condition,
                then,
                otherwise,
                ..
            } => self.inert(*condition) && self.inert(*then) && self.inert(*otherwise),
            Expr::Cast { operand, .. } => self.inert(*operand),
            // Its operand is not evaluated (C17 6.5.3.4 p2).
            Expr::SizeofType { .. } | Expr::SizeofExpr { .. } => true,
            // Each writes a list.
            Expr::Assign { .. }
            | Expr::Call { .. }
            | Expr::Subscript { .. }
            | Expr::Member { .. }
            | Expr::VaOp { .. }
            | Expr::VaArg { .. }
            | Expr::Error { .. } => false,
        }
    }

    /// Whether the place an assignment writes is computed without acting: a
    /// local, or `*` or `[]` over inert operands.
    ///
    /// Computing that place reads only those operands. The store itself is
    /// sequenced after the value computations of both operands (C17 6.5.16
    /// p3), so it follows the right operand's markers in C as in the block.
    /// Where the right operand writes a local the place reads, the place may
    /// be computed from either value, which is not answered here and is
    /// reported as a conflict instead ([`Self::unsequenced`]).
    fn inert_place(&self, id: ExprId) -> bool {
        match self.ast.expr(id) {
            Expr::Identifier { .. } => true,
            Expr::Unary {
                op: AstUnOp::Deref,
                operand,
                ..
            } => self.inert(*operand),
            Expr::Subscript { base, index, .. } => self.inert(*base) && self.inert(*index),
            _ => false,
        }
    }

    /// Push what an expression needs before its value can be built.
    fn begin_value(
        &mut self,
        id: ExprId,
        tasks: &mut Vec<Task>,
        values: &mut Vec<Operand>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        self.typed(id, diagnostics)?;

        // A constant expression is the constant it evaluates to, C17 6.6 p2,
        // and `types.rs` has that value only where every operand is a
        // constant too. Not the folding of `E + 0` over a non-constant that
        // ADR-0021 declines: nothing here is read or written at run time.
        // Before the descent, so none of its operands is lowered.
        // A literal is left to its own arm below, which says why its value
        // is asked there; it would give the same operand here.
        let folded = match self.ast.expr(id) {
            Expr::Number { .. } | Expr::Character { .. } | Expr::Length { .. } => None,
            _ => self.types.value(id),
        };
        if let Some(value) = folded {
            values.push(Operand::Constant(value));
            return Some(());
        }

        // Before [`Lowering::descend`], which is what takes the flag away for
        // this node's own operands. What the call arm below asks is whether
        // the flag reaches **the call**, and asking after the descent answers
        // about its arguments instead.
        let at_root = self.top_level;
        self.descend(id, tasks);

        // **A function designator is its address** (C17 6.3.2.1 p4), `*` of
        // a function pointer is a designator again and `&` of a designator
        // its address (6.5.3.2 p3 and p4), so each of these is the value of
        // what it applies to and reads no place (#1212). See ADR-0125.
        // Mutation: give a designator no value of its own;
        // `a_function_pointer_is_stored_called_and_compared` in
        // `tests/run.rs` does not build.
        if let Some(function) = self.designated(id) {
            if matches!(self.ast.expr(id), Expr::Identifier { .. }) {
                values.push(Operand::Function(function));
                return Some(());
            }
        }
        match self.ast.expr(id) {
            Expr::Unary {
                op: AstUnOp::Deref,
                operand,
                ..
            } if self.is_function(id) => {
                tasks.push(Task::Value(*operand));
                return Some(());
            }
            Expr::Unary {
                op: AstUnOp::AddrOf,
                operand,
                ..
            } if self.is_function(*operand) => {
                tasks.push(Task::Value(*operand));
                return Some(());
            }
            _ => {}
        }

        match self.ast.expr(id) {
            Expr::Number { .. }
            | Expr::Enumerator { .. }
            | Expr::Character { .. }
            | Expr::Length { .. } => {
                // Worked out by `types.rs`, where the base, the suffix and
                // the range of `int` are all one question. A constant it
                // could not read has no value and has already been reported,
                // which is why nothing is said here.
                //
                // **The `?` cannot fire today**, and it stays anyway. A
                // constant has no value exactly when it has no type, and
                // `typed` above has already returned `None` for that, so
                // replacing this with `unwrap_or(0)` breaks no test: measured.
                // That is a line a stronger rule upstream
                // answers for, and what it guards against is the day `typed`
                // narrows. Writing the fallback instead would put a number
                // nobody wrote into the IR, which is how `010` used to lower
                // to ten.
                values.push(Operand::Constant(self.types.value(id)?));
            }
            Expr::Identifier { .. }
            | Expr::Subscript { .. }
            | Expr::String { .. }
            | Expr::Member { .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Place(id));
            }
            // **The lists' places, and nothing else**: `__builtin_va_start`'s
            // last parameter only says where the arguments begin, and is not
            // evaluated (#1214).
            Expr::VaOp { op, operands, .. } => {
                tasks.push(Task::Finish(id));
                let lists = match op {
                    VaOp::Start | VaOp::End => &operands[..1],
                    VaOp::Copy => &operands[..],
                };
                for &list in lists.iter().rev() {
                    tasks.push(Task::Place(list));
                }
            }
            Expr::VaArg { list, .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Place(*list));
            }
            Expr::Unary { op, operand, .. } => {
                tasks.push(Task::Finish(id));
                match op {
                    // C17 6.5.3.2 p4 makes the operand of `*` a value and the
                    // result an lvalue, so `*(p + i)` and `*p++` are ordinary
                    // C. Asking for the operand's place instead would refuse
                    // both: what has a place here is `*p`, not `p + i`.
                    AstUnOp::Deref => tasks.push(Task::Place(id)),
                    // These read a place, and `&x` never reads `x` at all.
                    AstUnOp::AddrOf
                    | AstUnOp::PreInc
                    | AstUnOp::PreDec
                    | AstUnOp::PostInc
                    | AstUnOp::PostDec => tasks.push(Task::Place(*operand)),
                    AstUnOp::Plus | AstUnOp::Minus | AstUnOp::Not | AstUnOp::BitNot => {
                        tasks.push(Task::Value(*operand));
                    }
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                // `&&` and `||` are control flow: C17 6.5.13 p4 and 6.5.14 p4
                // say the right operand is not evaluated unless the left says
                // to, and `ir::BinOp` has no variant to lower them to.
                if matches!(op, AstBinOp::LogAnd | AstBinOp::LogOr) {
                    tasks.push(Task::Split(id));
                    tasks.push(Task::Value(*lhs));
                } else {
                    tasks.push(Task::Finish(id));
                    tasks.push(Task::Conflicts(id));
                    tasks.push(Task::Value(*rhs));
                    tasks.push(Task::Value(*lhs));
                }
            }
            // **A struct is copied from where it is**, its value never read
            // whole (#1201): the right operand is asked for its place, and
            // `finish_value` copies one place into the other.
            Expr::Assign {
                op: None,
                place,
                value,
                ..
            } if self.is_struct(*place) => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Conflicts(id));
                tasks.push(Task::Place(*value));
                tasks.push(Task::Place(*place));
            }
            Expr::Assign { place, value, .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Conflicts(id));
                tasks.push(Task::Value(*value));
                tasks.push(Task::Place(*place));
            }
            Expr::Conditional { condition, .. } => {
                tasks.push(Task::Split(id));
                tasks.push(Task::Value(*condition));
            }
            Expr::Call {
                callee,
                arguments,
                span,
            } => {
                let (callee, span) = (*callee, *span);
                tasks.push(Task::Finish(id));
                // C17 6.5.2.2 p10, first sentence: a sequence point after the
                // arguments and before the call. Popped after every argument
                // and before the `Finish` that builds the terminator, which is
                // where C puts it.
                //
                // **Only where the flag reaches the call.** The marker is a
                // claim about the whole block, and a block holds the other
                // operands of whatever encloses this: emitting one for `*p +
                // (free(p), 0)` would order the read against a free C leaves
                // it unordered against. That is ADR-0022's enclosure rule,
                // asked about the call rather than about the point, and it is
                // the same rule the four operators already ask, including
                // that a node whose other operands cannot act keeps it. See
                // ADR-0026.
                //
                // Mutation: emit it whatever encloses the call. Four cases
                // lose their `SC0402` outright, measured:
                // `an_unsequenced_use_the_check_meets_first_is_reported`,
                // `a_write_through_a_pointer_the_check_meets_first_is_reported`,
                // `a_discarded_read_the_check_meets_first_is_reported` and
                // `a_condition_read_through_a_pointer_in_an_unsequenced_operand_is_reported`.
                if at_root {
                    tasks.push(Task::Arguments(span));
                }
                tasks.push(Task::Conflicts(id));
                for &argument in arguments.iter().rev() {
                    // **A struct argument is the address of a copy of it**
                    // (#1201): its place, then the copy. See
                    // `Passes::StructCopy`.
                    if self.is_struct(argument) {
                        tasks.push(Task::Copied(argument));
                        tasks.push(Task::Place(argument));
                    } else {
                        tasks.push(Task::Value(argument));
                    }
                }
                // **A pointer called through is lowered first**, a value
                // below the arguments that the call takes back off when it
                // finishes (#1212). A pointer in a place is that place, read
                // where the call reads it, after the arguments, as an operand
                // of `g + f()` is read after `f()`: C17 6.5.2.2 p10 leaves the
                // order of the callee and the arguments unspecified, so
                // either is one C allows.
                if self.designated(callee).is_none() && !self.names_a_refused_function(callee) {
                    tasks.push(Task::Value(callee));
                }
            }
            Expr::Comma { lhs, rhs, .. } => {
                // 6.5.17 p2: the left is evaluated as a void expression, so its
                // value is built and dropped rather than not built. Dropping it
                // is [`Task::Discard`]'s, between the two, and there is nothing
                // left for a `Finish` to do afterwards: the right operand's
                // value is the comma's and is already where it belongs.
                tasks.push(Task::Value(*rhs));
                tasks.push(Task::Discard(id));
                tasks.push(Task::Value(*lhs));
            }
            Expr::Cast { operand, .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Value(*operand));
            }
            // A `sizeof` with a value had it pushed by the fold above, and its
            // operand is never evaluated (C17 6.5.3.4 p2). One that reaches
            // here has none: it is of a variable length array, worked out
            // when the node finishes (#1209).
            Expr::SizeofType { .. } | Expr::SizeofExpr { .. } => tasks.push(Task::Finish(id)),
            // The parser reported whatever made this, and the driver's gate
            // means it never reaches here. Reporting it again would be a second
            // diagnostic about the first one's problem, so the function is
            // abandoned without one.
            Expr::Error { .. } => return None,
        }

        Some(())
    }

    /// Push what an expression needs before the place it names can be built.
    fn begin_place(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        tasks: &mut Vec<Task>,
        places: &mut Vec<Place>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        self.typed(id, diagnostics)?;

        self.descend(id, tasks);

        match self.ast.expr(id) {
            // `p->m` needs the pointer's value and `s.m` the struct's place,
            // and both end as a member's address, `&(*p).m`, and a place
            // through it. See `Self::member`.
            Expr::Member {
                base, arrow: true, ..
            } => {
                tasks.push(Task::FinishPlace(id));
                tasks.push(Task::Value(*base));
            }
            Expr::Member {
                base, arrow: false, ..
            } => {
                tasks.push(Task::FinishPlace(id));
                tasks.push(Task::Place(*base));
            }
            Expr::Identifier { span, .. } => match self.static_named(id) {
                Some(object) => places.push(self.reach_static(builder, object, *span)),
                None => places.push(Place::local(self.local(id, diagnostics)?)),
            },
            // The array a literal is, through its address as any static is.
            Expr::String { span, .. } => {
                let object = self.string(id);
                places.push(self.reach_static(builder, object, *span));
            }
            Expr::Unary {
                op: AstUnOp::Deref,
                operand,
                ..
            } => {
                // **`*&E` is `E`.** C17 6.5.3.2 p3 makes `&E` the address of
                // what `E` designates and p4 makes `*` of that address the
                // object itself, and the footnote to p4 says it outright: where
                // `E` is an lvalue that is a valid operand of `&`, `*&E` is an
                // lvalue equal to `E`. It is the other half of the rule the `AddrOf` arm of
                // `finish_value` applies to `&*E`.
                //
                // Without this, the value went through a temporary holding an
                // address, and what a read of that is to the memory check is a
                // pointer read out of memory, which holds no site: a freed
                // pointer returned or dereferenced as `*&p` was answered in
                // silence where `p` is refused.
                //
                // **On the tree rather than on the IR**, unlike `&*E`. That one
                // is a place whose last step is a `Deref`, which the IR still
                // shows when the `&` arrives. This one would be an `Address`
                // written into a temporary, found and taken back out after it
                // was pushed. Asking for the place of `&`'s own operand builds
                // nothing to take back, and it is exactly what `&` would have
                // asked for, so `*&3` is refused as `&3` is. The `&` node is
                // not passed to `descend`, which costs nothing: `&` sequences
                // nothing, and the `*` above it has already been passed.
                if let Expr::Unary {
                    op: AstUnOp::AddrOf,
                    operand: addressed,
                    ..
                } = self.ast.expr(*operand)
                {
                    tasks.push(Task::Place(*addressed));
                } else {
                    tasks.push(Task::FinishPlace(id));
                    tasks.push(Task::Value(*operand));
                }
            }
            Expr::Subscript { base, index, .. } => {
                tasks.push(Task::FinishPlace(id));
                tasks.push(Task::Conflicts(id));
                tasks.push(Task::Value(*index));
                tasks.push(Task::Value(*base));
            }
            // **A call returning a struct names the temporary it returned
            // into** (#1201): its value is that place, taken as one.
            Expr::Call { .. } if self.is_struct(id) => {
                tasks.push(Task::Placed(id));
                tasks.push(Task::Value(id));
            }
            // Everything else has a value and no place. An assignment, `&`,
            // `++` and `--` each ask for one, so the message says that rather
            // than naming assignment: `----n` asks through the innermost `--`
            // and nothing in it is being assigned to. C17 6.5.16 p2 wants a
            // modifiable lvalue on the left of an assignment, and whether a
            // program breaks that constraint is the type checker's to say; what
            // is said here is only that there is nothing to read or write.
            //
            // The kinds are written out rather than wildcarded, so an
            // expression kind added later is `error[E0004]` here and has to say
            // whether it names a place.
            Expr::Number { .. }
            | Expr::Enumerator { .. }
            | Expr::Character { .. }
            | Expr::Length { .. }
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Assign { .. }
            | Expr::Conditional { .. }
            | Expr::Call { .. }
            | Expr::Comma { .. }
            | Expr::SizeofType { .. }
            | Expr::SizeofExpr { .. }
            | Expr::Cast { .. }
            | Expr::VaOp { .. }
            | Expr::VaArg { .. }
            | Expr::Error { .. } => {
                // **A struct value that is not an object is not lowered yet**,
                // and is not the program's fault: a copy, an argument and a
                // returned struct ask for the place of whatever they copy
                // (#1201), and C17 6.5.16.1 p1 lets the right of `a = b = c`
                // or `a = k ? b : c` be any struct value, which names none
                // here. Mutation: drop this; `a_struct_value_that_is_no_object_is_not_lowered_yet`
                // says the program names no place.
                if self.is_struct(id) {
                    self.struct_value_not_yet(id, self.ast.expr(id).span(), diagnostics);
                    return None;
                }
                diagnostics.report(
                    Diagnostic::error("this expression names no place")
                        .with_code(LOWERING)
                        .with_label(Label::primary(
                            self.ast.expr(id).span(),
                            "this is a value, not somewhere a value can live",
                        ))
                        .with_note(
                            "an assignment, `&`, `++` and `--` each need a place to work on",
                        ),
                );
                return None;
            }
        }

        Some(())
    }

    /// The type a value of type `ty` has once [`Self::decayed`] has converted
    /// it: a pointer to the first element for an array, and `ty` otherwise.
    fn decayed_ty(&mut self, ty: TyId) -> TyId {
        match self.unit.ty(ty) {
            Ty::Array { element, .. } => self.unit.push_type(Ty::Pointer(element)),
            // And a pointer to a function for a function (C17 6.3.2.1 p4).
            Ty::Function(_) => self.unit.push_type(Ty::Pointer(ty)),
            Ty::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            }
            | Ty::Void
            | Ty::Pointer(_)
            | Ty::Struct(_) => ty,
        }
    }

    /// The value of a place used as one: what it holds, or, where it is an
    /// array, a pointer to its first element.
    ///
    /// **C17 6.3.2.1 p3**: an expression of array type that is not the operand
    /// of `&` or `sizeof` is converted to a pointer to the array's first
    /// element. `&` takes its operand's place and never reaches here, and
    /// `sizeof`'s operand is never lowered. Two shapes, by where the array is:
    ///
    /// - **A local**, `a`: `Address(a[0])`, an `Index` of a constant zero
    ///   under `Rvalue::Address`, which the memory check reads as the local's
    ///   own start and as a may-write of it, since one store through it does
    ///   not replace what the rest of the array holds. **And as the array's
    ///   address escaping**, which the nullability check's escape scan reads
    ///   only from an `Rvalue::Address`: written any other way, a write
    ///   through the converted pointer would go unseen there.
    /// - **Reached through a pointer**, `*u`: a copy of `u`, since the address
    ///   of an array is the address of its first element, which is the move
    ///   `&*E` being `E` makes in [`Self::finish_value`]. Written as `Address`
    ///   of `*u` with the index appended, it is the `&*pp` shape the memory
    ///   check reads as the address of the local `u` itself, and a store
    ///   through it would land there rather than in the array (#621).
    ///
    /// Either way into a temporary of the element's pointer type, so the
    /// type a later analysis reads off it is the one C gives the result.
    fn decayed(&mut self, builder: &mut Builder, place: Place, span: Span) -> Operand {
        let ty = self
            .unit
            .place_ty(&builder.function, &place)
            .expect("a place this lowering built has a type");
        let Ty::Array { element, .. } = self.unit.ty(ty) else {
            return Operand::Copy(place);
        };
        let pointer = self.unit.push_type(Ty::Pointer(element));
        let first = builder.function.push_local(pointer);
        // **By the last step**, whatever precedes it: an array reached
        // through a pointer decays to that pointer. Asked of the shape
        // instead, `[Index, Deref]` would have been an `Indexed` place and
        // unreachable, where the last `Deref` is what decides (found by
        // review of #1092).
        let value = match place.without_last_deref() {
            Some(pointer) => Rvalue::Use(Operand::Copy(pointer)),
            None => match place.shape() {
                Shape::Local => Rvalue::Address(Place::new(
                    place.local,
                    vec![Projection::Index(Operand::Constant(0))],
                )),
                Shape::Deref(_) | Shape::Indexed => {
                    unreachable!("the lowering builds an `Index` only under `Rvalue::Address`")
                }
            },
        };
        // **And the `const` levels of the element it points at**: the array's
        // own, read as its decayed pointer, or, reached through a pointer to
        // it, that pointer's below its first level. Left unmarked, `GA[0] =
        // "ab"` with `const char *GA[2]` stored through a pointer that said no
        // `const` (#1144). See ADR-0121. Mutation: leave `first` unmarked;
        // `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
        // `into_a_const_static_array` is reported.
        // **Declared and carried apart**, each from its own: the array's
        // type is what `first` declares, and what a cast gave the pointer it
        // was reached through stays carried (ADR-0123).
        let levels = |of: fn(&Function, LocalId) -> u32| match &value {
            Rvalue::Use(Operand::Copy(pointer)) => {
                // A pointer to the array and the pointer to its first
                // element are read in as many loads, so only the steps
                // reaching the pointer shift.
                let below = pointer.steps().len();
                of(&builder.function, pointer.local)
                    .checked_shr(u32::try_from(below).unwrap_or(u32::MAX))
                    .unwrap_or(0)
            }
            _ => of(&builder.function, place.local),
        };
        let (declared, carried) = (
            levels(Function::const_levels),
            levels(Function::carried_levels),
        );
        builder.function.mark_const_levels(first, declared);
        builder.function.mark_carried_levels(first, carried);
        builder.push(Operation {
            place: Place::local(first),
            value,
            origin: Origin::Generated(span),
        });
        Operand::Copy(Place::local(first))
    }

    /// Build a node's value from the operands its children left.
    fn finish_value(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        values: &mut Vec<Operand>,
        places: &mut Vec<Place>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        match self.ast.expr(id) {
            Expr::Cast { operand, span, .. } => {
                let (operand, span) = (*operand, *span);
                self.cast(builder, id, operand, span, values, diagnostics)?;
            }
            // **A call to no function, handed the lists' addresses**, which
            // the memory check reads as a call to code it cannot read, and
            // the backend as LLVM's intrinsic. It writes nowhere and pushes
            // no value, as a call to a function returning `void` does
            // (#1214). See ADR-0126.
            Expr::VaOp { op, span, .. } => {
                let (op, span) = (*op, *span);
                let count = match op {
                    VaOp::Start | VaOp::End => 1,
                    VaOp::Copy => 2,
                };
                let lists = places.split_off(places.len() - count);
                // `&*p` being `p` (C17 6.5.3.2 p3), so a list reached
                // through a pointer is handed that pointer, as a struct's
                // address is. Handed `&*p`, the memory check read a list in
                // a heap block as an address it could not place, and doubted
                // the block's `free`.
                let arguments: Vec<Operand> = lists
                    .into_iter()
                    .map(|list| self.struct_address_of(builder, list, span))
                    .collect();
                let intrinsic = match op {
                    VaOp::Start => Intrinsic::VaStart,
                    VaOp::End => Intrinsic::VaEnd,
                    VaOp::Copy => Intrinsic::VaCopy,
                };
                let then = builder.function.reserve_block();
                builder.end(Terminator::Call {
                    callee: Called::Intrinsic(intrinsic),
                    arguments,
                    destination: None,
                    then: Some(then),
                    origin: Origin::Written(span),
                });
                builder.switch(then);
            }
            // The next argument, read out of the list into a temporary of
            // the type it is read as (#1214). See ADR-0126.
            Expr::VaArg { span, .. } => {
                let span = *span;
                let list = places.pop().expect("a list");
                let into = self.temporary(builder, id, diagnostics)?;
                builder.push(Operation {
                    place: Place::local(into),
                    value: Rvalue::VaArg(list),
                    origin: Origin::Written(span),
                });
                values.push(Operand::Copy(Place::local(into)));
            }
            // A `sizeof` with no value is of a variable length array, whose
            // length is the local it was worked out into, times its
            // element's size (#1209). Mutation: answer the element's size alone;
            // `a_variable_length_array_is_sized_when_declared` fails.
            Expr::SizeofExpr { operand, span } => {
                let (operand, span) = (*operand, *span);
                // An identifier here is a local: an object with static
                // storage duration is never a variable length array (C17
                // 6.7.6.2 p2), so its `sizeof` had a value.
                let array = match self.ast.expr(operand) {
                    Expr::Identifier { .. } => self.local(operand, diagnostics),
                    _ => None,
                };
                let Some((held, element)) =
                    array.and_then(|local| match self.unit.ty(builder.function.local(local)) {
                        Ty::Array {
                            element,
                            length: Length::Local(held),
                        } => Some((held, element)),
                        _ => None,
                    })
                else {
                    return refuse_run_time_sizeof(span, diagnostics);
                };
                let element = self
                    .unit
                    .size_of(element)
                    .expect("a variable length array's element has a size");
                let into = self.temporary(builder, id, diagnostics)?;
                builder.push(Operation {
                    place: Place::local(into),
                    value: Rvalue::Binary {
                        op: BinOp::Mul,
                        lhs: Operand::Copy(Place::local(held)),
                        rhs: Operand::Constant(i128::from(element)),
                    },
                    origin: Origin::Written(span),
                });
                values.push(Operand::Copy(Place::local(into)));
            }
            // **`sizeof` of a type a typedef worked the length of out** is
            // that length times the element's size (#1294): `T` names the
            // typedef's own array node. Any other is refused as before.
            // Mutation: refuse every one; the run of
            // `a_typedef_of_a_variable_length_array_is_sized_where_the_typedef_is`
            // is refused.
            Expr::SizeofType { ty, span } => {
                let (ty, span) = (*ty, *span);
                let Type::Array {
                    element,
                    length: Some(length),
                    ..
                } = *self.ast.ty(ty)
                else {
                    return refuse_run_time_sizeof(span, diagnostics);
                };
                let Some(&(held, _)) = self.typedef_lengths.get(&length) else {
                    return refuse_run_time_sizeof(span, diagnostics);
                };
                let element = self.ty(span, element, diagnostics)?;
                let Some(element) = self.unit.size_of(element) else {
                    return refuse_run_time_sizeof(span, diagnostics);
                };
                let into = self.temporary(builder, id, diagnostics)?;
                builder.push(Operation {
                    place: Place::local(into),
                    value: Rvalue::Binary {
                        op: BinOp::Mul,
                        lhs: Operand::Copy(Place::local(held)),
                        rhs: Operand::Constant(i128::from(element)),
                    },
                    origin: Origin::Written(span),
                });
                values.push(Operand::Copy(Place::local(into)));
            }
            // A constant is its own value, so nothing asks it to finish. The
            // arm is here because the match is written out: an expression kind
            // added later has to say what it does rather than fall through.
            Expr::Number { .. }
            | Expr::Enumerator { .. }
            | Expr::Character { .. }
            | Expr::Length { .. } => {}
            Expr::Identifier { .. }
            | Expr::Subscript { .. }
            | Expr::String { .. }
            | Expr::Member { .. } => {
                let place = places.pop().expect("a place");
                if self
                    .unit
                    .place_ty(&builder.function, &place)
                    .is_some_and(|ty| matches!(self.unit.ty(ty), Ty::Struct(_)))
                {
                    self.struct_value_not_yet(id, self.ast.expr(id).span(), diagnostics);
                    return None;
                }
                let span = self.ast.expr(id).span();
                values.push(self.decayed(builder, place, span));
            }
            Expr::Unary { op, span, .. } => {
                let span = *span;
                match op {
                    AstUnOp::Plus => {}
                    AstUnOp::Minus | AstUnOp::Not | AstUnOp::BitNot => {
                        let operand = values.pop().expect("an operand");
                        let op = match op {
                            AstUnOp::Minus => UnOp::Neg,
                            AstUnOp::Not => UnOp::Not,
                            _ => UnOp::BitNot,
                        };
                        let into = self.temporary(builder, id, diagnostics)?;
                        builder.push(Operation {
                            place: Place::local(into),
                            value: Rvalue::Unary { op, operand },
                            origin: Origin::Written(span),
                        });
                        values.push(Operand::Copy(Place::local(into)));
                    }
                    // `begin_place` put the `Deref` on, because `*p` is the
                    // place rather than `p` being one.
                    AstUnOp::Deref => {
                        let place = places.pop().expect("a place");
                        // `*p` read whole is a struct used as a value too, as
                        // `s` is (#1200 review), where no copy asked for its
                        // place.
                        if self
                            .unit
                            .place_ty(&builder.function, &place)
                            .is_some_and(|ty| matches!(self.unit.ty(ty), Ty::Struct(_)))
                        {
                            self.struct_value_not_yet(id, span, diagnostics);
                            return None;
                        }
                        values.push(self.decayed(builder, place, span));
                    }
                    AstUnOp::AddrOf => {
                        let place = places.pop().expect("a place");
                        // **`&*E` is `E`, and `&E1[E2]` is `E1 + E2`.** C17
                        // 6.5.3.2 p3 says that where the operand of `&` is the
                        // result of a unary `*`, neither operator is evaluated
                        // and the result is as if both were omitted, and that
                        // where it is the result of `[]`, the `&` goes and the
                        // `[]` becomes a `+`.
                        //
                        // One rule reaches both, because a subscript is already
                        // lowered as the addition 6.5.2.1 p2 defines it as: both
                        // arrive here as a place whose last step is a `Deref`,
                        // and taking that step off leaves the place whose value
                        // is the answer. `&c` has no step to take off and is the
                        // case where an address really is taken.
                        //
                        // **A `Deref` rather than any step at all**, and no
                        // mutation can hold the difference: `Projection` has two
                        // kinds and nothing outside a test builds an `Index`, so
                        // the two spellings pick the same places today. The
                        // clause is what makes it a `Deref`: it excepts a unary
                        // `*` and a `[]`, and says nothing about a field
                        // selector, so `&s.f` has to stay an address the day a
                        // struct can be written.
                        //
                        // Without this, `&*p` built an address of what `p`
                        // reaches, which lost the pointer and made `*&*p;` after
                        // a free silent; and it tainted `p`'s own allocation,
                        // which put a "may free it again" warning on a program
                        // with one `free` in it.
                        //
                        // The clause's two exceptions cost nothing here.
                        // The constraints still apply, and what enforces the
                        // one on `*` is that `types.rs` works out no type for
                        // `*x` where `x` is an `int`, so the program never
                        // reaches this arm; the note it is refused with says
                        // the gap is this compiler's rather than the
                        // program's, which is wrong and is #154. And the
                        // result is not an lvalue: `begin_place` has no arm
                        // for an address, so `&*p = q;` is refused exactly as
                        // it was.
                        //
                        // **One step, not every step.** `&**pp` is `*pp` and
                        // not `pp`, so this pops rather than clears, and
                        // `an_address_of_a_double_dereference` is the case
                        // that fails if it clears.
                        if let Some(pointer) = place.without_last_deref() {
                            values.push(Operand::Copy(pointer));
                        } else {
                            let into = self.temporary(builder, id, diagnostics)?;
                            builder.push(Operation {
                                place: Place::local(into),
                                value: Rvalue::Address(place),
                                origin: Origin::Written(span),
                            });
                            values.push(Operand::Copy(Place::local(into)));
                        }
                    }
                    AstUnOp::PreInc | AstUnOp::PreDec | AstUnOp::PostInc | AstUnOp::PostDec => {
                        let place = places.pop().expect("a place");
                        let before = matches!(op, AstUnOp::PostInc | AstUnOp::PostDec);
                        let step = match op {
                            AstUnOp::PreInc | AstUnOp::PostInc => BinOp::Add,
                            _ => BinOp::Sub,
                        };

                        // A postfix operator is the same write and a different
                        // answer: the old value has to be kept before the place
                        // is written, because the place is where it was.
                        let kept = if before {
                            let kept = self.temporary(builder, id, diagnostics)?;
                            builder.push(Operation {
                                place: Place::local(kept),
                                value: Rvalue::Use(Operand::Copy(place.clone())),
                                origin: Origin::Written(span),
                            });
                            Some(kept)
                        } else {
                            None
                        };

                        // 6.5.3.1 p2 and 6.5.2.4 p2 make `++E` and `E++` mean
                        // `E += 1`, which 6.5.16.2 p3 makes `E = E + 1`. So the
                        // step happens at the promoted type, the same as a
                        // compound assignment above, and for the same reason.
                        let stepped = self.promoted(builder, id, &place, diagnostics)?;
                        builder.push(Operation {
                            place: Place::local(stepped),
                            value: Rvalue::Binary {
                                op: step,
                                lhs: Operand::Copy(place.clone()),
                                rhs: Operand::Constant(1),
                            },
                            origin: Origin::Written(span),
                        });
                        builder.push(Operation {
                            place: place.clone(),
                            value: Rvalue::Use(Operand::Copy(Place::local(stepped))),
                            origin: Origin::Written(span),
                        });
                        self.write_back(builder, &place, span);
                        // A prefix operator answers what the place holds after
                        // the write, and that is taken here for the reason the
                        // assignment above gives: what a call does to the same
                        // place afterwards must not change this answer. 6.5.3.1
                        // p2 is the clause.
                        let answer = match kept {
                            Some(kept) => kept,
                            None => {
                                let held = self.temporary(builder, id, diagnostics)?;
                                builder.push(Operation {
                                    place: Place::local(held),
                                    value: Rvalue::Use(Operand::Copy(place)),
                                    origin: Origin::Written(span),
                                });
                                held
                            }
                        };
                        values.push(Operand::Copy(Place::local(answer)));
                    }
                }
            }
            Expr::Binary { op, span, .. } => {
                let (op, span) = (*op, *span);
                let rhs = values.pop().expect("a right operand");
                let lhs = values.pop().expect("a left operand");
                let op = binary(op).expect("`&&` and `||` are branches, not operators");
                let ty = self.ty_of(id, diagnostics)?;

                match Self::unmoved(self.unit.ty(ty), op, &lhs, &rhs) {
                    Some(unmoved) => values.push(unmoved),
                    None => {
                        // **At the type the operation is performed at**, which
                        // is the result's but for a comparison: that is made
                        // at the operands' common type (C17 6.3.1.8) and its
                        // 0 or 1 converted to the `int` it is (6.5.8 p6).
                        // An operation converts its operands to the type it
                        // is written into, so `-1 < 1u` compares 4294967295
                        // with 1. Mutation: compare at the result's type;
                        // `minus_one_is_not_below_one_unsigned` answers 1.
                        let at = match self.types.operated(id) {
                            Some(at) => self.ty(span, at, diagnostics)?,
                            None => ty,
                        };
                        let into = builder.function.push_local(at);
                        // **A pointer moved keeps the `const` levels of the
                        // one it moved**; a comparison or an integer is no
                        // pointer and takes none. See `Lowering::moved_levels`.
                        if matches!(self.unit.ty(at), Ty::Pointer(_)) {
                            let declared = Self::moved_levels(
                                &builder.function,
                                Function::const_levels,
                                [&lhs, &rhs],
                            );
                            let carried = Self::moved_levels(
                                &builder.function,
                                Function::carried_levels,
                                [&lhs, &rhs],
                            );
                            builder.function.mark_const_levels(into, declared);
                            builder.function.mark_carried_levels(into, carried);
                        }
                        builder.push(Operation {
                            place: Place::local(into),
                            value: Rvalue::Binary { op, lhs, rhs },
                            origin: Origin::Written(span),
                        });
                        let result = if at == ty {
                            into
                        } else {
                            let result = builder.function.push_local(ty);
                            builder.push(Operation {
                                place: Place::local(result),
                                value: Rvalue::Use(Operand::Copy(Place::local(into))),
                                origin: Origin::Generated(span),
                            });
                            result
                        };
                        values.push(Operand::Copy(Place::local(result)));
                    }
                }
            }
            Expr::Assign {
                op: None,
                place: written,
                span,
                ..
            } if self.is_struct(*written) => {
                let span = *span;
                let from = places.pop().expect("the struct copied");
                let into = places.pop().expect("the struct copied into");
                self.copy_struct(builder, into, from, span)?;
                // Its value is the struct, which is read whole by nothing
                // that is lowered yet.
                if self.discarding == Some(id) {
                    return Some(());
                }
                self.struct_value_not_yet(id, span, diagnostics);
                return None;
            }
            Expr::Assign { op, span, .. } => {
                let (op, span) = (*op, *span);
                let value = values.pop().expect("a value");
                let place = places.pop().expect("a place");
                let value = match op {
                    // 6.5.16.2 p3: `a += b` is `a = a + b` but for evaluating
                    // `a` once. Spelled that way here too, through a temporary
                    // of the promoted type, so that the two spellings are one
                    // IR and the addition is checked at the width C performs it
                    // at rather than at the width it is stored into.
                    Some(op) => {
                        let computed = self.promoted(builder, id, &place, diagnostics)?;
                        builder.push(Operation {
                            place: Place::local(computed),
                            value: Rvalue::Binary {
                                op: binary(op)
                                    .expect("a compound assignment is never `&&` or `||`"),
                                lhs: Operand::Copy(place.clone()),
                                rhs: value,
                            },
                            origin: Origin::Written(span),
                        });
                        Rvalue::Use(Operand::Copy(Place::local(computed)))
                    }
                    None => Rvalue::Use(value),
                };
                builder.push(Operation {
                    place: place.clone(),
                    value,
                    origin: Origin::Written(span),
                });
                self.write_back(builder, &place, span);

                // **Nothing read back where the value is discarded**, an
                // assignment that is a whole expression statement or a `for`
                // clause: no value is pushed, which `effect` reads as `void`.
                // See `Lowering::discarding`. Mutation: read it back anyway;
                // `a_write_through_a_pointer_that_may_name_itself_is_asked_only_where_it_is_read_again`'s
                // `self_no_read` and `after_an_out_parameter` report `SC0413`.
                if self.discarding == Some(id) {
                    return Some(());
                }
                // 6.5.16 p3: the value is what the left operand holds after
                // the assignment, and that is fixed here rather than read back
                // later. `(b = 1) + g(&b)` is the case: 6.5.2.2 p10 makes the
                // callee's execution indeterminately sequenced with the rest
                // of the expression, so `g` may write `b` before the addition
                // happens, and a copy taken now is one and not seven.
                let held = self.temporary(builder, id, diagnostics)?;
                builder.push(Operation {
                    place: Place::local(held),
                    value: Rvalue::Use(Operand::Copy(place)),
                    origin: Origin::Written(span),
                });
                values.push(Operand::Copy(Place::local(held)));
            }
            Expr::Call {
                callee,
                arguments,
                span,
            } => {
                let copied = arguments
                    .iter()
                    .filter(|&&argument| self.is_struct(argument))
                    .count();
                let (callee, count, span) = (*callee, arguments.len(), *span);
                let at = values.len() - count;
                let mut arguments: Vec<Operand> = values.split_off(at);
                let copies = self
                    .argument_copies
                    .split_off(self.argument_copies.len() - copied);
                // **A function this unit names, or the one a pointer holds**
                // (#1212). The pointer is the value under the arguments; one
                // that is a designator after all, `(0, f)()`, is a direct
                // call. See ADR-0125.
                // **A call to a function whose declaration was refused says
                // nothing more**: the declaration is where the problem is,
                // and was reported there. Mutation: lower its name as a
                // pointer to call through;
                // `a_call_to_a_refused_function_is_not_reported_twice` fails.
                if self.designated(callee).is_none() && self.names_a_refused_function(callee) {
                    return None;
                }
                // **Past a variadic prototype's parameters, each argument is
                // passed promoted** (C17 6.5.2.2 p7): a `char` or a `short`
                // as an `int`, since nothing declares a type to convert it
                // to (#1213). Mutation: pass it as it is;
                // `a_variadic_call_passes_a_char_as_an_int` in
                // `tests/cases/codegen` passes an `i8`.
                if let Some(fixed) = self.variadic_after(callee) {
                    let all: &[ExprId] = match self.ast.expr(id) {
                        Expr::Call { arguments, .. } => arguments,
                        _ => unreachable!("a call finishes as a call"),
                    };
                    for (index, &argument) in all.iter().enumerate().skip(fixed) {
                        // A struct crosses a variadic call as the target's
                        // convention for `va_arg` of it says, which nothing
                        // here lays out yet (#1213).
                        if self.is_struct(argument) {
                            diagnostics.report(
                                Diagnostic::error(
                                    "cannot compile a struct passed past a variadic function's parameters yet",
                                )
                                .with_code(LOWERING)
                                .with_label(Label::primary(
                                    self.ast.expr(argument).span(),
                                    "passed to `...` here",
                                )),
                            );
                            return None;
                        }
                        // **And a constant of any type but `int` as that
                        // type**: an `Operand::Constant` carries none, and
                        // the backend can only write one as `int`, so
                        // `1234567890123LL` crossed as 32 bits. A local of
                        // its own type says what it is. Mutation: leave
                        // constants as they are; `a_variadic_call_passes_a_char_as_an_int`
                        // passes `i32` for the `long long`.
                        let passed = if self.is_promoted_by_default(argument) {
                            Some(self.unit.push_type(Ty::INT))
                        } else if matches!(arguments[index], Operand::Constant(_)) {
                            let ty = self.ty_of(argument, diagnostics)?;
                            (self.unit.ty(ty) != Ty::INT).then_some(ty)
                        } else {
                            None
                        };
                        if let Some(ty) = passed {
                            let promoted = builder.function.push_local(ty);
                            builder.push(Operation {
                                place: Place::local(promoted),
                                value: Rvalue::Use(arguments[index].clone()),
                                origin: Origin::Generated(self.ast.expr(argument).span()),
                            });
                            arguments[index] = Operand::Copy(Place::local(promoted));
                        }
                    }
                }
                let called = match self.designated(callee) {
                    Some(function) => Called::Direct(function),
                    None => match values.pop() {
                        Some(Operand::Function(function)) => Called::Direct(function),
                        Some(pointer) => Called::Through(pointer),
                        None => unreachable!("a call through a pointer pushed the pointer first"),
                    },
                };
                // **A struct result goes into a temporary the caller makes**,
                // its address passed first, and the temporary is the call's
                // value: a place `Task::Placed` takes where one is asked
                // (#1201).
                if self.returns_a_struct(&builder.function, &called) {
                    let ty = self.ty_of(id, diagnostics)?;
                    let result = builder.function.push_local(ty);
                    let pointer = self.unit.push_type(Ty::Pointer(ty));
                    let address = builder.function.push_local(pointer);
                    builder.push(Operation {
                        place: Place::local(address),
                        value: Rvalue::Address(Place::local(result)),
                        origin: Origin::Generated(span),
                    });
                    arguments.insert(0, Operand::Copy(Place::local(address)));
                    let then = builder.function.reserve_block();
                    let returns = self.root != Some(id) || !self.does_not_return(&called);
                    builder.end(Terminator::Call {
                        callee: called,
                        arguments,
                        destination: None,
                        then: returns.then_some(then),
                        origin: Origin::Written(span),
                    });
                    builder.switch(then);
                    end_copies(builder, &copies, span);
                    values.push(Operand::Copy(Place::local(result)));
                    return Some(());
                }
                // A call to a function returning `void` writes nowhere, which
                // is what `Terminator::Call::destination`'s `None` is for:
                // a local made to hold nothing would be a write the source
                // never asked for. It pushes no value either; see `walk`.
                let into = if self.is_void(id) {
                    None
                } else {
                    Some(self.temporary(builder, id, diagnostics)?)
                };
                let then = builder.function.reserve_block();

                // A call ends a block: control leaves the function here, and
                // ADR-0010 is where that is argued. One that does not return
                // names no block to come back to, and whatever is lowered
                // after it still goes into `then`, which nothing reaches.
                //
                // Only where the call is the whole full expression. Inside a
                // larger one, an operand written before the call may still be
                // a place this walk reads only where its operator is lowered,
                // after the call, so `*p + (exit(1), 0)` would move the read
                // of `*p` into `then` and ask nothing about it, where C17 6.5
                // p3 lets it happen first. Keeping the edge there costs a
                // report about code that may not run, which a reader can see.
                let returns = self.root != Some(id) || !self.does_not_return(&called);
                builder.end(Terminator::Call {
                    callee: called,
                    arguments,
                    destination: into.map(Place::local),
                    then: returns.then_some(then),
                    origin: Origin::Written(span),
                });
                builder.switch(then);
                end_copies(builder, &copies, span);
                if let Some(into) = into {
                    values.push(Operand::Copy(Place::local(into)));
                }
            }
            // A conditional is answered by `merge` and never asks to finish,
            // an `Error` is refused before it can, and a comma is answered by
            // `discard` before its right operand is lowered, which leaves that
            // operand's value already standing as the comma's own. All three
            // arms are here because the match is written out rather than
            // wildcarded.
            Expr::Comma { .. } | Expr::Conditional { .. } | Expr::Error { .. } => {}
        }

        Some(())
    }

    /// Build a node's place from what its children left.
    fn finish_place(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        values: &mut Vec<Operand>,
        places: &mut Vec<Place>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        match self.ast.expr(id) {
            Expr::Member { arrow, .. } => {
                let pointer = if *arrow {
                    let operand = values.pop().expect("a pointer to a struct");
                    self.member_base(builder, id, operand, diagnostics)?
                } else {
                    let place = places.pop().expect("a struct");
                    self.struct_address(builder, id, place, diagnostics)?
                };
                let place = self.member(builder, id, pointer, diagnostics)?;
                places.push(place);
            }
            Expr::Unary { .. } => {
                let operand = values.pop().expect("a pointer");
                let place = self.pointed_at(id, operand, diagnostics)?;
                places.push(place.deref());
            }
            // C17 6.5.2.1 p2 defines `E1[E2]` as `(*((E1)+(E2)))`, and this
            // builds exactly that: one C expression is one shape in the IR, so
            // an analysis asking what an access reaches has one thing to read
            // rather than two spellings of it.
            //
            // A subscript on an array is the same shape: the array is converted
            // to a pointer to its first element where it is used as a value
            // ([`Self::decayed`]), so `a[i]` is `*(a + i)` and builds no
            // `Projection::Index` (#621).
            Expr::Subscript { base, index, span } => {
                let (base, index, span) = (*base, *index, *span);
                let offset = values.pop().expect("an index");
                let pointer = values.pop().expect("a base");
                // C17 6.5.2.1 p2 makes `E1[E2]` mean `*((E1)+(E2))`, so the
                // pointer may be either operand: `1[p]` is `p[1]`, and is
                // built as it is. Only the roles swap; both operands were
                // evaluated above, base first, as they are for `p[1]`.
                // `types.rs` has refused every pairing of typed operands that
                // is not a pointer and an integer, so the index is the pointer
                // wherever the base is not; where it is untyped, as in
                // `x[p - q]`, asking its type below is what reports it. Of the
                // pointer's type rather than the subscript's: what is worked
                // out here is the address, and the subscript is what that
                // address reaches.
                //
                // **Each operand's type as its value has it**, which for an
                // array is a pointer to its first element (C17 6.3.2.1 p3):
                // `a[3]` with `a` an `int[4]` adds to an `int *`, and asking the
                // type the frontend wrote took the index for the pointer.
                let base_ty = self.ty_of(base, diagnostics)?;
                let base_ty = self.decayed_ty(base_ty);
                let (base, pointer, offset, addressed) =
                    if matches!(self.unit.ty(base_ty), Ty::Pointer(_)) {
                        (base, pointer, offset, base_ty)
                    } else {
                        let index_ty = self.ty_of(index, diagnostics)?;
                        (index, offset, pointer, self.decayed_ty(index_ty))
                    };

                // `E[0]` is `*E`, so it is built as `*E` is: one C expression
                // is one shape in the IR, which the sentence above this arm has
                // asked for since it was written. See ADR-0021.
                match Self::unmoved(self.unit.ty(addressed), BinOp::Add, &pointer, &offset) {
                    Some(pointer) => {
                        let place = self.pointed_at(base, pointer, diagnostics)?;
                        places.push(place.deref());
                    }
                    None => {
                        let addressed = builder.function.push_local(addressed);
                        let declared = Self::moved_levels(
                            &builder.function,
                            Function::const_levels,
                            [&pointer, &offset],
                        );
                        let carried = Self::moved_levels(
                            &builder.function,
                            Function::carried_levels,
                            [&pointer, &offset],
                        );
                        builder.function.mark_const_levels(addressed, declared);
                        builder.function.mark_carried_levels(addressed, carried);
                        builder.push(Operation {
                            place: Place::local(addressed),
                            value: Rvalue::Binary {
                                op: BinOp::Add,
                                lhs: pointer,
                                rhs: offset,
                            },
                            origin: Origin::Written(span),
                        });
                        places.push(Place::new(addressed, vec![Projection::Deref]));
                    }
                }
            }
            // Nothing else schedules a `FinishPlace`, and the kinds are written
            // out rather than wildcarded because the cost of being wrong is
            // paid elsewhere: an arm that pushed no place would leave the next
            // `places.pop()` taking an outer expression's place instead.
            Expr::Number { .. }
            | Expr::Enumerator { .. }
            | Expr::Character { .. }
            | Expr::Length { .. }
            | Expr::String { .. }
            | Expr::Identifier { .. }
            | Expr::Binary { .. }
            | Expr::Assign { .. }
            | Expr::Conditional { .. }
            | Expr::Call { .. }
            | Expr::Comma { .. }
            | Expr::SizeofType { .. }
            | Expr::SizeofExpr { .. }
            | Expr::Cast { .. }
            | Expr::VaOp { .. }
            | Expr::VaArg { .. }
            | Expr::Error { .. } => {}
        }

        Some(())
    }

    /// The place an operand names, or a report that it names none.
    ///
    /// Everything this stage builds a value from is either a constant or a
    /// copy of a place, so a projection onto a constant is the only way to get
    /// here: `*0` is that shape. The frontend refuses it today, because
    /// `types.rs` reports an indirection through a non-pointer and gives it
    /// no type, and reporting rather than returning is what keeps that from being an
    /// invariant somebody has to remember: the two stacks stay in step, and a
    /// change upstream cannot turn this into a panic.
    fn pointed_at(
        &mut self,
        id: ExprId,
        operand: Operand,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Place> {
        match operand {
            Operand::Copy(place) => Some(place),
            // `*f` of a designator is the designator, which `begin_value`
            // answers before any place is asked, so none arrives here; one
            // that did would be a function read as an object.
            Operand::Function(_) => {
                diagnostics.report(
                    Diagnostic::error("a function is not an object to reach")
                        .with_code(LOWERING)
                        .with_label(Label::primary(
                            self.ast.expr(id).span(),
                            "this designates a function",
                        )),
                );
                None
            }
            Operand::Constant(_) => {
                diagnostics.report(
                    Diagnostic::error("a constant does not point at anything")
                        .with_code(LOWERING)
                        .with_label(Label::primary(
                            self.ast.expr(id).span(),
                            "this has nothing to reach",
                        )),
                );
                None
            }
        }
    }

    /// The local holding the pointer `p->m` reads through: `p` itself where
    /// it is a local, and a temporary holding its value otherwise, since a
    /// member's address is taken through a local (`Self::member`).
    fn member_base(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        operand: Operand,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<LocalId> {
        let Expr::Member { base, .. } = *self.ast.expr(id) else {
            unreachable!("asked of a member access");
        };
        if let Operand::Copy(place) = &operand {
            if place.steps().is_empty() {
                return Some(place.local);
            }
        }
        let into = self.temporary(builder, base, diagnostics)?;
        let carried = Self::moved_levels(&builder.function, Function::carried_levels, [&operand]);
        builder.function.mark_carried_levels(into, carried);
        builder.push(Operation {
            place: Place::local(into),
            value: Rvalue::Use(operand),
            origin: Origin::Generated(self.ast.expr(base).span()),
        });
        Some(into)
    }

    /// The local holding the address of the struct `s.m` reads a member of,
    /// `place` being where `s` is: `&s` where it is a local, the pointer
    /// gone through where it is reached through one, `(*p).m` being `p->m`
    /// (C17 6.5.2.3 p4 and 6.5.3.2 p3), and a temporary holding that
    /// pointer where it was read out of memory.
    fn struct_address(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        place: Place,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<LocalId> {
        let Expr::Member { base, .. } = *self.ast.expr(id) else {
            unreachable!("asked of a member access");
        };
        let span = self.ast.expr(base).span();
        let struct_ty = self.ty_of(base, diagnostics)?;
        let pointer_ty = self.unit.push_type(Ty::Pointer(struct_ty));
        // **Typed as the struct's pointer, not as `base`**, which is the
        // struct itself: `(**pp).m` took a temporary of `base`'s type, so a
        // pointer was held in a struct-typed local and the backend refused
        // the copy as a whole struct (#1200 review). Mutation: take
        // `Self::member_base`'s temporary for it;
        // `a_member_through_a_pointer_read_out_of_memory_is_read`'s
        // `through_two` is `SC0801`.
        if let Some(pointer) = place.without_last_deref() {
            if pointer.steps().is_empty() {
                return Some(pointer.local);
            }
            let into = builder.function.push_local(pointer_ty);
            let operand = Operand::Copy(pointer);
            let carried =
                Self::moved_levels(&builder.function, Function::carried_levels, [&operand]);
            builder.function.mark_carried_levels(into, carried);
            if let Some(written) = self.types.of(base) {
                builder
                    .function
                    .mark_const_levels(into, self.pointing_at_levels(written));
            }
            builder.push(Operation {
                place: Place::local(into),
                value: Rvalue::Use(operand),
                origin: Origin::Generated(span),
            });
            return Some(into);
        }
        let into = builder.function.push_local(pointer_ty);
        if let Some(written) = self.types.of(base) {
            builder
                .function
                .mark_const_levels(into, self.pointing_at_levels(written));
        }
        builder.push(Operation {
            place: Place::local(into),
            value: Rvalue::Address(place),
            origin: Origin::Generated(span),
        });
        Some(into)
    }

    /// The place of member `id` names through `pointer`, a local holding the
    /// address of its struct: `*t` with `t = &(*pointer).m`. **A `Field` only
    /// ever under that address**, as an `Index` is under an array's, so every
    /// read and write of a member is through a pointer and is asked as any
    /// other is: the memory check reads the address as the pointer moved by
    /// the member's offset, the slot of its object (ADR-0124).
    fn member(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        pointer: LocalId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Place> {
        let Some(member) = self.types.member(id) else {
            unreachable!("the type check records the member every access it typed names");
        };
        let (definition, path) = (member.definition, member.path.clone());
        // Its struct laid out, which a pointer to it reached through `->`
        // did not ask for.
        let mut within = self.struct_of(self.ast.expr(id).span(), definition, true, diagnostics)?;
        // The layout of the member named, through each anonymous member on
        // the way, for a bit-field's unit and bits.
        let (last, through) = path.split_last().expect("a member's path is never empty");
        for &index in through {
            let Ty::Struct(inner) = self
                .unit
                .ty(self.unit.struct_layout(within).members[index].ty)
            else {
                unreachable!("an anonymous member is a struct or a union");
            };
            within = inner;
        }
        let laid = self.unit.struct_layout(within).members[*last];
        // **A bit-field is reached through its unit** (#1206): the address is
        // the unit's, of the unit's type, and the place is a temporary the
        // bits are taken into. See `Lowering::bit_fields`.
        //
        // The member's type is the one its struct was laid out with, which is
        // the expression's but for a bit-field's unit and a flexible array
        // member, an array of none in the IR (#1208) and of no length in C.
        let member_ty = laid.ty;
        // **One `Field` per step**, through each anonymous member to the one
        // named (#1207): the place is `(*pointer).anonymous.m`, and every
        // reader of a place already walks a step at a time.
        let mut steps = vec![Projection::Deref];
        steps.extend(path.iter().map(|&index| {
            Projection::Field(u32::try_from(index).expect("fewer than 2^32 members"))
        }));
        let pointer_ty = self.unit.push_type(Ty::Pointer(member_ty));
        let address = builder.function.push_local(pointer_ty);
        let mut levels = Self::moved_levels(
            &builder.function,
            Function::const_levels,
            [&Operand::Copy(Place::local(pointer))],
        ) & 1;
        if let Some(written) = self.types.of(id) {
            levels |= self.pointing_at_levels(written);
        }
        builder.function.mark_const_levels(address, levels);
        builder.push(Operation {
            place: Place::local(address),
            value: Rvalue::Address(Place::new(pointer, steps)),
            origin: Origin::Written(self.ast.expr(id).span()),
        });
        let Some(bits) = laid.bits else {
            return Some(Place::local(address).deref());
        };
        let declared = self.declared_member(definition, &path);
        // A plain `int` bit-field is signed, as `clang` makes it (C17 6.7.2
        // p5 leaves it to the implementation), and `char` is as the target
        // makes a plain `char`. Every kind of type answered, so one added
        // later is `error[E0004]` here.
        let signed = match self.ast.ty(declared) {
            Type::Integer { rank, sign } => self.unit.target().integer(*rank, *sign).signed(),
            Type::Void
            | Type::VaList
            | Type::Pointer(_)
            | Type::Array { .. }
            | Type::Function { .. }
            | Type::Struct { .. } => {
                unreachable!("the type check holds a bit-field to an integer type")
            }
        };
        let field = BitField {
            unit: Place::local(address).deref(),
            unit_ty: laid.ty,
            bits,
            signed,
        };
        let value = self.temporary(builder, id, diagnostics)?;
        self.extract(builder, &field, value, self.ast.expr(id).span());
        builder.bit_fields.insert(value, field);
        Some(Place::local(value))
    }

    /// The type the member at `path` of `definition` was declared with: a
    /// bit-field's, which its unit is not.
    fn declared_member(&self, definition: TypeId, path: &[usize]) -> TypeId {
        let mut ty = definition;
        for &index in path {
            let Type::Struct {
                members: Some(members),
                ..
            } = self.ast.ty(ty)
            else {
                unreachable!("a member's path goes through struct definitions");
            };
            ty = members[index].ty;
        }
        ty
    }

    /// Take the bit-field `field` out of its unit into `into` (#1206).
    ///
    /// Unsigned, it is shifted down to bit 0 and masked to its width. Signed,
    /// it is shifted up so its top bit is the unit's, read as the unit's
    /// signed type, and shifted back down there, which copies that top bit
    /// into every bit above the width: the sign, carried. Every step is at
    /// the unit's own width; `into` has the type the access is typed at,
    /// which may be another, and the last store converts to it.
    fn extract(&mut self, builder: &mut Builder, field: &BitField, into: LocalId, span: Span) {
        let unit_bits = self.unit_bits(field);
        let (at, width) = (u64::from(field.bits.at), u64::from(field.bits.width));
        let read = self.bit_step(
            builder,
            field.unit_ty,
            Rvalue::Use(Operand::Copy(field.unit.clone())),
            span,
        );
        let value = if field.signed {
            let up = self.bit_step(
                builder,
                field.unit_ty,
                Rvalue::Binary {
                    op: BinOp::Shl,
                    lhs: Operand::Copy(Place::local(read)),
                    rhs: Operand::Constant(i128::from(unit_bits - at - width)),
                },
                span,
            );
            let signed_ty = self.signed_of(field.unit_ty);
            let as_signed = self.bit_step(
                builder,
                signed_ty,
                Rvalue::Use(Operand::Copy(Place::local(up))),
                span,
            );
            self.bit_step(
                builder,
                signed_ty,
                Rvalue::Binary {
                    op: BinOp::Shr,
                    lhs: Operand::Copy(Place::local(as_signed)),
                    rhs: Operand::Constant(i128::from(unit_bits - width)),
                },
                span,
            )
        } else {
            let down = self.bit_step(
                builder,
                field.unit_ty,
                Rvalue::Binary {
                    op: BinOp::Shr,
                    lhs: Operand::Copy(Place::local(read)),
                    rhs: Operand::Constant(i128::from(at)),
                },
                span,
            );
            self.bit_step(
                builder,
                field.unit_ty,
                Rvalue::Binary {
                    op: BinOp::BitAnd,
                    lhs: Operand::Copy(Place::local(down)),
                    rhs: Operand::Constant(mask(width)),
                },
                span,
            )
        };
        builder.push(Operation {
            place: Place::local(into),
            value: Rvalue::Use(Operand::Copy(Place::local(value))),
            origin: Origin::Generated(span),
        });
    }

    /// Where `written` is a bit-field's temporary, put what it now holds
    /// back into the unit, the unit's other bits kept, and take the
    /// bit-field out again, so the value the write leaves is what the
    /// bit-field holds, cut to its width (C17 6.5.16 p3) (#1206). Asked after
    /// every write to a place: an assignment, compound or not, and `++` and
    /// `--`.
    ///
    /// Mutation: skip this; `a_bit_field_is_read_and_written_within_its_unit`
    /// in `tests/run.rs` fails, nothing written reaching the struct.
    fn write_back(&mut self, builder: &mut Builder, written: &Place, span: Span) {
        if !written.steps().is_empty() {
            return;
        }
        let Some(field) = builder.bit_fields.get(&written.local).cloned() else {
            return;
        };
        let unit_bits = self.unit_bits(&field);
        let (at, width) = (u64::from(field.bits.at), u64::from(field.bits.width));
        let kept = mask(unit_bits) ^ (mask(width) << at);
        let read = self.bit_step(
            builder,
            field.unit_ty,
            Rvalue::Use(Operand::Copy(field.unit.clone())),
            span,
        );
        let cleared = self.bit_step(
            builder,
            field.unit_ty,
            Rvalue::Binary {
                op: BinOp::BitAnd,
                lhs: Operand::Copy(Place::local(read)),
                rhs: Operand::Constant(kept),
            },
            span,
        );
        let value = self.bit_step(
            builder,
            field.unit_ty,
            Rvalue::Use(Operand::Copy(written.clone())),
            span,
        );
        let cut = self.bit_step(
            builder,
            field.unit_ty,
            Rvalue::Binary {
                op: BinOp::BitAnd,
                lhs: Operand::Copy(Place::local(value)),
                rhs: Operand::Constant(mask(width)),
            },
            span,
        );
        let placed = self.bit_step(
            builder,
            field.unit_ty,
            Rvalue::Binary {
                op: BinOp::Shl,
                lhs: Operand::Copy(Place::local(cut)),
                rhs: Operand::Constant(i128::from(at)),
            },
            span,
        );
        let merged = self.bit_step(
            builder,
            field.unit_ty,
            Rvalue::Binary {
                op: BinOp::BitOr,
                lhs: Operand::Copy(Place::local(cleared)),
                rhs: Operand::Copy(Place::local(placed)),
            },
            span,
        );
        builder.push(Operation {
            place: field.unit.clone(),
            value: Rvalue::Use(Operand::Copy(Place::local(merged))),
            origin: Origin::Generated(span),
        });
        self.extract(builder, &field, written.local, span);
    }

    /// A temporary of type `ty` holding `value`: one step of taking a
    /// bit-field out of its unit or putting it back.
    fn bit_step(&mut self, builder: &mut Builder, ty: TyId, value: Rvalue, span: Span) -> LocalId {
        let local = builder.function.push_local(ty);
        builder.push(Operation {
            place: Place::local(local),
            value,
            origin: Origin::Generated(span),
        });
        local
    }

    /// How many bits `field`'s unit takes.
    fn unit_bits(&self, field: &BitField) -> u64 {
        u64::from(
            self.unit
                .integer(field.unit_ty)
                .expect("a bit-field's unit is an integer")
                .bits(),
        )
    }

    /// The signed integer type of `ty`'s rank, `ty` being an unsigned one.
    fn signed_of(&mut self, ty: TyId) -> TyId {
        let Ty::Integer { rank, .. } = self.unit.ty(ty) else {
            unreachable!("a bit-field's unit is an integer");
        };
        self.unit.push_type(Ty::Integer {
            rank,
            sign: safec_ir::ir::Sign::Signed,
        })
    }

    /// Lay out every struct a pointer is moved over or subtracted across
    /// and nothing else laid out, once every function is lowered (#638).
    ///
    /// **A struct behind a pointer is not laid out where the pointer is
    /// written** (#1202 review, see `Lowering::ty`), so one a function only
    /// ever reaches through pointers had no size: `q - p` over it was
    /// refused as having none, and `p + 1` panicked asking for its layout,
    /// while the same function built once anything else in the file declared
    /// an object of it. Here no struct is being laid out, so the recursion
    /// that rule avoids cannot arise, and only a struct the IR steps over is
    /// asked, so one with a member this compiler refuses is still refused
    /// only where it is used. Mutation: skip this;
    /// `a_struct_only_pointers_reach_is_stepped_and_subtracted` panics.
    fn lay_out_stepped(&mut self, diagnostics: &mut DiagnosticSink) {
        let mut stepped = Vec::new();
        for func in self.unit.functions() {
            let function = self.unit.function(func);
            // A declaration has no body to step over anything.
            if !function.is_defined() {
                continue;
            }
            for block in function.blocks() {
                for element in &block.elements {
                    let Element::Assign(Operation {
                        value:
                            Rvalue::Binary {
                                op: BinOp::Add | BinOp::Sub,
                                lhs,
                                rhs,
                            },
                        ..
                    }) = element
                    else {
                        continue;
                    };
                    for operand in [lhs, rhs] {
                        let Operand::Copy(place) = operand else {
                            continue;
                        };
                        let Some(ty) = self.unit.place_ty(function, place) else {
                            continue;
                        };
                        if let Ty::Pointer(pointee) = self.unit.ty(ty) {
                            if let Ty::Struct(id) = self.unit.ty(pointee) {
                                if self.unit.layout_of(id).is_none() {
                                    stepped.push(id);
                                }
                            }
                        }
                    }
                }
            }
        }
        for id in stepped {
            if self.unit.layout_of(id).is_none() {
                if let Some(&(at, ty)) = self.written.get(&id) {
                    let _ = self.struct_of(at, ty, true, diagnostics);
                }
            }
        }
    }

    /// Work out every array length with no constant value in the type a
    /// typedef declared, `ty`, where the typedef is reached (C17 6.7.8 p3,
    /// #1294): each into a local of `size_t`, recorded in `typedef_lengths`
    /// for what reads the type later, evaluated outermost first.
    ///
    /// Through arrays, pointers and a function's return, and **not into a
    /// function's parameters**, whose lengths 6.7.6.2 p5 treats as `*`. A
    /// typedef none of whose lengths needs working out writes nothing.
    ///
    /// **A length already worked out is not worked out again**: one an
    /// earlier typedef wrote, which `typedef T U;`, `typedef T *P;` or
    /// `typedef const T U;` reaches through T's type, is T's and was
    /// evaluated where T was (p3 evaluates only what the typedef's own
    /// declarator wrote). A typedef reached again at run time, in a loop,
    /// still evaluates its own, since this is asked once per typedef as it
    /// is lowered. Mutation: evaluate every length met; the run of
    /// `a_typedef_of_a_variable_length_array_is_sized_where_the_typedef_is`'s
    /// `alias` gives the later size and calls `f` twice.
    ///
    /// **Each local is a variably modified identifier's** for what a jump
    /// asks: it is in its scope's list, and begins there, so a jump past the
    /// typedef into its scope is refused under `SC0323` (6.8.6.1 p1), as one
    /// past an array's declaration is. Mutation: leave it out of the list;
    /// `a_jump_into_the_scope_of_a_variable_length_typedef_is_refused` is
    /// accepted.
    fn typedef_lengths_of(
        &mut self,
        builder: &mut Builder,
        ty: TypeId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let mut current = ty;
        loop {
            match *self.ast.ty(current) {
                Type::Array {
                    element,
                    length: Some(length),
                    ..
                } => {
                    if self.types.value(length).is_none()
                        && !self.typedef_lengths.contains_key(&length)
                    {
                        let at = self.ast.expr(length).span();
                        let counted = self.value(builder, length, diagnostics)?;
                        let (rank, sign) = self.unit.target().size_type();
                        let size = self.unit.push_type(Ty::Integer { rank, sign });
                        let held = builder.function.push_local(size);
                        let scope = self.scopes.last_mut().expect("a scope is open");
                        scope.declared.push(held);
                        builder.element(Element::StorageLive {
                            local: held,
                            origin: Origin::Generated(at),
                        });
                        builder.push(Operation {
                            place: Place::local(held),
                            value: Rvalue::Use(counted),
                            origin: Origin::Generated(at),
                        });
                        builder.function.mark_variable_length(held);
                        self.typedef_lengths.insert(length, (held, at));
                        self.typedef_locals.push((held, at));
                    }
                    current = element;
                }
                Type::Array { element, .. } => current = element,
                Type::Pointer(pointee) => current = pointee,
                // The return, which 6.7.6 p3 puts in the declarator's
                // chain, and not the parameters. Mutation: stop here;
                // the run's `returning` does not call `f`.
                Type::Function { returns, .. } => current = returns,
                _ => return Some(()),
            }
        }
    }

    /// The IR type of a variable length array declared with type `ty`, its
    /// length worked out into a local of `size_t` here, where the
    /// declaration is reached (C17 6.7.6.2 p5, #1209), or `None` where `ty`
    /// is no such array. Only an array whose own length is not a constant
    /// and whose element has a size: one nested deeper, or behind a
    /// pointer, is refused by [`Self::ty`] as before.
    ///
    /// The length is not checked against zero or a negative value, which C
    /// makes undefined (p5). The memory check doubts every declaration of
    /// one instead, since it cannot show the length is above zero:
    /// `a_variable_length_array_is_doubted_where_its_length_is_unknown`.
    ///
    /// **An array type a typedef named takes the length the typedef worked
    /// out** (`specified`, what the declaration's specifiers wrote, is `ty`
    /// itself, and `typedef_lengths` holds it): C17 6.7.8 p3 works it out
    /// once, where the typedef is reached, and working it out again here, at
    /// each object, gave a different size wherever the length had changed
    /// since (#1294). Copied into the object's own local, so a later pass
    /// over the typedef, in a loop, cannot resize an object still alive.
    /// Mutation: evaluate the length here instead; the run of
    /// `a_typedef_of_a_variable_length_array_is_sized_where_the_typedef_is`
    /// gives the later size. One the table does not hold, which no typedef
    /// this function reached declared, is refused as before.
    fn variable_length(
        &mut self,
        builder: &mut Builder,
        ty: TypeId,
        specified: TypeId,
        span: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Option<TyId>> {
        let Type::Array {
            element,
            length: Some(length),
            ..
        } = *self.ast.ty(ty)
        else {
            return Some(None);
        };
        // A constant length is `Self::ty`'s, which refuses one that is not
        // positive as before; the type check has refused it first in
        // every program measured, so asking `positive_length` here instead
        // changes nothing a test can see.
        if self.types.value(length).is_some() {
            return Some(None);
        }
        let typedef = self.typedef_lengths.get(&length).copied();
        // A type a typedef named has had its length worked out where the
        // typedef is, before anything of the type is declared: a typedef
        // has block scope here (6.7.6.2 p2, and the type check refuses one
        // at file scope), and every one this function reaches is lowered in
        // order. Asked rather than assumed, so that a way round it panics
        // naming what was missed instead of working the length out again.
        assert!(
            ty != specified || typedef.is_some(),
            "an object of a typedef's variable length type before the typedef's length"
        );
        let element = self.ty(span, element, diagnostics)?;
        let counted = match typedef {
            Some((from, _)) => Operand::Copy(Place::local(from)),
            None => self.value(builder, length, diagnostics)?,
        };
        let (rank, sign) = self.unit.target().size_type();
        let size = self.unit.push_type(Ty::Integer { rank, sign });
        let held = builder.function.push_local(size);
        builder.push(Operation {
            place: Place::local(held),
            value: Rvalue::Use(counted),
            origin: Origin::Generated(self.ast.expr(length).span()),
        });
        Some(Some(self.unit.push_type(Ty::Array {
            element,
            length: Length::Local(held),
        })))
    }

    /// The `const` levels of a pointer to an object of type `ty`: bit `0`
    /// where `ty` itself is `const`, and `ty`'s own levels one further down.
    /// See [`Self::const_levels`].
    fn pointing_at_levels(&self, ty: TypeId) -> u32 {
        u32::from(self.ast.is_const(ty)) | (self.const_levels(ty) << 1)
    }

    /// Branch on the first operand of a `&&`, `||` or `?:`.
    fn split(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        tasks: &mut Vec<Task>,
        values: &mut Vec<Operand>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        let condition = values.pop().expect("a condition");
        // Only a `?:` can be `void`: a `&&` or `||` is an `int`.
        let answer = if self.is_void(id) {
            None
        } else {
            Some(self.temporary(builder, id, diagnostics)?)
        };
        let join = builder.function.reserve_block();

        match self.ast.expr(id) {
            Expr::Binary { op, lhs, rhs, .. } => {
                let (op, lhs, rhs) = (*op, *lhs, *rhs);
                let answer = answer.expect("a `&&` or `||` is an `int`");
                // The left operand decides the answer unless the right is
                // reached, so it is written before the branch and overwritten
                // after. What is written is whether it is non-zero and not
                // what it is: C17 6.5.13 p3 and 6.5.14 p3 say `&&` and `||`
                // yield 1 or 0, so `3 && 5` is 1 rather than 5. At the left
                // operand's span and not the whole `a && b`'s, because a read
                // in it is what a caret here points at: `*p && c` after a free
                // underlines `*p`.
                builder.push(Operation {
                    place: Place::local(answer),
                    value: truth(condition),
                    origin: Origin::Written(self.decides(lhs)),
                });

                let second = builder.function.reserve_block();
                let (then, otherwise) = match op {
                    AstBinOp::LogAnd => (second, join),
                    _ => (join, second),
                };
                // The left operand, not the whole `a && b`. What this branch
                // asks is whether the left one was non-zero, so that is the
                // expression that decides, and the field says the expression
                // rather than what it is part of. Nothing points here today,
                // because the condition is a temporary and a temporary has no
                // projection to dereference, but a field that is only true
                // where somebody happens to read it is not true.
                builder.end(Terminator::Branch {
                    condition: Operand::Copy(Place::local(answer)),
                    then,
                    otherwise,
                    origin: Origin::Written(self.decides(lhs)),
                });
                builder.switch(second);
                // C17 6.5.13 p4 and 6.5.14 p4: if the right operand is
                // evaluated, a sequence point separates it from the left. This
                // block is the one it is evaluated in, so the head of it is
                // where that point falls.
                if self.top_level {
                    builder.sequenced(self.ast.expr(lhs).span());
                }

                self.pending.insert(
                    id,
                    Pending {
                        answer: Some(answer),
                        join,
                        otherwise: None,
                    },
                );
                tasks.push(Task::Merge(id));
                tasks.push(Task::Value(rhs));
            }
            Expr::Conditional {
                condition: asked,
                then,
                ..
            } => {
                let (asked, then) = (*asked, *then);
                let taken = builder.function.reserve_block();
                let otherwise = builder.function.reserve_block();
                builder.end(Terminator::Branch {
                    condition,
                    then: taken,
                    otherwise,
                    origin: Origin::Written(self.decides(asked)),
                });
                builder.switch(taken);
                // C17 6.5.15 p4: between the first operand and whichever of
                // the other two is evaluated. Both arms get one, here and in
                // [`Lowering::second`], because either may be the one that
                // runs.
                if self.top_level {
                    builder.sequenced(self.ast.expr(asked).span());
                }

                self.pending.insert(
                    id,
                    Pending {
                        answer,
                        join,
                        otherwise: Some(otherwise),
                    },
                );
                tasks.push(Task::Second(id));
                tasks.push(Task::Value(then));
            }
            // Only a `&&`, a `||` and a `?:` split, and the kinds are written
            // out because an arm that fell through here would leave `join`
            // reserved and never filled, which is a panic at whatever later
            // moment somebody walks the graph.
            Expr::Number { .. }
            | Expr::Enumerator { .. }
            | Expr::Character { .. }
            | Expr::Length { .. }
            | Expr::String { .. }
            | Expr::Identifier { .. }
            | Expr::Unary { .. }
            | Expr::Assign { .. }
            | Expr::Call { .. }
            | Expr::Subscript { .. }
            | Expr::Member { .. }
            | Expr::Comma { .. }
            | Expr::SizeofType { .. }
            | Expr::SizeofExpr { .. }
            | Expr::Cast { .. }
            | Expr::VaOp { .. }
            | Expr::VaArg { .. }
            | Expr::Error { .. } => {}
        }

        Some(())
    }

    /// Write the `then` arm of a `?:` and start its `else`.
    /// The left operand of a comma is done, and nobody wants its value.
    ///
    /// C17 6.5.17 p2 evaluates it as a void expression, which is what
    /// [`Builder::discarded`] records and is the same thing an expression
    /// statement does. The span is the left operand's own, so `*p, i;`
    /// underlines `*p` rather than both.
    ///
    /// **The other half of a comma is not this precise.** `i, *p;` is
    /// discarded by the statement, which has the whole comma in hand and
    /// nothing narrower, so it underlines `i, *p`. #147 narrowed a controlling
    /// expression's comma to its right operand and left this one, which
    /// records a value rather than deciding anything, to an issue of its own.
    fn discard(&mut self, builder: &mut Builder, id: ExprId, values: &mut Vec<Operand>) {
        let Expr::Comma { lhs, .. } = self.ast.expr(id) else {
            // Only the arm above pushes this, and it pushes it for a comma.
            // Answering nothing rather than panicking, because what a wrong
            // task would cost here is one element missing from one block, and
            // `Expr::Conditional` sets the same precedent one task over.
            return;
        };
        let lhs_id = *lhs;
        let lhs = self.ast.expr(lhs_id).span();
        // A left operand that pushed no value has none to discard.
        if self.pushes(lhs_id) {
            let value = values.pop().expect("a left operand");
            builder.discarded(value, lhs);
        }

        // C17 6.5.17 p2. This is a task between the operands rather than
        // something the comma builds when it finishes, because an element
        // built where a node finishes lands after everything the node
        // contains, and what this one says is about what came before it.
        if self.top_level {
            builder.sequenced(lhs);
        }
    }

    fn second(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        tasks: &mut Vec<Task>,
        values: &mut Vec<Operand>,
    ) {
        let Expr::Conditional {
            condition,
            then,
            otherwise,
            span,
        } = self.ast.expr(id)
        else {
            return;
        };
        let (condition, then, otherwise, span) = (*condition, *then, *otherwise, *span);
        let pending = &self.pending[&id];
        let (answer, join, start) = (pending.answer, pending.join, pending.otherwise);

        // A `void` `?:` has no answer to write. An arm that still pushed a
        // value, `*p` of a `void *`, is read on its own path and discarded
        // there, as an expression statement would.
        match answer {
            Some(answer) => {
                let value = values.pop().expect("the first arm");
                builder.push(Operation {
                    place: Place::local(answer),
                    value: Rvalue::Use(value),
                    origin: Origin::Written(span),
                });
            }
            None => {
                if self.pushes(then) {
                    let value = values.pop().expect("the first arm");
                    builder.discarded(value, self.ast.expr(then).span());
                }
            }
        }
        builder.end(Terminator::Goto(join));
        builder.switch(start.expect("a `?:` reserves its second arm"));
        // The other half of 6.5.15 p4, for the arm taken when the condition
        // was zero. [`Lowering::split`] has the first.
        if self.top_level {
            builder.sequenced(self.ast.expr(condition).span());
        }

        tasks.push(Task::Merge(id));
        tasks.push(Task::Value(otherwise));
    }

    /// Write the last arm and come back together.
    fn merge(&mut self, builder: &mut Builder, id: ExprId, values: &mut Vec<Operand>) {
        let span = self.ast.expr(id).span();
        let Pending { answer, join, .. } = self.pending.remove(&id).expect("a branch to merge");

        // A `void` `?:` has no value, so the conditional pushes none, and an
        // arm that pushed one is discarded on its own path as `second` does.
        // See `walk`.
        let Some(answer) = answer else {
            if let Expr::Conditional { otherwise, .. } = self.ast.expr(id) {
                let otherwise = *otherwise;
                if self.pushes(otherwise) {
                    let value = values.pop().expect("the last arm");
                    builder.discarded(value, self.ast.expr(otherwise).span());
                }
            }
            builder.end(Terminator::Goto(join));
            builder.switch(join);
            return;
        };
        let value = values.pop().expect("the last arm");
        // A `&&` or `||` answers 1 or 0 and a `?:` answers what its arm is
        // worth. C17 6.5.13 p3 and 6.5.14 p3 say the first, and 6.5.15 p4 the
        // second: the conditional operator's value is the operand's, converted,
        // rather than a truth value.
        //
        // A `&&` or `||` writes at its right operand's span, as `split` writes
        // the left at the left's. A `?:` arm keeps the whole span for now:
        // what its write points at is a different question, left to its own
        // issue.
        let (value, span) = match self.ast.expr(id) {
            Expr::Binary { rhs, .. } => (truth(value), self.decides(*rhs)),
            _ => (Rvalue::Use(value), span),
        };
        builder.push(Operation {
            place: Place::local(answer),
            value,
            origin: Origin::Written(span),
        });
        builder.end(Terminator::Goto(join));
        builder.switch(join);

        values.push(Operand::Copy(Place::local(answer)));
    }

    /// A local to hold what an expression works out.
    ///
    /// Every value that is not already in a place gets one, because a place is
    /// what the analyses ask about: `docs/safety-model.md`'s memory axis asks
    /// whether a place is still allocated, and a value with nowhere to live is
    /// a question it cannot be asked.
    /// Finish the cast `id` of `operand`, whose value is on `values`. See
    /// ADR-0117.
    ///
    /// - To `void`, the operand was evaluated for its effects (C17 6.5.4
    ///   p6 with 6.3.2.2) and nothing is kept: a value read through a place
    ///   is still a read, as a discarded comma operand's is.
    /// - Between two integer types, or between two pointer types, a copy into
    ///   a temporary of the type converted to, as an assignment converts:
    ///   the memory check reads a pointer whose element type changed as it
    ///   reads one converted through `void *`, in bytes.
    /// - A null pointer constant to a pointer type is the null it is.
    /// - Between a pointer and an integer otherwise, [`Rvalue::Reinterpret`],
    ///   which the memory check reports unproven where it is written.
    ///
    /// A cast between pointer types that drops `const` is a copy as any
    /// other is: what may not be written is the object, and the memory check
    /// follows that through every spelling of the pointer (ADR-0120). Made
    /// a value nothing follows, a free through it was not followed and a use
    /// after it went unreported (#1122). Mutation: lower it as
    /// `Rvalue::Reinterpret`; `a_free_through_a_cast_that_drops_const_is_followed`
    /// fails.
    fn cast(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        operand: ExprId,
        span: Span,
        values: &mut Vec<Operand>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        if self.is_void(id) {
            if self.pushes(operand) {
                let value = values.pop().expect("an operand");
                builder.discarded(value, self.ast.expr(operand).span());
            }
            return Some(());
        }
        let value = values.pop().expect("an operand");
        let pointer = |ty: Option<TypeId>, ast: &Ast| match ty.map(|ty| ast.ty(ty)) {
            Some(Type::Pointer(_) | Type::Array { .. } | Type::Function { .. }) => true,
            Some(Type::Integer { .. } | Type::Void | Type::Struct { .. } | Type::VaList) | None => {
                false
            }
        };
        let (to, from) = (
            pointer(self.types.of(id), self.ast),
            pointer(self.types.of(operand), self.ast),
        );
        let rvalue = match (to, from, &value) {
            (true, false, Operand::Constant(0)) | (true, true, _) | (false, false, _) => {
                Rvalue::Use(value)
            }
            (true, false, _) | (false, true, _) => Rvalue::Reinterpret(value),
        };
        let into = self.temporary(builder, id, diagnostics)?;
        // **What a pointer to `const` held, it still may**, `const` dropped
        // or not: `(int *)p` with `p` a parameter may name an object defined
        // `const` the caller handed. Found by the review of #1122: the cast
        // was what asked such a pointer, and followed it asked nothing.
        // Mutation: leave the result unmarked; `a_write_through_a_cast_that_drops_const_is_unproven`'s
        // `through_a_global`, cast as it is read out of memory, goes silent.
        // **Carried, not declared**: the cast's own type is what a reader of
        // it declared, and asked at the operand's levels instead,
        // `unp((char **)&c)` was believed (#1158). See ADR-0123.
        if let Some(ty) = self.types.of(operand) {
            let levels = self.const_levels(ty);
            builder.function.mark_carried_levels(into, levels);
        }
        builder.push(Operation {
            place: Place::local(into),
            value: rvalue,
            origin: Origin::Written(span),
        });
        values.push(Operand::Copy(Place::local(into)));
        Some(())
    }

    fn temporary(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<LocalId> {
        let ty = self.ty_of(id, diagnostics)?;
        let local = builder.function.push_local(ty);
        if let Some(ty) = self.types.of(id) {
            let levels = self.const_levels(ty);
            builder.function.mark_const_levels(local, levels);
        }
        Some(local)
    }

    /// The `const` levels a pointer moved by arithmetic keeps: those of the
    /// local it was moved from, `G2 + 1` reading the rows of `const char
    /// *G2[2][2]` as `G2` does. The other operand is an integer and has none.
    /// Left unmarked, `G2[1][0] = "ab"` was reported as a write (#1144).
    /// Mutation: answer `0`; `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
    /// `into_a_const_static_array` reports its row.
    fn moved_levels<'o>(
        function: &Function,
        of: fn(&Function, LocalId) -> u32,
        operands: impl IntoIterator<Item = &'o Operand>,
    ) -> u32 {
        operands
            .into_iter()
            .filter_map(|operand| match operand {
                Operand::Copy(place) if place.steps().is_empty() => Some(of(function, place.local)),
                _ => None,
            })
            .fold(0, |levels, each| levels | each)
    }

    /// The levels at which `ty` points at `const`, bit `k` where what `k`
    /// dereferences of a value of it read points at an object whose type is
    /// `const` at its innermost element: `const int **` is bit `1`, `int
    /// *const *` bit `0`. See ADR-0121.
    ///
    /// **An array as the pointer it decays to**, a pointer to its element
    /// (C17 6.3.2.1 p3): `const char *GA[2]` is read through `GA[0]` as a
    /// `const char **` is, and read as no pointer it gave no levels, so a
    /// store of a literal into its element was reported as a write (#1144).
    /// Mutation: answer `0` for an array; `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
    /// `into_a_const_static_array` is reported.
    ///
    /// **Only the outermost array is a level.** An array a pointer points at
    /// sits at the address the pointer holds, so `T (*)[N]` is read in as
    /// many loads as `T *`, and its element is the next level's. Counted as
    /// a level of its own, every bit past it moved one level down and
    /// `char *const (*)[2]` said its `char` was `const` (#1144 review).
    /// Mutation: count every array as a level;
    /// `read_only_storage_below_what_leaves_or_arrives_is_unproven`'s
    /// `handed_rows_of_const_pointers` goes silent.
    fn const_levels(&self, ty: TypeId) -> u32 {
        let mut levels = 0;
        let mut ty = ty;
        for level in 0..u32::BITS {
            let pointee = match self.ast.ty(ty) {
                Type::Pointer(pointee) => *pointee,
                Type::Array { element, .. } if level == 0 => *element,
                _ => break,
            };
            if self.read_only(pointee) {
                levels |= 1 << level;
            }
            ty = self.innermost_element(pointee);
        }
        levels
    }

    /// `ty` with every array it is peeled off: the element an array of
    /// arrays holds at the bottom, and `ty` itself where it is no array.
    fn innermost_element(&self, ty: TypeId) -> TypeId {
        let mut ty = ty;
        while let Type::Array { element, .. } = self.ast.ty(ty) {
            ty = *element;
        }
        ty
    }

    /// A temporary of the type an operation is performed at.
    ///
    /// `int` where the place holds a number, because C17 6.3.1.1 p2 promotes
    /// every integer type this frontend has to it and 6.5.16.2 p3 makes `E1
    /// op= E2` mean `E1 = E1 op E2`, which puts the operation at the promoted
    /// type and the narrowing in the assignment. `types.rs` says the same
    /// about `a + b` and is where this stops being a constant: the day `long`
    /// parses, the promoted type of a pair is a question again, because C
    /// performs the operation at what the usual arithmetic conversions give
    /// the pair rather than at what promoting the left operand gives. `void`
    /// takes this arm to keep the `match` total and is not an answer about C:
    /// 6.5.16.2 p1 wants an arithmetic or a pointer left operand and `void` is
    /// neither, so `*v += 1` on a `void *` is `error[SC0306]` in `types.rs`,
    /// and the driver does not lower a tree that reported one.
    ///
    /// Without that, `c += 100` on a `char` writes its addition straight into
    /// an 8-bit place, and the interpreter reads that place's type as the width
    /// the operation happened at. C says 200 is an ordinary `int` there and the
    /// truncation to `char` is a conversion, so a run that stopped would be
    /// reporting a defined program as undefined. ADR-0013 rests on an
    /// operation's destination carrying the promoted type; this is what makes
    /// that true where no expression node does.
    ///
    /// **The place's own type where that is a pointer**, because C17 6.5.6 p8
    /// makes `p + i` a pointer rather than a number, and the obligation
    /// [`Operation`] states is to carry the type C performs the operation at,
    /// whichever type that is. What `int` costs here is not a wrong value:
    /// ADR-0030 has the memory check read a local's declared type to tell the
    /// pointer operand of an addition from the integer beside it, and drop the
    /// integer, so a temporary declared `int` holding `p + i` is an allocation
    /// that check can be handed and not see. Nothing was being lost while this
    /// answered `int`, because that temporary has one use and no expression
    /// this frontend builds puts it beside a pointer operand; the requirement
    /// was broken all the same, and the set of expressions grows every phase.
    /// `docs/c-family.md` is where it is written down, and what it costs.
    ///
    /// [`TranslationUnit::place_ty`] answers `None` for an `Index` of anything
    /// but an array, which the lowering builds only under `Rvalue::Address`, and
    /// for a `Deref` through something that is not a pointer. What excludes the
    /// second is not [`Lowering::begin_place`]'s own
    /// reading, which pushes a `Deref` without asking what it dereferences: it
    /// is the `typed` call that begins it, because `types.rs` gives `*x` no
    /// type where `x` is a number, so the expression is refused before a place
    /// exists. A fallback to `int` here would put the silence above back for a
    /// place nobody could name; a panic says which one.
    ///
    /// The whole place is asked about rather than its base local, because
    /// `*pp` on an `int **` is an `int *` and the operation happens at what
    /// the place holds.
    fn promoted(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        place: &Place,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<LocalId> {
        // An integer is computed at the type the type check worked out:
        // `E op E2`'s, the usual arithmetic conversions of the place's
        // promoted type and the other operand's (C17 6.5.16.2 p3).
        if let Some(at) = self.types.operated(id) {
            let at = self.ty(self.ast.expr(id).span(), at, diagnostics)?;
            return Some(builder.function.push_local(at));
        }
        let ty = self
            .unit
            .place_ty(&builder.function, place)
            .expect("a place this lowering built has a type");

        // Written out rather than spelled `matches!`, for the reason
        // `memory/transfer.rs::Allocations::is_pointer` gives: a kind of type
        // nobody has added yet is not a number, and `error[E0004]` here is what
        // asks a fourth kind whether an operation on it happens at its own
        // type.
        Some(match self.unit.ty(ty) {
            Ty::Pointer(_) => builder.function.push_local(ty),
            Ty::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            } => unreachable!("the type check records what an integer is computed at"),
            Ty::Void => {
                let int = self.unit.push_type(Ty::INT);
                builder.function.push_local(int)
            }
            // An array is no modifiable lvalue (C17 6.3.2.1 p1), so it is the
            // operand of no compound assignment and no `++` or `--`, which
            // `types.rs` refuses before a place exists.
            Ty::Array { .. } => unreachable!(
                "an array reached a compound assignment, `++` or `--`, which the type check refuses"
            ),
            // No arithmetic is done on a struct (C17 6.5.16.2 p1, 6.5.2.4
            // p1), which `types.rs` refuses before a place exists.
            Ty::Struct(_) => unreachable!(
                "a struct reached a compound assignment, `++` or `--`, which the type check refuses"
            ),
            // No place has a function's type: a function is no object.
            Ty::Function(_) => unreachable!("a place of a function's type, which nothing builds"),
        })
    }

    /// The local an identifier means.
    ///
    /// A name with no local is one the IR cannot reach: an object at file
    /// scope is the case that arrives first, because every place is rooted at a
    /// local and #74 is where that changes. A function used as a value is the
    /// other.
    fn local(&mut self, id: ExprId, diagnostics: &mut DiagnosticSink) -> Option<LocalId> {
        let span = self.ast.expr(id).span();
        let local = self.named(id);

        if local.is_none() {
            diagnostics.report(
                Diagnostic::error("cannot compile a use of this name yet")
                    .with_code(LOWERING)
                    .with_label(Label::primary(span, "this is not a local or a parameter"))
                    .with_note("an object declared outside a function is not supported so far"),
            );
        }

        local
    }

    /// The local an identifier means, or `None` with nothing said.
    ///
    /// [`Self::local`]'s question for a reader that is not lowering the
    /// identifier: a callee's name is a function and has no local, and
    /// whoever lowers it is the one who reports that.
    fn named(&self, id: ExprId) -> Option<LocalId> {
        self.resolution
            .resolved(id)
            .map(|binding| self.resolution.binding(binding).name)
            .and_then(|name| self.locals.get(&name).copied())
    }

    /// The local an expression is, where it is an identifier and nothing
    /// more: the operand a write by name lands in.
    fn named_by_itself(&self, id: ExprId) -> Option<LocalId> {
        match self.seen_through(id) {
            Some(inner) => self.named_by_itself(inner),
            None => match self.ast.expr(id) {
                Expr::Identifier { .. } => self.named(id),
                _ => None,
            },
        }
    }

    /// What `*&E` is, which is `E`, where `id` is that shape.
    fn seen_through(&self, id: ExprId) -> Option<ExprId> {
        let Expr::Unary {
            op: AstUnOp::Deref,
            operand,
            ..
        } = self.ast.expr(id)
        else {
            return None;
        };
        match self.ast.expr(*operand) {
            Expr::Unary {
                op: AstUnOp::AddrOf,
                operand: inner,
                ..
            } => Some(*inner),
            _ => None,
        }
    }

    /// Every named local one argument reads or writes, in source order, each
    /// with where and whether it is a write.
    ///
    /// **What C17 6.5 p2 asks about, and no more.** A local is accessed where
    /// its value is read or where it is assigned, incremented or decremented
    /// by name. Taking its address reads nothing (6.3.2.1 p2), so `&q` is no
    /// access, and neither is a write through a pointer, into a member or into
    /// an element: nothing here says which object those land in. A compound
    /// assignment reads as well as writes, and the write is what is kept. See
    /// ADR-0052.
    ///
    /// `as_place` starts the walk as a place, which is how a simple
    /// assignment's left operand is read: the local it names is the store's,
    /// which [`Self::unsequenced`] compares against the value's writes on its
    /// own, while what computing the place reads, `*q` or `q[i]`, is an
    /// access here.
    ///
    /// A worklist rather than recursion, as every walk of an expression here
    /// is, because an operand is as deep as its source.
    fn accesses(&self, operand: ExprId, as_place: bool) -> Vec<(LocalId, Span, bool)> {
        #[cfg(test)]
        ACCESS_WALKS.with(|walks| walks.set(walks.get() + 1));
        let mut found = Vec::new();
        // The expression, and whether it is read as a place rather than for
        // its value. Pushed in reverse, so they come off in source order.
        let mut work = vec![(operand, as_place)];
        while let Some((id, place)) = work.pop() {
            // **`*&E` is `E`**, because the lowering reads it so: `*&q = &x`
            // is a write of `q` by name in the IR, and walking it as a write
            // through a pointer left `h(*q, *&q = &x)` silent. See
            // `begin_place`.
            if let Some(inner) = self.seen_through(id) {
                work.push((inner, place));
                continue;
            }
            match self.ast.expr(id) {
                Expr::Identifier { span, .. } => {
                    if let (false, Some(local)) = (place, self.named(id)) {
                        found.push((local, *span, false));
                    }
                }
                Expr::Assign {
                    place: target,
                    value,
                    span,
                    ..
                } => {
                    if let Some(local) = self.named_by_itself(*target) {
                        found.push((local, *span, true));
                        work.push((*value, false));
                    } else {
                        work.push((*value, false));
                        work.push((*target, true));
                    }
                }
                Expr::Unary {
                    op, operand, span, ..
                } => match op {
                    AstUnOp::PreInc | AstUnOp::PreDec | AstUnOp::PostInc | AstUnOp::PostDec => {
                        if let Some(local) = self.named_by_itself(*operand) {
                            found.push((local, *span, true));
                        } else {
                            work.push((*operand, true));
                        }
                    }
                    AstUnOp::AddrOf => work.push((*operand, true)),
                    AstUnOp::Deref
                    | AstUnOp::Plus
                    | AstUnOp::Minus
                    | AstUnOp::Not
                    | AstUnOp::BitNot => work.push((*operand, false)),
                },
                Expr::Binary { lhs, rhs, .. } | Expr::Comma { lhs, rhs, .. } => {
                    work.push((*rhs, false));
                    work.push((*lhs, false));
                }
                Expr::Subscript { base, index, .. } => {
                    work.push((*index, false));
                    work.push((*base, false));
                }
                Expr::Conditional {
                    condition,
                    then,
                    otherwise,
                    ..
                } => {
                    work.push((*otherwise, false));
                    work.push((*then, false));
                    work.push((*condition, false));
                }
                Expr::Call {
                    callee, arguments, ..
                } => {
                    work.extend(arguments.iter().rev().map(|&argument| (argument, false)));
                    work.push((*callee, false));
                }
                Expr::Cast { operand, .. } => work.push((*operand, false)),
                // A member is refused before anything reads it, the other two
                // name nothing, and `sizeof` evaluates nothing.
                // A list is written in place, and is no pointer.
                Expr::VaArg { list, .. } => work.push((*list, true)),
                Expr::VaOp { operands, .. } => {
                    work.extend(operands.iter().rev().map(|&operand| (operand, true)));
                }
                Expr::SizeofType { .. }
                | Expr::SizeofExpr { .. }
                | Expr::Member { .. }
                | Expr::Number { .. }
                | Expr::Enumerator { .. }
                | Expr::Character { .. }
                | Expr::Length { .. }
                | Expr::String { .. }
                | Expr::Error { .. } => {}
            }
        }
        found
    }

    /// One [`Element::UnsequencedAccess`] for each pointer local one of this
    /// node's operands writes and another reads or writes: the first write,
    /// and the first access in another operand.
    ///
    /// The operands are a call's arguments, an assignment's place and value,
    /// a binary operator's two sides, or a subscript's base and index. A
    /// pair of accesses is found at the lowest node whose operands it lies
    /// across, since below that both are in one operand; an outer node can
    /// still report the same local over another pair, which is a second
    /// report rather than a missed one. A node that sequences never asks, and
    /// neither does one with a single operand.
    ///
    /// Only a pointer, because that is what the memory check is believed
    /// about; `k(i, i++)` is undefined by the same paragraph and is not
    /// reported, which `an_integer_another_argument_writes_is_not_reported`
    /// pins. See ADR-0052.
    ///
    /// On the tree, because the conflict is between the operands as written
    /// and not in the order they were lowered.
    fn unsequenced(&self, function: &Function, node: ExprId) -> Vec<Element> {
        // **No write below, nothing to report**: every element this builds
        // needs one operand to write by name, and an assignment is one
        // itself. Asked first, because walking each operand whole at every
        // node made `a + a + ... + a` quadratic: ten thousand terms took 27
        // seconds to lower, one test that was most of `safec-c`'s suite
        // (#1330). Mutation: ask every node;
        // `a_chain_with_no_write_walks_no_operand` fails.
        let operands: &[ExprId] = match self.ast.expr(node) {
            Expr::Binary { lhs, rhs, .. } => &[*lhs, *rhs],
            Expr::Subscript { base, index, .. } => &[*base, *index],
            Expr::Call { arguments, .. } => arguments,
            _ => &[],
        };
        if !matches!(self.ast.expr(node), Expr::Assign { .. })
            && !operands
                .iter()
                .any(|operand| self.writes_below[operand.index()])
        {
            return Vec::new();
        }
        let (each, between, at): (Vec<_>, Between, Span) = match self.ast.expr(node) {
            Expr::Call {
                arguments, span, ..
            } => (
                arguments
                    .iter()
                    .map(|&argument| self.accesses(argument, false))
                    .collect(),
                Between::Arguments,
                *span,
            ),
            // A compound assignment reads its place's value as well, `E1 =
            // E1 op (E2)` by C17 6.5.16.2 p3, so its place is walked as a
            // value and the local it names is a read. Found by review.
            Expr::Assign {
                op,
                place,
                value,
                span,
            } => (
                vec![
                    self.accesses(*place, op.is_none()),
                    self.accesses(*value, false),
                ],
                Between::Operands,
                *span,
            ),
            Expr::Binary { lhs, rhs, span, .. } => (
                vec![self.accesses(*lhs, false), self.accesses(*rhs, false)],
                Between::Operands,
                *span,
            ),
            Expr::Subscript {
                base, index, span, ..
            } => (
                vec![self.accesses(*base, false), self.accesses(*index, false)],
                Between::Operands,
                *span,
            ),
            // Only those four push the task; nothing, rather than a panic, as
            // `discard` answers. Written out rather than `_`, so a kind added
            // later is asked whether its operands are unsequenced.
            Expr::Number { .. }
            | Expr::Enumerator { .. }
            | Expr::Character { .. }
            | Expr::Length { .. }
            | Expr::String { .. }
            | Expr::Identifier { .. }
            | Expr::Unary { .. }
            | Expr::Conditional { .. }
            | Expr::Member { .. }
            | Expr::Comma { .. }
            | Expr::SizeofType { .. }
            | Expr::SizeofExpr { .. }
            | Expr::Cast { .. }
            | Expr::VaOp { .. }
            | Expr::VaArg { .. }
            | Expr::Error { .. } => return Vec::new(),
        };
        let mut said: Vec<LocalId> = Vec::new();
        let mut elements = self.conflicts(function, &each, between, at, &mut said);
        // **The store is a write too.** C17 6.5.16 p3 sequences it after the
        // value computations of both operands and nothing more, so a write
        // to the same local in the value is unsequenced against it unless a
        // sequence point inside the value puts that write first: `q = q++`
        // and `q = (q = s)` are undefined by 6.5 p2, `q = (q = r, q)` and `q
        // = g(q++)` are not. Found by review; see ADR-0052.
        let store = match self.ast.expr(node) {
            Expr::Assign {
                op: _,
                place,
                value,
                ..
            } => self
                .named_by_itself(*place)
                .map(|local| (local, *place, *value)),
            _ => None,
        };
        if let Some((local, place, value)) = store {
            let pointer = match self.unit.ty(function.local(local)) {
                Ty::Pointer(_) => true,
                // No whole array is written, so none is a pointer written
                // twice; nor is a function, which no local is.
                Ty::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Ty::Void
                | Ty::Array { .. }
                | Ty::Struct(_)
                | Ty::Function(_) => false,
            };
            let unordered = self
                .unordered_writes(value)
                .into_iter()
                .find(|&(other, _)| other == local);
            if let (true, false, Some((_, written))) = (pointer, said.contains(&local), unordered) {
                elements.push(Element::UnsequencedAccess {
                    local,
                    written,
                    accessed: self.ast.expr(place).span(),
                    both_write: true,
                    between,
                    origin: Origin::Written(at),
                });
            }
        }
        elements
    }

    /// One [`Element::UnsequencedAccess`] for each pointer local one list of
    /// accesses writes and another list reads or writes, each list being one
    /// operand C leaves unsequenced against the others.
    ///
    /// `said` is every local already reported for these operands, and gains
    /// each one this reports, so [`Self::unsequenced`]'s store check does not
    /// report it again. Shared with [`Self::lengths`], whose lengths are
    /// unsequenced against each other just as a call's arguments are (#529).
    fn conflicts(
        &self,
        function: &Function,
        each: &[Vec<(LocalId, Span, bool)>],
        between: Between,
        at: Span,
        said: &mut Vec<LocalId>,
    ) -> Vec<Element> {
        let mut elements = Vec::new();
        for (i, accesses) in each.iter().enumerate() {
            for &(local, written, write) in accesses {
                // Written out rather than spelled `matches!`, for the reason
                // `promoted` gives.
                let pointer = match self.unit.ty(function.local(local)) {
                    Ty::Pointer(_) => true,
                    Ty::Integer {
                        rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                        sign: _,
                    }
                    | Ty::Void
                    | Ty::Array { .. }
                    | Ty::Struct(_)
                    | Ty::Function(_) => false,
                };
                if !write || !pointer || said.contains(&local) {
                    continue;
                }
                let other = each
                    .iter()
                    .enumerate()
                    .filter(|&(j, _)| j != i)
                    .flat_map(|(_, accesses)| accesses)
                    .find(|&&(other, _, _)| other == local);
                if let Some(&(_, accessed, both_write)) = other {
                    said.push(local);
                    elements.push(Element::UnsequencedAccess {
                        local,
                        written,
                        accessed,
                        both_write,
                        between,
                        origin: Origin::Written(at),
                    });
                }
            }
        }
        elements
    }

    /// Every write by name in an assignment's value that no sequence point
    /// inside the value puts before the value is computed.
    ///
    /// A write is ordered first where it sits in the left operand of `,`,
    /// `&&` or `||`, in the condition of `?:`, or in a call's arguments or
    /// callee, each of which C17 sequences before what follows (6.5.17 p2,
    /// 6.5.13 p4, 6.5.14 p4, 6.5.15 p4, 6.5.2.2 p10). Anywhere else it is
    /// unsequenced against the store. **Wrong in the direction of too few** is
    /// a silence, so anything not listed counts as unordered. See ADR-0052.
    fn unordered_writes(&self, value: ExprId) -> Vec<(LocalId, Span)> {
        let mut found = Vec::new();
        // The expression, and whether a sequence point inside the value
        // already puts it before the value's computation.
        let mut work = vec![(value, false)];
        while let Some((id, ordered)) = work.pop() {
            if let Some(inner) = self.seen_through(id) {
                work.push((inner, ordered));
                continue;
            }
            match self.ast.expr(id) {
                Expr::Assign {
                    place, value, span, ..
                } => {
                    if let (false, Some(local)) = (ordered, self.named_by_itself(*place)) {
                        found.push((local, *span));
                    }
                    work.push((*value, ordered));
                    work.push((*place, ordered));
                }
                Expr::Unary {
                    op, operand, span, ..
                } => {
                    let writes = matches!(
                        op,
                        AstUnOp::PreInc | AstUnOp::PreDec | AstUnOp::PostInc | AstUnOp::PostDec
                    );
                    if let (true, false, Some(local)) =
                        (writes, ordered, self.named_by_itself(*operand))
                    {
                        found.push((local, *span));
                    }
                    work.push((*operand, ordered));
                }
                Expr::Comma { lhs, rhs, .. } => {
                    work.push((*rhs, ordered));
                    work.push((*lhs, true));
                }
                Expr::Binary { op, lhs, rhs, .. } => {
                    let sequences = matches!(op, AstBinOp::LogAnd | AstBinOp::LogOr);
                    work.push((*rhs, ordered));
                    work.push((*lhs, ordered || sequences));
                }
                Expr::Conditional {
                    condition,
                    then,
                    otherwise,
                    ..
                } => {
                    work.push((*otherwise, ordered));
                    work.push((*then, ordered));
                    work.push((*condition, true));
                }
                Expr::Call {
                    callee, arguments, ..
                } => {
                    work.extend(arguments.iter().map(|&argument| (argument, true)));
                    work.push((*callee, true));
                }
                Expr::Subscript { base, index, .. } => {
                    work.push((*index, ordered));
                    work.push((*base, ordered));
                }
                Expr::Cast { operand, .. } => work.push((*operand, ordered)),
                // Its operand is not evaluated, so it writes nothing.
                Expr::SizeofType { .. } | Expr::SizeofExpr { .. } => {}
                // A list is written, and is no pointer an assignment stores.
                Expr::VaOp { .. } | Expr::VaArg { .. } => {}
                Expr::Identifier { .. }
                | Expr::Number { .. }
                | Expr::Enumerator { .. }
                | Expr::Character { .. }
                | Expr::Length { .. }
                | Expr::String { .. }
                | Expr::Member { .. }
                | Expr::Error { .. } => {}
            }
        }
        found
    }

    /// The function an expression designates, where it designates one: a
    /// function's name, or `*` or `&` of one. Anything else a call calls is
    /// a pointer, called through (#1212). See ADR-0125.
    fn designated(&self, id: ExprId) -> Option<FuncId> {
        match self.ast.expr(id) {
            // **Not a name a local or a static shadows**: the table is
            // keyed by spelling, and a local `f` beside a function `f` is
            // the local.
            Expr::Identifier { .. } => {
                if self.named(id).is_some() || self.static_named(id).is_some() {
                    return None;
                }
                self.resolution
                    .resolved(id)
                    .map(|binding| self.resolution.binding(binding).name)
                    .and_then(|name| self.functions.get(self.sources.snippet(name)).copied())
            }
            // `*f` and `&f` of a designator are that designator's function
            // (C17 6.5.3.2 p3 and p4), so `(*f)()` is a direct call.
            Expr::Unary {
                op: AstUnOp::Deref | AstUnOp::AddrOf,
                operand,
                ..
            } => self.designated(*operand),
            _ => None,
        }
    }

    /// Whether `id` is the name of a function whose declaration was refused,
    /// which is no local, no static and no function the unit holds, or `*`
    /// or `&` of one, as [`Self::designated`] reads them.
    ///
    /// Mutation: drop the `static_named` test; `a_static_named_like_a_refused_function_is_called`
    /// fails, the call dropped with its caller's body.
    fn names_a_refused_function(&self, id: ExprId) -> bool {
        match self.ast.expr(id) {
            Expr::Identifier { span, .. } => {
                self.named(id).is_none()
                    && self.static_named(id).is_none()
                    && self.refused.contains(self.sources.snippet(*span))
            }
            Expr::Unary {
                op: AstUnOp::Deref | AstUnOp::AddrOf,
                operand,
                ..
            } => self.names_a_refused_function(*operand),
            _ => false,
        }
    }

    /// Whether the expression `id` has a function type: a designator, or
    /// `*` of a pointer to a function.
    fn is_function(&self, id: ExprId) -> bool {
        self.types
            .of(id)
            .is_some_and(|ty| matches!(self.ast.ty(ty), Type::Function { .. }))
    }

    /// How many parameters the function `callee` calls declares, where its
    /// prototype ends in `, ...`: the arguments from there on take no
    /// declared type. `None` for any other callee.
    fn variadic_after(&self, callee: ExprId) -> Option<usize> {
        let mut ty = self.types.of(callee)?;
        if let Type::Pointer(pointee) = self.ast.ty(ty) {
            ty = *pointee;
        }
        match self.ast.ty(ty) {
            Type::Function {
                parameters: Parameters::Prototype(parameters),
                variadic: true,
                ..
            } => Some(parameters.len()),
            _ => None,
        }
    }

    /// Whether the default argument promotions change the type of `id`, a
    /// `char` or a `short` (C17 6.5.2.2 p6).
    fn is_promoted_by_default(&self, id: ExprId) -> bool {
        self.types
            .of(id)
            .is_some_and(|ty| self.ast.is_promoted_by_default(ty))
    }

    /// Whether what `called` calls returns a struct by value, which it then
    /// writes where its first argument says: a function's own record, or
    /// the signature of what a pointer called through points at (#1212).
    fn returns_a_struct(&self, function: &Function, called: &Called) -> bool {
        match called {
            Called::Direct(id) => self.unit.function(*id).returns_a_struct(),
            Called::Through(Operand::Copy(place)) => self
                .unit
                .place_ty(function, place)
                .and_then(|ty| match self.unit.ty(ty) {
                    Ty::Pointer(pointee) => match self.unit.ty(pointee) {
                        Ty::Function(signature) => Some(signature),
                        _ => None,
                    },
                    _ => None,
                })
                .is_some_and(|signature| {
                    self.unit
                        .signature(signature)
                        .parameters
                        .first()
                        .map(|&(_, passes)| passes)
                        == Some(Passes::StructReturn)
                }),
            Called::Through(Operand::Constant(_) | Operand::Function(_)) | Called::Intrinsic(_) => {
                false
            }
        }
    }

    /// Whether a call to `called` is one C says does not return.
    ///
    /// Decided by name alone. A unit that defines one of these never reaches
    /// here, because names refuses the definition, so whatever defines it
    /// elsewhere is a program C does not define and is not asked about. See
    /// ADR-0051.
    ///
    /// A call through a pointer is believed to return, as the checks believe
    /// it: nothing here names which function it is.
    fn does_not_return(&self, called: &Called) -> bool {
        let Some(called) = called.direct() else {
            return false;
        };
        let name = self.unit.function(called).spelled(self.sources);
        DOES_NOT_RETURN
            .iter()
            .any(|&(reserved, _)| reserved == name)
    }
}

#[cfg(test)]
thread_local! {
    /// How many times [`Lowering::accesses`] has walked an operand on this
    /// thread, so a test can say a walk was skipped rather than time it.
    pub(crate) static ACCESS_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether each expression of `ast`, or anything below it, assigns,
/// increments or decrements, indexed by [`ExprId::index`].
///
/// **One pass in id order**, since a node is pushed only after the nodes it
/// holds (`Ast::push_expr` takes them as ids), so each child's answer is
/// there before its parent asks. Over every child, which is all
/// [`Lowering::accesses`] walks and more, so a write it finds there is one
/// this finds: one this counts and that does not makes a node asked rather
/// than skipped, which costs time and changes no answer.
///
/// Mutation: count no write; sixteen cases reporting an unsequenced access
/// go silent, `a_dereference_in_an_argument_another_argument_writes_is_reported`
/// among them.
fn writes_below(ast: &Ast) -> Vec<bool> {
    let mut writes: Vec<bool> = Vec::new();
    for id in ast.expr_ids() {
        let below = |ids: &[ExprId]| ids.iter().any(|child| writes[child.index()]);
        let written = match ast.expr(id) {
            // Each writes a list.
            Expr::Assign { .. } | Expr::VaOp { .. } | Expr::VaArg { .. } => true,
            Expr::Unary { op, operand, .. } => {
                matches!(
                    op,
                    AstUnOp::PreInc | AstUnOp::PreDec | AstUnOp::PostInc | AstUnOp::PostDec
                ) || below(&[*operand])
            }
            Expr::Binary { lhs, rhs, .. } | Expr::Comma { lhs, rhs, .. } => below(&[*lhs, *rhs]),
            Expr::Subscript { base, index, .. } => below(&[*base, *index]),
            Expr::Conditional {
                condition,
                then,
                otherwise,
                ..
            } => below(&[*condition, *then, *otherwise]),
            Expr::Call {
                callee, arguments, ..
            } => below(&[*callee]) || below(arguments),
            Expr::Member { base, .. } => below(&[*base]),
            Expr::SizeofExpr { operand, .. } | Expr::Cast { operand, .. } => below(&[*operand]),
            Expr::Number { .. }
            | Expr::Character { .. }
            | Expr::String { .. }
            | Expr::Length { .. }
            | Expr::Identifier { .. }
            | Expr::Enumerator { .. }
            | Expr::SizeofType { .. }
            | Expr::Error { .. } => false,
        };
        writes.push(written);
    }
    writes
}

/// Whether an operand is non-zero, as a value.
///
/// C's truth values are 1 and 0 rather than whatever decided them: 6.5.13 p3
/// and 6.5.14 p3 say `&&` and `||` yield one or the other, so an answer copied
/// from the operand that decided it would make `3 && 5` five. The IR has no
/// truth of its own, so the comparison is the operation that says it.
fn truth(operand: Operand) -> Rvalue {
    Rvalue::Binary {
        op: BinOp::Ne,
        lhs: operand,
        rhs: Operand::Constant(0),
    }
}

/// The IR operator an AST operator means, where one exists.
///
/// `LogAnd` and `LogOr` have none, and that is deliberate: they are branches,
/// which is what the doc comment on `ir::BinOp` says and what `split` builds.
fn binary(op: AstBinOp) -> Option<BinOp> {
    Some(match op {
        AstBinOp::Mul => BinOp::Mul,
        AstBinOp::Div => BinOp::Div,
        AstBinOp::Rem => BinOp::Rem,
        AstBinOp::Add => BinOp::Add,
        AstBinOp::Sub => BinOp::Sub,
        AstBinOp::Shl => BinOp::Shl,
        AstBinOp::Shr => BinOp::Shr,
        AstBinOp::Lt => BinOp::Lt,
        AstBinOp::Gt => BinOp::Gt,
        AstBinOp::Le => BinOp::Le,
        AstBinOp::Ge => BinOp::Ge,
        AstBinOp::Eq => BinOp::Eq,
        AstBinOp::Ne => BinOp::Ne,
        AstBinOp::BitAnd => BinOp::BitAnd,
        AstBinOp::BitXor => BinOp::BitXor,
        AstBinOp::BitOr => BinOp::BitOr,
        AstBinOp::LogAnd | AstBinOp::LogOr => return None,
    })
}

#[cfg(test)]
mod tests;
