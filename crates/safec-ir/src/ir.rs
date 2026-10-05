//! The Safety IR: what every analysis walks and every backend reads.
//!
//! `docs/architecture.md` calls this the central architectural boundary. Above
//! it is one frontend and below it are the analyses, the interpreter and
//! eventually LLVM, and the point of the boundary is that neither side has to
//! know the other: `docs/c-family.md` argues that a Clang adapter must be able
//! to reach this layer without the C frontend existing, and #73 is the change
//! that makes cargo enforce it.
//!
//! **An operation writes to a place; it does not define a value.** That is the
//! shape `docs/safety-model.md` asks for by naming Value and Place as two
//! things, and it is what the analyses this project exists to write are about:
//! "is this place still allocated" and "who is responsible for it" are
//! questions about places. The other shape, where every operation defines a
//! fresh value and memory is reached only through loads and stores, is better
//! for a backend and worse here, because a variable written in one block and
//! read after a loop's back edge stops being one thing.
//!
//! **A block ends with a terminator, and a call is one.** Control leaves a
//! function abnormally where a callee is entered, so a call that can take an
//! edge no statement produced has to be the thing that ends a block. See
//! [ADR-0010].
//!
//! **A block says where a local's storage began and ended.** `{ int x; p = &x;
//! }` and the same program without the braces are two different IRs, because
//! the first carries an [`Element::StorageDead`] the second does not, and an
//! analysis can therefore say what a pointer outlived rather than only where it
//! came from. See [ADR-0012].
//!
//! **Every place is still rooted at a local**, so an object with static storage
//! duration cannot be named at all. That is the other half of what a lifetime
//! analysis eventually needs, it changes what a [`Place`] is rooted at rather
//! than adding a variant beside the ones here, and #85 is where it is decided.
//! It waits on the frontend: the parser does not read the storage classes, so
//! no C program can ask for one yet.
//!
//! Nothing here reads an IR. `crates/safec/src/lowering.rs` builds one from
//! the typed AST; the printer is [`crate::print`] and the interpreter is
//! [`crate::interp`], and what this module owes them is a shape they do not
//! have to agree about first.
//!
//! [ADR-0010]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0010-give-the-graph-an-edge-no-statement-produced.md
//! [ADR-0012]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0012-say-a-local-s-storage-began-and-ended-in-the-block.md

use std::collections::HashMap;

use crate::source::Span;
use crate::target::{Integer, Target};

/// Which function, within one [`TranslationUnit`].
///
/// An opaque id and not a name, which `docs/c-family.md` asks for and gives the
/// reason: two `static` functions in different translation units share a name
/// and are different functions, and a C++ mangled name is an ABI detail. It is
/// also what "the unit of analysis is the instantiation" asks not to be
/// blocked by, because two functions from one source range are two ids
/// carrying one [`Function::name`]. `docs/c-family.md` says that one can wait
/// and costs nothing to accommodate later, which is a weaker claim than
/// having it: what is here is the keying, not the instantiation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FuncId(u32);

/// Which type, within one [`TranslationUnit`].
///
/// Two of these are equal exactly when the types are the same, because
/// [`TranslationUnit::push_type`] hands back the id it already has for a type
/// it already holds. That is why [`Ty`] may derive `PartialEq` where the
/// frontend's `ast::Type` refuses to: this one reaches the rest of itself
/// through ids that are themselves unique.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TyId(u32);

/// Which block, within one [`Function`].
///
/// Meaningless in another function, the way an index into one arena is
/// meaningless in another. See ADR-0008, which is the same decision one layer
/// up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(u32);

/// Which local, within one [`Function`].
///
/// [`Ord`] for the reason [`Place`] gives, because a place is ordered by this
/// first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(u32);

impl FuncId {
    /// The index this handle refers to.
    ///
    /// For a side table with a slot per function, which is what an analysis
    /// keeps when it summarises one: whether a parameter is borrowed or taken
    /// is a fact about a function that its callers read.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl LocalId {
    /// The index this handle refers to.
    ///
    /// For a side table with a slot per local, which is what a dataflow
    /// analysis is. `FileId::index` in `crates/safec/src/source.rs` is the same
    /// thing several layers up.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl BlockId {
    /// The index this handle refers to.
    ///
    /// For a side table with a slot per block, which is what a fixpoint over a
    /// control-flow graph keeps.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A type, as the IR holds one.
///
/// The IR's own and not the frontend's, because the crate that holds this must
/// not depend on the crate that parses C: `docs/c-family.md` calls that arrow
/// the whole boundary, and an adapter for another language reaches this type
/// rather than `ast::Type`.
///
/// **No widths here, and that is the decision rather than a gap.** What an
/// `Int` is worth is asked of the unit, which carries the target:
/// [`TranslationUnit::integer`]. Putting it in the type instead would dissolve
/// `Int` and `Char` into one integer kind with a width and a sign, and `--emit
/// safety-ir` would stop saying which one a program wrote. See ADR-0013, which
/// rejected exactly that and says what would reverse it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// `int`.
    Int,
    /// `char`.
    Char,
    /// `void`, which is what a function returns when it returns nothing.
    Void,
    /// A pointer to the type this id names.
    Pointer(TyId),
}

/// A step from a local towards the thing an operation means.
///
/// `docs/safety-model.md`'s Place is a local plus however many of these it
/// takes to reach what is being read or written: `*p` is one [`Deref`], and
/// `a[i]` is one [`Index`]. A field selector joins them when structs do.
///
/// [`Ord`] for the reason [`Place`] gives.
///
/// [`Deref`]: Projection::Deref
/// [`Index`]: Projection::Index
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Projection {
    /// What the pointer points at.
    Deref,
    /// The element this operand selects.
    Index(Operand),
}

/// Somewhere a value can be read from or written to.
///
/// The thing the safety analyses are about. "Is this place still allocated" and
/// "has this place been moved out of" are the questions
/// `docs/safety-model.md`'s memory and ownership axes ask, and neither is a
/// question about a value.
///
/// A place with a projection is not the place it starts from: `p` and `*p` are
/// two places, and an analysis that treated them as one would say a pointer is
/// live when what it points at is not.
///
/// [`Eq`] and [`Hash`] because a dataflow analysis keys its lattice on a place
/// rather than on a local: `p` and `*p` have separate states and a side table
/// indexed by [`LocalId`] has one slot for both.
///
/// **[`Ord`] so that a lattice value holding one can be canonical, and for no
/// other reason.** [`crate::dataflow::Analysis::Value`] decides whether the
/// walk has ended by comparing, so a value that holds the same facts in two
/// arrangements never compares equal and never converges, which ADR-0016
/// measured as a hang. A set of places is kept sorted to stop that, and this is
/// what sorts it. One place is not more or less than another in any sense the
/// language has, and nothing should read the order as meaning anything.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Place {
    /// Where it starts.
    pub local: LocalId,
    /// How to get from there to what is meant. Empty is the local itself.
    pub projection: Vec<Projection>,
}

impl Projection {
    /// What this kind of step is called, for `--emit safety-ir`.
    ///
    /// The same shape as `ast::Expr::name` and for the same reason: the
    /// artifact is an interface, so a spelling is decided in one place rather
    /// than written out wherever something is printed.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Deref => "Deref",
            Self::Index(_) => "Index",
        }
    }
}

impl Operand {
    /// What this kind of operand is called, for `--emit safety-ir`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Copy(_) => "Copy",
            Self::Constant(_) => "Constant",
        }
    }
}

impl Place {
    /// The local itself, with nothing between.
    pub fn local(local: LocalId) -> Self {
        Self {
            local,
            projection: Vec::new(),
        }
    }
}

/// What an operation reads.
///
/// [`Ord`] for the reason [`Place`] gives, because a projection can hold one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operand {
    /// The value held in a place.
    ///
    /// One name for what C's lvalue conversion does, and not yet two: a move
    /// and a copy are the same thing until ownership exists to tell them apart,
    /// and `docs/roadmap.md` puts that in Phase 7. The variant is named for
    /// what it does today.
    Copy(Place),
    /// A constant, as the source wrote it.
    ///
    /// Wide enough for every integer constant C can write, and not narrowed to
    /// a target's width, for the reason [`Ty`] gives.
    Constant(i128),
}

/// An operator with one operand.
///
/// `+` is not here: C17 6.5.3.3 p2 makes unary `+` the promoted value of its
/// operand, which is [`Rvalue::Use`]. Neither are `++` and `--`, which are an
/// operation that writes back to the place they read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `-`
    Neg,
    /// `!`
    Not,
    /// `~`
    BitNot,
}

impl UnOp {
    /// What this operator is called, for `--emit safety-ir`.
    ///
    /// The IR's name for it rather than C's spelling. `--emit ast` prints what
    /// the source wrote, because that is what a tree is; this artifact is the
    /// IR's own, and an operator here means what the IR says it means whatever
    /// language reached it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Neg => "Neg",
            Self::Not => "Not",
            Self::BitNot => "BitNot",
        }
    }
}

/// An operator with two operands.
///
/// `&&` and `||` are not here, and their absence is the point: C17 6.5.13 p4
/// and 6.5.14 p4 make them short-circuit, so they are control flow and become
/// blocks and a [`Terminator::Branch`]. An IR that kept them as operators would
/// hide an edge from every analysis that walks the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `&`
    BitAnd,
    /// `^`
    BitXor,
    /// `|`
    BitOr,
}

impl BinOp {
    /// What this operator is called, for `--emit safety-ir`.
    ///
    /// See [`UnOp::name`] for why these are the IR's names rather than C's
    /// spellings.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mul => "Mul",
            Self::Div => "Div",
            Self::Rem => "Rem",
            Self::Add => "Add",
            Self::Sub => "Sub",
            Self::Shl => "Shl",
            Self::Shr => "Shr",
            Self::Lt => "Lt",
            Self::Gt => "Gt",
            Self::Le => "Le",
            Self::Ge => "Ge",
            Self::Eq => "Eq",
            Self::Ne => "Ne",
            Self::BitAnd => "BitAnd",
            Self::BitXor => "BitXor",
            Self::BitOr => "BitOr",
        }
    }
}

/// What an operation computes.
#[derive(Clone, Debug, PartialEq)]
pub enum Rvalue {
    /// The operand itself.
    Use(Operand),
    /// One operand under an operator.
    Unary {
        /// Which operator.
        op: UnOp,
        /// What it applies to.
        operand: Operand,
    },
    /// Two operands under an operator.
    ///
    /// **A well-formed safety IR does not add or subtract a literal zero from
    /// a pointer here.** `E[0]` and `*E` are one C expression, and an analysis
    /// that met them as two shapes would answer differently about one program;
    /// the fold belongs to whoever builds the IR, which is ADR-0021. Nothing
    /// checks it at this boundary, so `docs/c-family.md` carries what a
    /// frontend owes. The memory check follows an unfolded zero as an offset
    /// that may be zero, which
    /// `an_unfolded_zero_offset_is_followed_as_an_offset_that_may_be_zero` in
    /// `crates/safec-ir/tests/freed.rs` holds.
    Binary {
        /// Which operator.
        op: BinOp,
        /// The operand on the left.
        lhs: Operand,
        /// The operand on the right.
        rhs: Operand,
    },
    /// The address of a place.
    ///
    /// The one operation that turns a place into a value, which is why the
    /// lifetime analysis will start here: everything that can outlive what it
    /// points at came through this.
    Address(Place),
}

impl Rvalue {
    /// What this kind of value is called, for `--emit safety-ir`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Use(_) => "Use",
            Self::Unary { .. } => "Unary",
            Self::Binary { .. } => "Binary",
            Self::Address(_) => "Address",
        }
    }
}

/// Whether the source wrote an operation, or something else caused it.
///
/// `docs/c-family.md` calls this attribution, and `Span`'s own doc comment says
/// it expects a third coordinate for where code came from, which implicit
/// operations are named as one consumer of. If that coordinate arrives and
/// wants this name, this is the type that gives way: what it distinguishes is
/// narrower, and a span's origin is the more obvious reading of the word.
///
/// Both carry a span, because both have somewhere to point. What differs is
/// what a diagnostic may say: text that a user wrote can be quoted back, and a
/// destructor at the end of a scope has, in `docs/roadmap.md`'s words, "a
/// location to blame and no source text". The variant names are that
/// document's too: it asks attribution to distinguish "written here" from
/// "generated, caused by this".
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Origin {
    /// The source wrote this, here.
    Written(Span),
    /// Nobody wrote this. It exists because of what is at this span.
    Generated(Span),
}

impl Origin {
    /// What this kind of origin is called, for `--emit safety-ir`.
    ///
    /// Lower case, because it is a word about the operation on the same line
    /// rather than the name of a thing: `Operation t.c:2:5 generated` reads as
    /// a sentence and `Operation t.c:2:5 Generated` reads as two nouns.
    pub fn name(self) -> &'static str {
        match self {
            Self::Written(_) => "written",
            Self::Generated(_) => "generated",
        }
    }

    /// Where to point, whichever kind it is.
    pub fn span(self) -> Span {
        match self {
            Self::Written(span) | Self::Generated(span) => span,
        }
    }
}

/// One step of a block: something written, something evaluated and thrown
/// away, or storage beginning or ending.
///
/// Not every step writes a value. An automatic object's storage begins when
/// control enters the block it belongs to and ends when that block does, and
/// C17 6.2.4 p2 says the difference matters: "if an object is referred to
/// outside of its lifetime, the behavior is undefined". Without a step that
/// says so, `{ int x; p = &x; }` and the same program without the braces are
/// one IR, and the analysis that is supposed to tell them apart has the same
/// material for both. See [ADR-0012].
///
/// Only a local whose scope is narrower than its function gets a pair. A
/// parameter and a local declared in the function's own body live exactly as
/// long as the frame, which every consumer already knows.
///
/// [ADR-0012]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0012-say-a-local-s-storage-began-and-ended-in-the-block.md
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    /// A place, and what is written into it.
    Assign(Operation),
    /// A place evaluated for its side effects, whose value nobody wanted.
    ///
    /// C17 6.8.3 p2 evaluates an expression statement as a void expression and
    /// 6.3.2.2 discards what it yields. **What it does not do is skip the
    /// evaluation**, and the evaluation is the whole of what this records:
    /// 6.5.3.2 p4 makes the unary `*` undefined for an invalid pointer, with a
    /// footnote naming an address after the end of its object's lifetime among
    /// them, and nothing about that turns on whether anybody wanted the result.
    ///
    /// **Not an [`Self::Assign`] into a temporary**, which would need no new
    /// kind and no consumer changes at all. An earlier draft argued that C
    /// performs no read here; that is wrong, and a review caught it. 6.3.2.1
    /// p2 lists the contexts where an lvalue is not converted and a void
    /// expression is not among them, and `clang -O0` emits the load, measured.
    /// What this kind buys is that the reaching is separable from the reading,
    /// so a consumer can answer for the reaching alone. The interpreter is what
    /// spends that: its slots model "undefined to read" rather than
    /// "unspecified value", so reading `int x; int *q = &x; *q;` would stop a
    /// program C defines.
    ///
    /// **Only where the place goes through a projection**, which is a rule
    /// about what is worth carrying rather than about what C defines.
    /// Evaluating `p` on its own can be undefined too, by 6.3.2.1 p2's last
    /// sentence where the object is uninitialised and its address was never
    /// taken; no check here would read an element saying so, and building one
    /// anyway re-blesses a third of the corpus's artifacts for a line nothing
    /// asks about.
    Evaluate {
        /// What was evaluated.
        place: Place,
        /// Where the expression that was evaluated is written.
        origin: Origin,
    },
    /// This local has storage from here on, and nothing in it.
    ///
    /// C17 6.2.4 p6 begins the lifetime at entry into the block rather than at
    /// the declaration, and ends it when "execution of that block ends in any
    /// way", so each iteration of a loop is a fresh lifetime and this is what
    /// the second one starts from.
    StorageLive {
        /// Whose storage.
        local: LocalId,
        /// Where the declaration that asked for it is written.
        origin: Origin,
    },
    /// This local's storage is gone from here on.
    StorageDead {
        /// The scope that is ending, named by its own span.
        ///
        /// Which starts at the `{`, so this points there rather than at the
        /// `}` where the storage actually goes. The two coincide for saying
        /// *which* scope ended and differ for saying *where*, and the first
        /// diagnostic that has to say where is what should change it: the
        /// closing brace is not on the tree today, so pointing at it means
        /// giving `ast::Stmt::Compound` a second span rather than doing
        /// arithmetic on this one.
        origin: Origin,
        /// Whose storage.
        local: LocalId,
    },
    /// Everything before this is sequenced before everything after it.
    ///
    /// **A block's element list is a total order and C gives a partial one.**
    /// The order two elements appear in says which one this frontend chose to
    /// emit first, and that is not the same claim as C17 6.5 p3's, which leaves
    /// the operands of most operators unsequenced. This is where the two
    /// coincide: nothing before it can happen after anything following it.
    ///
    /// C17 Annex C is the complete list of sequence points and ADR-0022 is
    /// which of them this is emitted for and why the enclosure rule is the
    /// whole of the difficulty. **A frontend that emits one where C gives none
    /// hands every analysis a proof the standard does not license**, which is
    /// the failure `docs/safety-model.md` is written to prevent, so the bias
    /// when building one is towards emitting fewer.
    ///
    /// **It says what is ordered and not what is unordered.** A consumer
    /// walking forwards learns that everything behind this is sequenced before
    /// everything ahead of it; it learns nothing about two things it has not
    /// reached yet. Answering "are these two unsequenced" is therefore the
    /// consumer's to arrange: the memory check carries what it has read since
    /// the last one of these forwards to meet whatever frees it, which is
    /// ADR-0023. This element is what bounds how far.
    ///
    /// **So a frontend that emits too few is louder rather than quieter, in
    /// both directions.** One missing where C gives one leaves a free
    /// unsequenced, which is a suspicion where there would have been a proof,
    /// and it leaves a read carried further forward than it should be, which is
    /// a suspicion where there would have been nothing. Neither is a claim
    /// about safety that the standard does not license, which is the bias the
    /// paragraph above asks for.
    Sequenced {
        /// The expression this point falls **after**, rather than the operator
        /// that put it there.
        ///
        /// `free(p), *p = 42;` answers the span of `free(p)` and not the
        /// comma's, because the tree has no span for an operator token: an
        /// `Expr::Comma` carries one span covering both operands and the comma
        /// between them. Two markers in one block can therefore print the same
        /// position and mean different points, which the artifact for
        /// `a_comma_sequences_a_free_before_a_use` shows. Whoever wants a
        /// `sequenced here` label is who has to add the operator's own span.
        origin: Origin,
    },
    /// Every argument of the call that follows has been evaluated.
    ///
    /// C17 6.5.2.2 p10, first sentence: "There is a sequence point after the
    /// evaluations of the function designator and the actual arguments but
    /// before the actual call." ADR-0022 expressed that point by putting the
    /// argument operations before the call terminator, which is enough for a
    /// consumer that looks backwards and nothing at all for one that carries a
    /// fact forwards. [ADR-0026] is why it is an element now.
    ///
    /// Emitted where no unsequenced operator encloses the **call**, which is
    /// ADR-0022's enclosure rule asked about the call rather than about the
    /// point: a call's arguments are unsequenced against each other, so a call
    /// below a `+` gets none of these and its argument reads stay open.
    ///
    /// **Half of what [`Self::Sequenced`] says, and the half is the design.**
    /// A read behind this is ordered before everything that follows. A *free*
    /// behind this is not concluded to be ordered against anything, because a
    /// call carries reads in its own operands and a consumer judges those after
    /// this element, while C puts them before it: concluding the other half
    /// made `g((free(p), 0), *p)` a proved use after free, about two arguments
    /// C leaves unsequenced.
    ///
    /// **Nothing loses by the omission**, because anything in the same full
    /// expression that can follow one of these is separated from it by a
    /// [`Self::Sequenced`]: the operators that keep a call eligible for this
    /// element are exactly the four that emit one. `free(p), *p = 42;` is the
    /// case to read rather than a call at the root, because there the free is a
    /// comma's left operand and the write does follow it, and the comma's own
    /// marker is what proves it.
    ///
    /// So a consumer that answers this element by doing nothing is this
    /// compiler before it existed: a suspicion where C licensed silence, which
    /// is the direction [`Self::Sequenced`]'s own last paragraph asks for.
    ///
    /// [ADR-0026]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0026-say-that-a-call-s-arguments-have-been-evaluated.md
    ArgumentsEvaluated {
        /// The call whose arguments these were, named by its own span.
        origin: Origin,
    },
}

impl Element {
    /// What this kind of element is called, for `--emit safety-ir`.
    ///
    /// `Assign` answers `"Operation"`, which is what the artifact called it
    /// before there was anything else in the list. A word nobody had to change
    /// is a corpus expectation nobody had to re-bless.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Assign(_) => "Operation",
            Self::Evaluate { .. } => "Evaluate",
            Self::StorageLive { .. } => "StorageLive",
            Self::StorageDead { .. } => "StorageDead",
            Self::Sequenced { .. } => "Sequenced",
            Self::ArgumentsEvaluated { .. } => "ArgumentsEvaluated",
        }
    }
}

/// One step: a place, and what is written into it.
///
/// **An arithmetic operation's destination carries the type C performs it at.**
/// Whoever builds one owes that: C17 6.3.1.1 p2 promotes the operands of an
/// arithmetic operator, so `char + char` happens at `int` and the narrowing
/// back to a `char` is a separate assignment under 6.3.1.3. An `Operation`
/// whose `value` is a [`Rvalue::Binary`] or a [`Rvalue::Unary`] and whose
/// `place` is narrower says the arithmetic happened at the narrow width, and a
/// consumer that believes it calls a defined program undefined. The interpreter
/// is that consumer today and a backend is the next one: LLVM needs the
/// operation's own type to emit it at all.
///
/// A frontend that lowers `c += 1` as one operation into `c` breaks this, which
/// is what `safec`'s did until it was measured. The fix is what C says the
/// program is: compute into a temporary of the promoted type, then assign.
/// `lowering.rs::promoted` is where that is done and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    /// What is written to.
    pub place: Place,
    /// What is computed.
    pub value: Rvalue,
    /// Where it came from.
    pub origin: Origin,
}

/// How a block ends, and where control goes from it.
///
/// **A call is here rather than among the operations**, because control can
/// leave a function where a callee is entered: a `longjmp` returns somewhere
/// else and an exception unwinds, and both happen at a call. An operation sits
/// in the middle of a block's list, and a block cannot say that an edge leaves
/// from the middle of it. See [ADR-0010], which is also where [`Abnormal`] is
/// argued.
///
/// [ADR-0010]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0010-give-the-graph-an-edge-no-statement-produced.md
/// [`Abnormal`]: Terminator::Abnormal
#[derive(Clone, Debug, PartialEq)]
pub enum Terminator {
    /// Straight on.
    Goto(BlockId),
    /// One way or the other.
    Branch {
        /// What decides.
        condition: Operand,
        /// Where a non-zero condition goes.
        then: BlockId,
        /// Where a zero condition goes.
        otherwise: BlockId,
        /// Where the controlling expression is, so that a diagnostic can point
        /// at it.
        ///
        /// **The controlling expression, not the statement it belongs to.**
        /// `condition` can be a place read through a pointer, and then it is
        /// the only place in a block that a check has to name: `if (*p)` after
        /// a free has to underline `*p`. Underlining the whole `if` instead
        /// would put a caret on code that is not the defect, which this project
        /// ranks below saying nothing at all.
        ///
        /// **The whole controlling expression, and no finer than that.** C17
        /// 6.5.17 p2 makes a comma expression's value its right operand, so
        /// `if (c, *p)` underlines `c, *p` where only `*p` decided anything.
        /// That is wider than it could be and is still the expression rather
        /// than the statement, which is the failure this field exists to
        /// prevent; `a_dereference_after_a_comma_in_a_condition` pins what it
        /// does today and #147 is where it narrows.
        ///
        /// Whoever builds one owes that, and nothing downstream can recover it
        /// from a span that is already too wide. That is why the obligation is
        /// here and not beside the check that spends it.
        origin: Origin,
    },
    /// Enter another function, and come back.
    Call {
        /// Which function.
        callee: FuncId,
        /// What it is passed.
        arguments: Vec<Operand>,
        /// Where its result is written, or nothing where the value is
        /// discarded.
        ///
        /// `free(p);` writes nowhere, and a mandatory destination would make
        /// the lowering invent a local to throw the result into. An analysis
        /// that reads a write as an initialisation would then see one the
        /// source never asked for.
        destination: Option<Place>,
        /// Where control goes when it returns normally.
        then: BlockId,
        /// Where this call is, so that a diagnostic can point at it.
        ///
        /// One of the two terminators that carry one, and they are the two a
        /// diagnostic has had to name: `docs/safety-model.md` asks for "p freed
        /// here" and a free is a call, and [`Self::Branch`] has one because a
        /// dereference in a controlling expression has nowhere else to point.
        /// [`Operation`] carries the same field for the same reason, and a call
        /// is not an operation.
        origin: Origin,
    },
    /// Leave the function. The value is in local 0, which
    /// [`Function::return_place`] names.
    Return,
    /// An edge no statement produced.
    ///
    /// Nothing builds one. It is here so that every walk over a terminator has
    /// to answer for it before the thing that produces one exists, which is
    /// what ADR-0010 decided and why. An exception, a `longjmp` and the path an
    /// error handler runs on are all this shape.
    Abnormal {
        /// Where control goes instead.
        to: BlockId,
    },
}

impl Terminator {
    /// What this kind of ending is called, for `--emit safety-ir`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Goto(_) => "Goto",
            Self::Branch { .. } => "Branch",
            Self::Call { .. } => "Call",
            Self::Return => "Return",
            Self::Abnormal { .. } => "Abnormal",
        }
    }

    /// Every block this one can reach, appended to `out`.
    ///
    /// The graph's edges, in one place, so that the next walker does not work
    /// them out again. `ast::Expr::extend_children` is the same shape one layer
    /// up and exists for the same reason.
    ///
    /// The `match` is exhaustive and written out, so a terminator added later
    /// is `error[E0004]` here and in every other walk. That is ADR-0010's
    /// confirmation.
    ///
    /// **The order is an interface.** `dataflow::Analysis::edge` names an edge
    /// by its index into this list, so a `Branch`'s `then` is 0 and its
    /// `otherwise` is 1, and swapping the two here changes what every analysis
    /// written against it means without changing a line of that analysis. An
    /// analysis that refines the wrong arm is one that can be silent about the
    /// arm it was meant to prove something about.
    /// `every_terminator_says_where_control_can_go` asserts the order for
    /// exactly that reason.
    ///
    /// The fields are written out too, and `..` is deliberately not used. A
    /// second edge on a kind that already exists is the likelier growth than a
    /// new kind: an unwinding call keeps `then` and gains somewhere to go when
    /// the callee does not return normally. Spelled this way that field is
    /// `error[E0027]` here, and spelled `..` it would be silently dropped from
    /// the edge set while every walk kept compiling.
    pub fn successors(&self, out: &mut Vec<BlockId>) {
        match self {
            Self::Goto(to) | Self::Abnormal { to } => out.push(*to),
            Self::Branch {
                condition: _,
                then,
                otherwise,
                origin: _,
            } => out.extend([*then, *otherwise]),
            Self::Call {
                callee: _,
                arguments: _,
                destination: _,
                then,
                origin: _,
            } => out.push(*then),
            Self::Return => {}
        }
    }
}

/// A straight run of elements with one way in and one way out.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// What happens, in order.
    pub elements: Vec<Element>,
    /// How it ends.
    pub terminator: Terminator,
}

/// What a function has of a body.
///
/// A declaration and a definition nobody has finished lowering would both be an
/// empty list of blocks, and they are not the same thing. The difference is
/// what an analysis may assume at a call: a body it can see says what the call
/// does, and a body that is not here says nothing at all.
#[derive(Clone, Debug)]
enum Body {
    /// The definition is not in this translation unit.
    Declared,
    /// Blocks, in the order their ids were handed out.
    ///
    /// `None` is a block whose id exists and whose contents do not yet.
    Defined(Vec<Option<Block>>),
}

/// One parameter, as a call is checked against it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameter {
    /// Its type, which is the type of the local it becomes.
    pub ty: TyId,
    /// Where `_Nonnull` was written on it, if it was.
    ///
    /// A promise rather than a type: the body may believe it and every call
    /// is checked against it, which is ADR-0037. A span rather than a `bool`,
    /// because a report about a call points at the promise it broke.
    pub nonnull: Option<Span>,
}

/// A parameter declared with nothing but its type.
fn unannotated(ty: TyId) -> Parameter {
    Parameter { ty, nonnull: None }
}

/// One function's IR.
#[derive(Clone, Debug)]
pub struct Function {
    /// The span of its name, which is not its identity: [`FuncId`] is.
    pub name: Span,
    /// Local 0 is the return place, then the parameters, then the rest.
    locals: Vec<TyId>,
    /// One per parameter, in the order they were declared.
    nonnull: Vec<Option<Span>>,
    /// Whether this is a hatch.
    hatch: bool,
    body: Body,
}

impl Function {
    /// A function with a return place, its parameters, and no blocks yet.
    ///
    /// The parameters are taken here rather than pushed one at a time, because
    /// they are the locals that follow the return place and nothing else may
    /// come between: an interface that let a caller interleave them would have
    /// an invariant to remember instead of a shape that holds it.
    ///
    /// None of them carries `_Nonnull`. [`Function::with_parameters`] is the
    /// one that can say one does.
    pub fn new(name: Span, returns: TyId, parameters: impl IntoIterator<Item = TyId>) -> Self {
        Self::with_parameters(name, returns, parameters.into_iter().map(unannotated))
    }

    /// [`Function::new`], with what each parameter was declared as.
    ///
    /// A second constructor rather than [`Function::new`] taking anything that
    /// converts into a [`Parameter`]: that was tried and is `error[E0283]` at
    /// every `Function::new(at, int, [])`, because an empty array gives the
    /// compiler no type to infer.
    pub fn with_parameters(
        name: Span,
        returns: TyId,
        parameters: impl IntoIterator<Item = Parameter>,
    ) -> Self {
        let mut locals = vec![returns];
        let mut nonnull = Vec::new();
        for parameter in parameters {
            locals.push(parameter.ty);
            nonnull.push(parameter.nonnull);
        }

        Self {
            name,
            locals,
            nonnull,
            hatch: false,
            body: Body::Defined(Vec::new()),
        }
    }

    /// This function, as a hatch.
    ///
    /// A hatch claims nothing about its body: what a check could not prove in
    /// it is a statement about the hatch rather than about the program, and a
    /// driver lists it rather than reporting it. What a check concludes does
    /// not change. See ADR-0038.
    ///
    /// # Panics
    ///
    /// If this is a declaration. What a hatch says is about a body, and a
    /// function with none has nothing for it to say it about.
    ///
    /// **Not "unchecked".** Every check runs over a hatch's body as over any
    /// other; what differs is where an unproven conclusion goes.
    pub fn hatched(mut self) -> Self {
        assert!(self.is_defined(), "a declaration cannot be a hatch");
        self.hatch = true;
        self
    }

    /// Whether this is a hatch.
    pub fn hatch(&self) -> bool {
        self.hatch
    }

    /// A function this translation unit calls and does not contain.
    ///
    /// Its locals are its return place and its parameters, because that is what
    /// a call is checked against. It has no blocks and cannot be given any.
    pub fn declaration(
        name: Span,
        returns: TyId,
        parameters: impl IntoIterator<Item = TyId>,
    ) -> Self {
        Self::declaration_with_parameters(name, returns, parameters.into_iter().map(unannotated))
    }

    /// [`Function::declaration`], with what each parameter was declared as.
    pub fn declaration_with_parameters(
        name: Span,
        returns: TyId,
        parameters: impl IntoIterator<Item = Parameter>,
    ) -> Self {
        let mut declared = Self::with_parameters(name, returns, parameters);
        declared.body = Body::Declared;
        declared
    }

    /// Where `_Nonnull` was written on the parameter `local` is, or `None` if
    /// it was not, or if `local` is not a parameter.
    ///
    /// # Panics
    ///
    /// If `local` came from a different [`Function`].
    pub fn nonnull(&self, local: LocalId) -> Option<Span> {
        assert!(
            local.index() < self.locals.len(),
            "no local {}",
            local.index()
        );
        match local.index() {
            0 => None,
            index => self.nonnull.get(index - 1).copied().flatten(),
        }
    }

    /// Whether the body is here.
    pub fn is_defined(&self) -> bool {
        matches!(self.body, Body::Defined(_))
    }

    /// The blocks.
    ///
    /// # Panics
    ///
    /// If this is a declaration.
    fn defined(&self) -> &[Option<Block>] {
        match &self.body {
            Body::Defined(blocks) => blocks,
            Body::Declared => panic!("a declaration has no blocks"),
        }
    }

    /// The blocks, to add to.
    ///
    /// # Panics
    ///
    /// If this is a declaration.
    fn defined_mut(&mut self) -> &mut Vec<Option<Block>> {
        match &mut self.body {
            Body::Defined(blocks) => blocks,
            Body::Declared => panic!("a declaration has no blocks"),
        }
    }

    /// Where a `return` leaves its value.
    pub fn return_place(&self) -> LocalId {
        LocalId(0)
    }

    /// The parameters, in the order they were declared.
    pub fn parameters(&self) -> impl Iterator<Item = LocalId> + use<> {
        (1..=self.nonnull.len() as u32).map(LocalId)
    }

    /// Add a local, and hand back the id that names it.
    pub fn push_local(&mut self, ty: TyId) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(ty);
        id
    }

    /// Add a block, and hand back the id that names it.
    ///
    /// The id is taken before the push and not from `len()` after it, which is
    /// the mistake ADR-0008 records for the tree's arenas and the same one
    /// here.
    ///
    /// # Panics
    ///
    /// If this is a declaration.
    pub fn push_block(&mut self, block: Block) -> BlockId {
        let id = self.reserve_block();
        self.fill_block(id, block);
        id
    }

    /// Hand back an id for a block that has not been built yet.
    ///
    /// A terminator names the block control goes to, so a graph with a cycle
    /// cannot be built out of finished blocks alone: a `while` body names the
    /// header it jumps back to, the header names the body, and one of the two
    /// ids has to exist before its block does. The arms of an `if` need the
    /// same thing for the block they join at.
    ///
    /// # Panics
    ///
    /// If this is a declaration.
    pub fn reserve_block(&mut self) -> BlockId {
        let blocks = self.defined_mut();
        let id = BlockId(blocks.len() as u32);
        blocks.push(None);
        id
    }

    /// Put a block where a reserved id said one would go.
    ///
    /// # Panics
    ///
    /// If this is a declaration, if `id` came from a different [`Function`], or
    /// if `id` has already been filled. Filling twice would drop a block that
    /// other blocks still name, which is a graph that looks whole and is not.
    pub fn fill_block(&mut self, id: BlockId, block: Block) {
        let slot = &mut self.defined_mut()[id.index()];
        assert!(slot.is_none(), "block {} is already filled", id.index());
        *slot = Some(block);
    }

    /// The type of the local `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Function`].
    pub fn local(&self, id: LocalId) -> TyId {
        self.locals[id.index()]
    }

    /// Every local, in the order their ids were handed out.
    ///
    /// The return place first, then the parameters, then whatever a body
    /// needed. A count is `locals().len()`, which is why this hands back the
    /// ids rather than the number: a walk over them is what a printer and a
    /// dataflow analysis each want, and neither can build a [`LocalId`].
    pub fn locals(&self) -> impl ExactSizeIterator<Item = LocalId> + use<> {
        (0..self.locals.len() as u32).map(LocalId)
    }

    /// The block `id` names.
    ///
    /// # Panics
    ///
    /// If this is a declaration, if `id` came from a different [`Function`], or
    /// if `id` was reserved and never filled.
    pub fn block(&self, id: BlockId) -> &Block {
        self.defined()[id.index()]
            .as_ref()
            .unwrap_or_else(|| panic!("block {} was reserved and never filled", id.index()))
    }

    /// Where control enters, which is the block whose id was handed out first.
    ///
    /// Nothing outside this module can build a [`BlockId`], and a walk over a
    /// control-flow graph has to start somewhere, so without this an
    /// interpreter or an analysis could not take its first step. `blocks()`
    /// hands back blocks rather than ids for the same reason `locals()` used to
    /// hand back a count: it answers a different question.
    ///
    /// The first block is the entry because that is what a builder does, and
    /// saying so here is what makes it a property of the IR rather than of
    /// whoever built one.
    ///
    /// # Panics
    ///
    /// If this is a declaration, or if it has no blocks. A definition always
    /// has one, because a body ends with a terminator and a terminator ends a
    /// block.
    pub fn entry(&self) -> BlockId {
        assert!(!self.defined().is_empty(), "a definition has a first block");
        BlockId(0)
    }

    /// Every block, in the order their ids were handed out.
    ///
    /// # Panics
    ///
    /// If this is a declaration, or if a block was reserved and never filled. A
    /// walk over a graph with a hole in it would answer questions about a
    /// function nobody finished building.
    pub fn blocks(&self) -> impl ExactSizeIterator<Item = &Block> {
        self.defined().iter().enumerate().map(|(index, block)| {
            block
                .as_ref()
                .unwrap_or_else(|| panic!("block {index} was reserved and never filled"))
        })
    }
}

/// One `.c` file's worth of IR.
///
/// A translation unit and not a program: linking is what would make several of
/// these one program, and nothing links yet. It is the unit a lowering produces
/// and the unit an analysis is handed.
/// Not `Default`, deliberately. A unit that nobody said the target of is one
/// whose `int` has no width, and `docs/architecture.md` says the artifact
/// depends on that target rather than on whoever is running. Making the target
/// the one argument of [`TranslationUnit::new`] is how a caller cannot forget.
#[derive(Clone, Debug)]
pub struct TranslationUnit {
    target: Target,
    functions: Vec<Function>,
    types: Vec<Ty>,
    /// So that one type has one id. Not part of the shape: an implementation
    /// detail of [`Self::push_type`].
    interned: HashMap<Ty, TyId>,
}

impl TranslationUnit {
    /// An empty unit, for that machine.
    pub fn new(target: Target) -> Self {
        Self {
            target,
            functions: Vec::new(),
            types: Vec::new(),
            interned: HashMap::new(),
        }
    }

    /// The machine this unit is for.
    ///
    /// See ADR-0013 for why it is here rather than in the backend, and why a
    /// pointer's width is not among what it says.
    pub fn target(&self) -> Target {
        self.target
    }

    /// What this type is worth on this unit's target, if it is an integer.
    ///
    /// `None` for `void` and for a pointer, which are the two that have no
    /// width to answer with: a `void` is not a value and a pointer's width is
    /// not something this compiler needs yet. A caller that wants to convert or
    /// to check a range has to say what it does about those, which is the point
    /// of answering `Option` rather than panicking.
    pub fn integer(&self, id: TyId) -> Option<Integer> {
        match self.ty(id) {
            Ty::Int => Some(self.target.int()),
            Ty::Char => Some(self.target.char()),
            Ty::Void | Ty::Pointer(_) => None,
        }
    }

    /// The type a place reaches, or `None` where its projections do not fit.
    ///
    /// A fact about the IR rather than about running it, so it is here rather
    /// than in the interpreter: what `*p` is worth is the same question whoever
    /// asks.
    ///
    /// `None` rather than a panic, because the caller that asks most is the
    /// interpreter, whose whole doctrine is that it stops and says why instead
    /// of dying. A `Deref` of something that is not a pointer is a unit the
    /// lowering would not build and a hand-built one can, which is the case a
    /// Clang adapter is: telling it what is wrong beats a backtrace.
    pub fn place_ty(&self, function: &Function, place: &Place) -> Option<TyId> {
        let mut ty = function.local(place.local);
        for projection in &place.projection {
            ty = match (projection, self.ty(ty)) {
                (Projection::Deref, Ty::Pointer(pointee)) => pointee,
                (Projection::Deref, _) => return None,
                // `Index` is never built: `p[i]` lowers as `*(p + i)`, which
                // `docs/frontend.md` records. An analysis that starts building
                // one has to answer here.
                (Projection::Index(_), _) => return None,
            };
        }
        Some(ty)
    }

    /// The id for this type, which is the one it already had if it has one.
    ///
    /// Interned rather than appended, so that two ids are equal exactly when
    /// the types are the same. Every analysis asks that question constantly and
    /// the alternative is a structural walk at each call, which is what
    /// `ast::Ast::compatible` had to become one layer up. It works here and not
    /// there because a `Ty` reaches the rest of itself through ids that are
    /// themselves unique, so equality of the representation is equality of the
    /// type.
    pub fn push_type(&mut self, ty: Ty) -> TyId {
        if let Some(id) = self.interned.get(&ty) {
            return *id;
        }

        let id = TyId(self.types.len() as u32);
        self.types.push(ty);
        self.interned.insert(ty, id);
        id
    }

    /// Add a function, and hand back the id that names it.
    ///
    /// A caller with a body still to build pushes [`Function::declaration`]
    /// here and hands the definition to [`Self::fill_function`] later. That is
    /// what a call to a function whose body does not exist yet needs, and it is
    /// not an unusual case: `int f(void) { return f(); }` is one, and so is
    /// either half of a mutually recursive pair.
    pub fn push_function(&mut self, function: Function) -> FuncId {
        let id = FuncId(self.functions.len() as u32);
        self.functions.push(function);
        id
    }

    /// Give a function that was pushed as a declaration its body.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`TranslationUnit`], or if it already
    /// names a definition. Replacing a definition would leave the calls that
    /// were checked against the first one pointing at the second, which is a
    /// unit that looks whole and is not, and it is the same reason
    /// [`Function::fill_block`] refuses to fill a block twice.
    pub fn fill_function(&mut self, id: FuncId, function: Function) {
        let slot = &mut self.functions[id.index()];
        assert!(
            !slot.is_defined(),
            "function {} is already defined",
            id.index()
        );
        *slot = function;
    }

    /// The type `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`TranslationUnit`].
    pub fn ty(&self, id: TyId) -> Ty {
        self.types[id.0 as usize]
    }

    /// The function `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`TranslationUnit`].
    pub fn function(&self, id: FuncId) -> &Function {
        &self.functions[id.0 as usize]
    }

    /// Every function's id, in the order they were pushed.
    ///
    /// Ids rather than functions, the way [`Function::locals`] hands back
    /// locals: nothing outside this module can build a [`FuncId`], so a caller
    /// that has a `&Function` cannot say which function it is holding, and an
    /// interpreter asked to run `main` had no way to name it. The body comes
    /// from [`Self::function`], and a count is `functions().len()`.
    pub fn functions(&self) -> impl ExactSizeIterator<Item = FuncId> + use<> {
        (0..self.functions.len() as u32).map(FuncId)
    }
}

#[cfg(test)]
mod tests;
