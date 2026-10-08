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
//! cannot read, a call with no function to name, and a second definition of
//! one name. One code rather than six, for the reason `types.rs` gives for
//! `MISMATCH`: what differs between them is the message, and a reader
//! filtering on the code wants to know the IR could not be built rather than a
//! list of ways that can happen.
//!
//! `parser.rs` splits its two codes along a different line, and the difference
//! is worth naming: `TOO_DEEP` is separate from `EXPECTED` because a program
//! nested too deeply is well formed and refused, while an unexpected token is
//! a program nobody wrote correctly. Every case here is the first kind, so the
//! split has nothing to divide.

use std::collections::{HashMap, HashSet};

use crate::ast::{Ast, BinOp as AstBinOp, Expr, ExprId, Item, Parameters, Stmt, StmtId, Type};
use crate::ast::{Nullability, Specifier, TypeId, UnOp as AstUnOp, spell_type};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::sema::Resolution;
use crate::types::Types;
use safec_ir::ir::{
    BinOp, Block, BlockId, Element, FuncId, Function, LocalId, Operand, Operation, Origin,
    Parameter, Place, Projection, Promise, Rvalue, Terminator, TranslationUnit, Ty, TyId, UnOp,
};
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

/// Something the frontend accepted and this stage cannot express.
///
/// One code for every shape of it, for the reason `types.rs` gives for
/// `MISMATCH`: what differs between them is the message, and a reader filtering
/// on the code wants "the IR could not be built" rather than a list of ways
/// that can happen.
const LOWERING: Code = Code::new("SC0304");

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
         specifiers, because a caller is checked against the declaration it sees",
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
/// `constant` says of the expression as written. `(x, 1)` lowers to the
/// constant its right operand is, and is not a constant expression: C17 6.6 p3
/// forbids a comma operator in one, and 6.8.5 p6 lets an implementation assume
/// a loop on an expression that is not one terminates, so `while ((x, 1)) {}`
/// is not `while (1) {}` and is left a `Branch`. `1 == 1` and `!0` are
/// constant expressions this frontend does not evaluate yet, so they stay a
/// `Branch` and a false report rather than a guess. The arm not taken is still
/// lowered, into blocks nothing reaches.
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
        functions: HashMap::new(),
        prototypes: HashMap::new(),
        return_nullability: HashMap::new(),
        refused: HashSet::new(),
        defined: HashSet::new(),
        pending: HashMap::new(),
        top_level: false,
        root: None,
    };

    lowering.declare(diagnostics);
    lowering.define(diagnostics);
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
    /// The compound statements that are open, innermost last, each holding the
    /// locals it declared directly.
    ///
    /// The function's body is the first, so a scope narrower than the function
    /// is one at index 1 or beyond. Only those get storage markers: a local
    /// declared in the body itself lives exactly as long as the frame, which
    /// every consumer already knows, and ADR-0012 argues why that is enough.
    scopes: Vec<Vec<LocalId>>,
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
    /// The names whose signature this stage could not read.
    ///
    /// Reported once, where the declaration is. A call to one of them is not
    /// reported again: the caller wrote an ordinary call and the fault is in a
    /// declaration somewhere else.
    refused: HashSet<String>,
    /// The names this translation unit defines a function for.
    ///
    /// Asked of this rather than of the IR, because bodies are lowered in file
    /// order and a function defined below a call to it has no body in the IR
    /// yet when the call is lowered. Read by [`Lowering::does_not_return`].
    defined: HashSet<String>,
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
    /// The full expression being lowered, which is the expression
    /// [`Lowering::value`] was last asked for.
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
        if place.projection.is_empty() {
            return;
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
    /// Pushed only where nothing unsequenced encloses the call, which is why
    /// the flag is read rather than the task always being pushed: see
    /// [`Lowering::begin_value`].
    ///
    /// [`Lowering::begin_value`]: Lowering::begin_value
    Arguments(Span),
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
        | Expr::Identifier { .. }
        | Expr::Unary { .. }
        | Expr::Assign { .. }
        | Expr::Call { .. }
        | Expr::Subscript { .. }
        | Expr::Error { .. } => false,
    }
}

impl Lowering<'_> {
    /// Give every function a [`FuncId`] before any body is built.
    ///
    /// A call names its callee by id, and a callee can be defined after its
    /// caller or be the caller itself, so the ids come first and the bodies
    /// second. Until a body arrives the function is a declaration, which is
    /// what it truthfully is.
    fn declare(&mut self, diagnostics: &mut DiagnosticSink) {
        for item in self.ast.items() {
            match item {
                Item::Function(function) => {
                    let (name, ty, written) =
                        (function.name, function.ty, function.return_nullability);
                    self.defined.insert(self.sources.snippet(name).to_owned());
                    self.declare_one(name, ty, written, true, diagnostics);
                }
                Item::Declaration { declarators, .. } => {
                    for declarator in declarators {
                        // A declaration of an object rather than a function is
                        // not in the IR at all: every place is rooted at a
                        // local, so there is nothing for a global to be. A use
                        // of one is reported where it is used, which is where a
                        // reader can see what it cost.
                        //
                        // **`declarator.init` is read and dropped here**, so
                        // `int g = 5;` at file scope emits nothing and says
                        // nothing. That is contained only because a use of `g`
                        // is refused: there is no way to observe the value
                        // that went missing. #85 is where a global gets
                        // somewhere to live, and it is the change that has to
                        // come back for this initializer.
                        let declaration = &declarator.declaration;
                        if let Some(name) = declaration.name {
                            let (ty, written) = (declaration.ty, declaration.nullability);
                            self.declare_one(name, ty, written, false, diagnostics);
                        }
                    }
                }
                Item::Error { .. } => {}
            }
        }
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
        // diagnostic about the callee is pointed.
        if !self.functions.contains_key(self.sources.snippet(name)) {
            let mut declared = Function::declaration_with_parameters(name, returns, lowered);
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

    /// Refuse a prototype of a function that disagrees with the first prototype
    /// of it about a parameter's nullability specifier, or record this one as
    /// the first. See [`DISAGREEING_ANNOTATION`].
    ///
    /// Position by position, over as many parameters as both have. A
    /// declaration with a different count is a different disagreement, which is
    /// #57's and is not answered here.
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
        let returns = self.ty(name, returns, diagnostics)?;
        let mut lowered = Vec::new();
        if let Parameters::Prototype(parameters) = parameters {
            for parameter in parameters {
                let at = parameter.name.unwrap_or(parameter.span);
                if !self.adjusted_without_loss(parameter.written) {
                    diagnostics.report(
                        Diagnostic::error("cannot compile a parameter declared as this array yet")
                            .with_code(LOWERING)
                            .with_label(Label::primary(at, "declared here"))
                            .with_note(
                                "this is a gap in this compiler rather than a fault in the program",
                            ),
                    );
                    return None;
                }
                lowered.push(Parameter {
                    ty: self.ty(at, parameter.ty, diagnostics)?,
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
                });
            }
        }

        Some((returns, lowered))
    }

    /// Whether a parameter written as `written` loses nothing by being the
    /// pointer C17 6.7.6.3 p7 adjusts it to.
    ///
    /// The parser adjusts every array parameter, and `Declaration::ty` is
    /// what this stage lowers. What the adjustment throws away is the
    /// array's own declarator: its length, which 6.9.1 p10 evaluates on entry
    /// to the function where it is not a constant, so `int a[(free(p), 1)]`
    /// frees `p` and `int a[*p = 1]` writes through it. So only `T[]` and
    /// `T[N]` with `N` a positive number, of an element that is neither
    /// `void` nor a function, are lowered, and anything else is refused as
    /// it was before parameters were adjusted, rather than lowered with its
    /// length's effects gone. The element and a literal length are
    /// `types.rs`'s to refuse first, under C17 6.7.6.2 p1, and this is a
    /// defence for them; a length that is not a literal is what reaches
    /// here, `[-1]` among them until #384, and evaluating one that is not a
    /// constant on entry is #382.
    fn adjusted_without_loss(&self, written: TypeId) -> bool {
        let mut current = written;
        let mut array = false;
        while let Type::Array { element, length } = self.ast.ty(current) {
            array = true;
            if let Some(length) = *length {
                let positive = self.constant_expression(length)
                    && self.types.value(length).is_some_and(|value| value > 0);
                if !positive {
                    return false;
                }
            }
            current = *element;
        }
        !array || !matches!(self.ast.ty(current), Type::Void | Type::Function { .. })
    }

    /// Whether an expression is one of the constant expressions [`decided`]
    /// folds: an integer constant, or one under unary `+`.
    ///
    /// Not every constant expression (C17 6.6), only those this frontend can
    /// already lower to an `Operand::Constant`, and never one with a comma
    /// operator in it, which 6.6 p3 forbids and which lowers to a constant
    /// anyway. Every expression kind written out, so that one added later is
    /// answered for here rather than folded or not by default.
    fn constant_expression(&self, expr: ExprId) -> bool {
        match self.ast.expr(expr) {
            Expr::Number { span: _ } => true,
            Expr::Unary {
                op: AstUnOp::Plus,
                operand,
                span: _,
            } => self.constant_expression(*operand),
            _ => false,
        }
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
            // C17 6.9 p5 allows one external definition of a name and this
            // compiler does not check it yet, so a second one arrives here
            // rather than being reported before it. The first body is kept,
            // because replacing it would leave every call that was lowered
            // against it pointing at another function's blocks.
            if self.unit.function(id).is_defined() {
                diagnostics.report(
                    Diagnostic::error(format!(
                        "`{}` is defined more than once",
                        self.sources.snippet(name)
                    ))
                    .with_code(LOWERING)
                    .with_label(Label::primary(name, "this definition is not used"))
                    .with_label(Label::secondary(
                        self.unit.function(id).name,
                        "the first one is here",
                    )),
                );
                continue;
            }
            let Some(built) = self.body(name, ty, body, hatch, written, diagnostics) else {
                continue;
            };

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
        } = self.ast.ty(ty)
        else {
            return None;
        };
        let (returns, parameters) = (*returns, parameters.clone());
        let (returns, lowered) = self.signature(name, returns, &parameters, diagnostics)?;
        let mut function = Function::with_parameters(name, returns, lowered);
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
        if let Parameters::Prototype(parameters) = &parameters {
            for (parameter, local) in parameters.iter().zip(function.parameters()) {
                if let Some(at) = parameter.name {
                    self.locals.insert(at, local);
                }
            }
        }

        let mut builder = Builder {
            function,
            block: None,
            elements: Vec::new(),
        };
        builder.open();

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
        if self.sources.snippet(name) == "main" && self.unit.ty(returns) == Ty::Int {
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

        Some(builder.function)
    }

    /// The IR type of a type the frontend wrote.
    ///
    /// A loop rather than a recursion, following `ast::spell_type`: the chain
    /// is bounded by the parser, and a walk over a type is the shape that stops
    /// being bounded first when something else builds one.
    fn ty(&mut self, at: Span, id: TypeId, diagnostics: &mut DiagnosticSink) -> Option<TyId> {
        let mut pointers = 0usize;
        let mut current = id;

        let base = loop {
            match self.ast.ty(current) {
                Type::Int => break Ty::Int,
                Type::Char => break Ty::Char,
                Type::Void => break Ty::Void,
                Type::Pointer(pointee) => {
                    pointers += 1;
                    current = *pointee;
                }
                // An array is not a pointer and saying it is would tell the
                // memory analysis that one object is another. Nothing has
                // asked the IR to hold either an array or a function yet, and
                // #74, which is the issue for what it cannot say, is about
                // storage duration rather than about these.
                Type::Array { .. } | Type::Function { .. } => {
                    diagnostics.report(
                        Diagnostic::error(format!(
                            "cannot compile something of type `{}` yet",
                            spell_type(self.sources, self.ast, id)
                        ))
                        .with_code(LOWERING)
                        .with_label(Label::primary(at, "declared here"))
                        .with_note("`int`, `char`, `void` and pointers to them are all this compiler holds so far"),
                    );
                    return None;
                }
            }
        };

        let mut ty = self.unit.push_type(base);
        for _ in 0..pointers {
            ty = self.unit.push_type(Ty::Pointer(ty));
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
            // on one problem, and its note would be false for `123abc`, which
            // is the program's fault and not this compiler's. Every
            // expression *not* in the list is a gap nobody has reported yet,
            // which is what the message says.
            //
            // The driver does not lower a tree the type check reported
            // about, so no run reaches the list today. It is kept as the
            // defence it was: were that gate to open, a constant's error
            // would not gain a second caret and a false note.
            if !matches!(self.ast.expr(id), Expr::Number { .. }) {
                diagnostics.report(
                    Diagnostic::error("cannot compile an expression whose type is not known")
                        .with_code(LOWERING)
                        .with_label(Label::primary(
                            span,
                            "nothing worked out what type this has",
                        ))
                        .with_note(
                            "this is a gap in this compiler rather than a fault in the program",
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
        // always an `int`: `Expr::Number` is the one thing that builds one and
        // `types.rs` gives it `int`. A cast or a string literal would break
        // that, and `((int *)0)[n]` would fold to `n`, which is a wrong
        // **value** rather than a wrong report. Neither is implemented; when
        // one is, this decides by the operand's type instead.
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
                self.scopes.push(Vec::new());

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
                if builder.reachable() {
                    // Reverse order of declaration, which is the order a C++
                    // destructor would run in and costs nothing to get right
                    // while the list is being written.
                    for &local in declared.iter().rev() {
                        builder.element(Element::StorageDead {
                            // Nobody wrote "end this storage": the `}` is what
                            // it exists because of, which is what `Generated`
                            // means and what stops a diagnostic quoting a
                            // block of source back as if a user had asked for
                            // this. The last byte of a compound statement is
                            // its `}`, and the lowering only ever sees a tree
                            // that parsed without a word said about it.
                            origin: Origin::Generated(Span::new(
                                span.file(),
                                span.end() - 1,
                                span.end(),
                            )),
                            local,
                        });
                    }
                }
                lowered?;
            }
            Stmt::Return { value, span } => {
                let (value, span) = (*value, *span);
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
                // own body is the first scope and holds nothing, so no depth
                // test is needed. See ADR-0012.
                for scope in self.scopes.iter().rev() {
                    for &local in scope.iter().rev() {
                        builder.element(Element::StorageDead {
                            origin: Origin::Generated(span),
                            local,
                        });
                    }
                }
                builder.end(Terminator::Return);
            }
            Stmt::Declaration { declarators, .. } => {
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
                        let ty = self.ty(span, ty, diagnostics)?;
                        let local = builder.function.push_local(ty);
                        self.locals.insert(name, local);

                        // Only a scope narrower than the function's own body. The
                        // first entry is the body, and ADR-0012 says why a local
                        // that lives as long as the frame needs no marker.
                        if self.scopes.len() > 1 {
                            let scope = self.scopes.last_mut().expect("a scope is open");
                            scope.push(local);
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
                            let operand = self.value(builder, init, diagnostics)?;
                            builder.push(Operation {
                                place: Place::local(local),
                                value: Rvalue::Use(operand),
                                origin: Origin::Written(Span::new(
                                    name.file(),
                                    name.start(),
                                    span.end(),
                                )),
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
                    Origin::Written(asked),
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
                let constant = self.constant_expression(condition);
                let condition = self.value(builder, condition, diagnostics)?;
                let inside = builder.function.reserve_block();
                let after = builder.function.reserve_block();
                builder.end(decided(
                    condition,
                    constant,
                    inside,
                    after,
                    Origin::Written(asked),
                ));

                builder.enter(inside, Some(asked));
                self.stmt(builder, body, diagnostics)?;
                if builder.reachable() {
                    builder.end(Terminator::Goto(header));
                }

                builder.enter(after, Some(asked));
            }
            Stmt::For {
                initialiser,
                condition,
                step,
                body,
                ..
            } => {
                let (initialiser, condition, step, body) = (*initialiser, *condition, *step, *body);
                if let Some(initialiser) = initialiser {
                    self.effect(builder, initialiser, diagnostics)?;
                    builder.sequenced(self.ast.expr(initialiser).span());
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
                        let asked = asked.expect("a condition carries a span");
                        let constant = self.constant_expression(condition);
                        let condition = self.value(builder, condition, diagnostics)?;
                        builder.end(decided(
                            condition,
                            constant,
                            inside,
                            after,
                            Origin::Written(asked),
                        ));
                    }
                    // 6.8.5.3 p2: an absent condition is replaced by a non-zero
                    // constant, so the loop has no exit edge of its own.
                    None => builder.end(Terminator::Goto(inside)),
                }

                builder.enter(inside, asked);
                self.stmt(builder, body, diagnostics)?;
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
            }
            // The driver hands this stage a tree nothing reported about, so a
            // node the parser gave up on cannot be here. Reporting it would be
            // a second diagnostic about the first one's problem, which is what
            // the per-input gate exists to stop.
            Stmt::Error { .. } => {}
        }

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
            self.walk(builder, root, diagnostics)?
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
        if let Some(value) = self.walk(builder, root, diagnostics)? {
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
    fn walk(
        &mut self,
        builder: &mut Builder,
        root: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<Option<Operand>> {
        // Set here rather than restored, so that an expression abandoned
        // half-way by a type error cannot leave the next one reading a flag
        // the walk that set it never finished with. Every caller of this is a
        // full expression: Annex C's list and `lower_stmt`'s arms are the same
        // eight places.
        self.top_level = true;
        self.root = Some(root);

        let mut tasks = vec![Task::Value(root)];
        let mut values: Vec<Operand> = Vec::new();
        let mut places: Vec<Place> = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Value(id) => {
                    self.begin_value(id, &mut tasks, &mut values, diagnostics)?;
                }
                Task::Place(id) => {
                    self.begin_place(id, &mut tasks, &mut places, diagnostics)?;
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
                Task::Restore(top_level) => self.top_level = top_level,
            }
        }

        let value = values.pop();
        assert!(
            values.is_empty(),
            "every value an expression pushed was read: {values:?}"
        );
        Some(value)
    }

    /// Whether lowering `id` leaves a value on the walk's stack.
    ///
    /// A call to a `void` function and a `void` `?:` leave none, and a comma
    /// leaves what its right operand leaves. Everything else leaves one,
    /// `void` included: `*p` of a `void *` is a read of a place, and a
    /// consumer that dropped it for its type would drop the read, which is
    /// what the memory and nullability checks ask about.
    fn pushes(&self, mut id: ExprId) -> bool {
        // A loop rather than a recursion, because a comma's right operand
        // can be a comma as deep as the parser lets parentheses go.
        loop {
            match self.ast.expr(id) {
                Expr::Call { .. } | Expr::Conditional { .. } => return !self.is_void(id),
                Expr::Comma { rhs, .. } => id = *rhs,
                _ => return true,
            }
        }
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
    fn descend(&mut self, id: ExprId, tasks: &mut Vec<Task>) {
        if self.top_level && !sequences(self.ast.expr(id)) {
            tasks.push(Task::Restore(true));
            self.top_level = false;
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

        // Before [`Lowering::descend`], which is what takes the flag away for
        // this node's own operands. What the call arm below asks is whether
        // anything unsequenced encloses **the call**, and asking after the
        // descent answers about its arguments instead, which is always no.
        let at_root = self.top_level;
        self.descend(id, tasks);

        match self.ast.expr(id) {
            Expr::Number { .. } => {
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
            Expr::Identifier { .. } | Expr::Subscript { .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Place(id));
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
                    tasks.push(Task::Value(*rhs));
                    tasks.push(Task::Value(*lhs));
                }
            }
            Expr::Assign { place, value, .. } => {
                tasks.push(Task::Finish(id));
                tasks.push(Task::Value(*value));
                tasks.push(Task::Place(*place));
            }
            Expr::Conditional { condition, .. } => {
                tasks.push(Task::Split(id));
                tasks.push(Task::Value(*condition));
            }
            Expr::Call {
                arguments, span, ..
            } => {
                let span = *span;
                tasks.push(Task::Finish(id));
                // C17 6.5.2.2 p10, first sentence: a sequence point after the
                // arguments and before the call. Popped after every argument
                // and before the `Finish` that builds the terminator, which is
                // where C puts it.
                //
                // **Only where nothing unsequenced encloses the call.** The
                // marker is a claim about the whole block, and a block holds
                // the other operands of whatever encloses this: emitting one
                // for `*p + (free(p), 0)` would order the read against a free
                // C leaves it unordered against. That is ADR-0022's enclosure
                // rule, asked about the call rather than about the point, and
                // it is the same rule the four operators already ask. See
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
                for &argument in arguments.iter().rev() {
                    tasks.push(Task::Value(argument));
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
        id: ExprId,
        tasks: &mut Vec<Task>,
        places: &mut Vec<Place>,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<()> {
        self.typed(id, diagnostics)?;

        self.descend(id, tasks);

        match self.ast.expr(id) {
            Expr::Identifier { .. } => {
                places.push(Place::local(self.local(id, diagnostics)?));
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
                tasks.push(Task::Value(*index));
                tasks.push(Task::Value(*base));
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
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Assign { .. }
            | Expr::Conditional { .. }
            | Expr::Call { .. }
            | Expr::Comma { .. }
            | Expr::Error { .. } => {
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
            // A constant is its own value, so nothing asks it to finish. The
            // arm is here because the match is written out: an expression kind
            // added later has to say what it does rather than fall through.
            Expr::Number { .. } => {}
            Expr::Identifier { .. } | Expr::Subscript { .. } => {
                let place = places.pop().expect("a place");
                values.push(Operand::Copy(place));
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
                        values.push(Operand::Copy(place));
                    }
                    AstUnOp::AddrOf => {
                        let mut place = places.pop().expect("a place");
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
                        if matches!(place.projection.last(), Some(Projection::Deref)) {
                            place.projection.pop();
                            values.push(Operand::Copy(place));
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
                        let stepped = self.promoted(builder, &place);
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
                        let into = builder.function.push_local(ty);
                        builder.push(Operation {
                            place: Place::local(into),
                            value: Rvalue::Binary { op, lhs, rhs },
                            origin: Origin::Written(span),
                        });
                        values.push(Operand::Copy(Place::local(into)));
                    }
                }
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
                        let computed = self.promoted(builder, &place);
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
                let (callee, count, span) = (*callee, arguments.len(), *span);
                let at = values.len() - count;
                let arguments: Vec<Operand> = values.split_off(at);
                let called = self.callee(callee, diagnostics)?;
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
                let returns = self.root != Some(id) || !self.does_not_return(called);
                builder.end(Terminator::Call {
                    callee: called,
                    arguments,
                    destination: into.map(Place::local),
                    then: returns.then_some(then),
                    origin: Origin::Written(span),
                });
                builder.switch(then);
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
            Expr::Unary { .. } => {
                let operand = values.pop().expect("a pointer");
                let mut place = self.pointed_at(id, operand, diagnostics)?;
                place.projection.push(Projection::Deref);
                places.push(place);
            }
            // C17 6.5.2.1 p2 defines `E1[E2]` as `(*((E1)+(E2)))`, and this
            // builds exactly that: one C expression is one shape in the IR, so
            // an analysis asking what an access reaches has one thing to read
            // rather than two spellings of it.
            //
            // `Projection::Index` is what an array wants and nothing builds one
            // yet, because an array is a type this stage refuses. Whoever gives
            // the IR arrays decides whether a subscript on one is an `Index`.
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
                // `x[p - q]`, asking its type below is what reports it. Of the pointer's type rather than the
                // subscript's: what is worked out here is the address, and the
                // subscript is what that address reaches.
                let base_ty = self.ty_of(base, diagnostics)?;
                let (base, pointer, offset, addressed) =
                    if matches!(self.unit.ty(base_ty), Ty::Pointer(_)) {
                        (base, pointer, offset, base_ty)
                    } else {
                        (index, offset, pointer, self.ty_of(index, diagnostics)?)
                    };

                // `E[0]` is `*E`, so it is built as `*E` is: one C expression
                // is one shape in the IR, which the sentence above this arm has
                // asked for since it was written. See ADR-0021.
                match Self::unmoved(self.unit.ty(addressed), BinOp::Add, &pointer, &offset) {
                    Some(pointer) => {
                        let mut place = self.pointed_at(base, pointer, diagnostics)?;
                        place.projection.push(Projection::Deref);
                        places.push(place);
                    }
                    None => {
                        let addressed = builder.function.push_local(addressed);
                        builder.push(Operation {
                            place: Place::local(addressed),
                            value: Rvalue::Binary {
                                op: BinOp::Add,
                                lhs: pointer,
                                rhs: offset,
                            },
                            origin: Origin::Written(span),
                        });
                        places.push(Place {
                            local: addressed,
                            projection: vec![Projection::Deref],
                        });
                    }
                }
            }
            // Nothing else schedules a `FinishPlace`, and the kinds are written
            // out rather than wildcarded because the cost of being wrong is
            // paid elsewhere: an arm that pushed no place would leave the next
            // `places.pop()` taking an outer expression's place instead.
            Expr::Number { .. }
            | Expr::Identifier { .. }
            | Expr::Binary { .. }
            | Expr::Assign { .. }
            | Expr::Conditional { .. }
            | Expr::Call { .. }
            | Expr::Comma { .. }
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
            Expr::Binary { op, lhs, rhs, span } => {
                let (op, lhs, rhs, span) = (*op, *lhs, *rhs, *span);
                let answer = answer.expect("a `&&` or `||` is an `int`");
                // The left operand decides the answer unless the right is
                // reached, so it is written before the branch and overwritten
                // after. What is written is whether it is non-zero and not
                // what it is: C17 6.5.13 p3 and 6.5.14 p3 say `&&` and `||`
                // yield 1 or 0, so `3 && 5` is 1 rather than 5.
                builder.push(Operation {
                    place: Place::local(answer),
                    value: truth(condition),
                    origin: Origin::Written(span),
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
                    origin: Origin::Written(self.ast.expr(lhs).span()),
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
                    origin: Origin::Written(self.ast.expr(asked).span()),
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
            | Expr::Identifier { .. }
            | Expr::Unary { .. }
            | Expr::Assign { .. }
            | Expr::Call { .. }
            | Expr::Subscript { .. }
            | Expr::Comma { .. }
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
    /// **The other half of a comma is not this precise, and that is #147.**
    /// `i, *p;` is discarded by the statement, which has the whole comma in
    /// hand and nothing narrower, so it underlines `i, *p`. Two reviewers
    /// raised it as one thing: a span that covers more than the sub-expression
    /// that mattered, which is the same defect #147 records for a controlling
    /// expression and is settled for all of them there.
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
        let value = match self.ast.expr(id) {
            Expr::Binary { .. } => truth(value),
            _ => Rvalue::Use(value),
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
    fn temporary(
        &mut self,
        builder: &mut Builder,
        id: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<LocalId> {
        let ty = self.ty_of(id, diagnostics)?;
        Some(builder.function.push_local(ty))
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
    /// [`TranslationUnit::place_ty`] answers `None` for an `Index` projection,
    /// which nothing builds, and for a `Deref` through something that is not a
    /// pointer. What excludes the second is not [`Lowering::begin_place`]'s own
    /// reading, which pushes a `Deref` without asking what it dereferences: it
    /// is the `typed` call that begins it, because `types.rs` gives `*x` no
    /// type where `x` is a number, so the expression is refused before a place
    /// exists. A fallback to `int` here would put the silence above back for a
    /// place nobody could name; a panic says which one.
    ///
    /// The whole place is asked about rather than its base local, because
    /// `*pp` on an `int **` is an `int *` and the operation happens at what
    /// the place holds.
    fn promoted(&mut self, builder: &mut Builder, place: &Place) -> LocalId {
        let ty = self
            .unit
            .place_ty(&builder.function, place)
            .expect("a place this lowering built has a type");

        // Written out rather than spelled `matches!`, for the reason
        // `memory/transfer.rs::Allocations::is_pointer` gives: a kind of type
        // nobody has added yet is not a number, and `error[E0004]` here is what
        // asks a fourth kind whether an operation on it happens at its own
        // type.
        match self.unit.ty(ty) {
            Ty::Pointer(_) => builder.function.push_local(ty),
            Ty::Int | Ty::Char | Ty::Void => {
                let int = self.unit.push_type(Ty::Int);
                builder.function.push_local(int)
            }
        }
    }

    /// The local an identifier means.
    ///
    /// A name with no local is one the IR cannot reach: an object at file
    /// scope is the case that arrives first, because every place is rooted at a
    /// local and #74 is where that changes. A function used as a value is the
    /// other.
    fn local(&mut self, id: ExprId, diagnostics: &mut DiagnosticSink) -> Option<LocalId> {
        let span = self.ast.expr(id).span();
        let local = self
            .resolution
            .resolved(id)
            .map(|binding| self.resolution.binding(binding).name)
            .and_then(|name| self.locals.get(&name).copied());

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

    /// The function a call names.
    ///
    /// Nothing reaches the report below today, and the path that would is worth
    /// naming: a callee this stage cannot resolve is a pointer to a function,
    /// and a pointer to a function is a type [`Lowering::ty`] refuses where it
    /// is declared, so the function holding the call is already refused by
    /// then. The report is here because the day `Ty` grows a function type is
    /// the day this becomes reachable, and a `None` returned in silence would
    /// be a function dropped with nothing said.
    fn callee(&mut self, callee: ExprId, diagnostics: &mut DiagnosticSink) -> Option<FuncId> {
        let span = self.ast.expr(callee).span();
        let named = self
            .resolution
            .resolved(callee)
            .map(|binding| self.resolution.binding(binding).name)
            .and_then(|name| self.functions.get(self.sources.snippet(name)).copied());

        if named.is_none() && !self.refused.contains(self.sources.snippet(span)) {
            diagnostics.report(
                Diagnostic::error("cannot compile this call yet")
                    .with_code(LOWERING)
                    .with_label(Label::primary(
                        span,
                        "this is not a function this compiler found",
                    ))
                    .with_note("a call through a function pointer is not supported so far"),
            );
        }

        named
    }

    /// Whether a call to `called` is one C says does not return.
    ///
    /// Decided by name, and only for a function this translation unit does
    /// not define: one defined here is lowered like any other, and its body is
    /// what the checks read rather than its name. See ADR-0051.
    fn does_not_return(&self, called: FuncId) -> bool {
        let name = self.sources.snippet(self.unit.function(called).name);
        DOES_NOT_RETURN.contains(&name) && !self.defined.contains(name)
    }
}

/// Four of the library functions C says do not return to their caller.
///
/// `abort` (C17 7.22.4.1), `exit` (7.22.4.4), `_Exit` (7.22.4.5) and
/// `quick_exit` (7.22.4.7). `longjmp` (7.13.2.1) and `thrd_exit` (7.26.5.5)
/// do not return either: the first takes a `jmp_buf`, which this frontend
/// cannot declare, and the second belongs to the threads that
/// `docs/roadmap.md` places in Phase 8. The name is believed because 7.1.3 reserves it: a
/// program that defines one with external linkage has no behaviour C
/// defines. [`Lowering::does_not_return`] answers `false` for one defined
/// here all the same, so its body is what the checks read.
const DOES_NOT_RETURN: &[&str] = &["abort", "exit", "_Exit", "quick_exit"];

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
