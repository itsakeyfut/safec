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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
/// **No widths.** `docs/architecture.md` says an artifact depends on the target
/// "once type widths reach them", and nothing here has reached that yet: an
/// `Int` is C's `int` for whatever target, and the phase that lowers to LLVM is
/// where a width becomes a thing this has to carry.
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
/// [`Deref`]: Projection::Deref
/// [`Index`]: Projection::Index
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

/// One step of a block: something written, or storage beginning or ending.
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
    /// This local has storage from here on, and nothing in it.
    ///
    /// C17 6.2.4 p6 begins the lifetime at entry into the block rather than at
    /// the declaration, and ends it when "execution of that block ends in any
    /// way", so each iteration of a loop is a fresh lifetime and this is what
    /// the second one starts from.
    StorageLive {
        /// Whose storage.
        local: LocalId,
        /// The scope whose opening this is.
        origin: Origin,
    },
    /// This local's storage is gone from here on.
    StorageDead {
        /// Whose storage.
        local: LocalId,
        /// The scope whose closing this is.
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
            Self::StorageLive { .. } => "StorageLive",
            Self::StorageDead { .. } => "StorageDead",
        }
    }
}

/// One step: a place, and what is written into it.
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
        /// The only terminator that carries one, because it is the only one a
        /// diagnostic has had to name so far: `docs/safety-model.md` asks for
        /// "p freed here", and a free is a call. [`Operation`] carries the same
        /// field for the same reason, and a call is not an operation.
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

/// One function's IR.
#[derive(Clone, Debug)]
pub struct Function {
    /// The span of its name, which is not its identity: [`FuncId`] is.
    pub name: Span,
    /// Local 0 is the return place, then the parameters, then the rest.
    locals: Vec<TyId>,
    parameters: usize,
    body: Body,
}

impl Function {
    /// A function with a return place, its parameters, and no blocks yet.
    ///
    /// The parameters are taken here rather than pushed one at a time, because
    /// they are the locals that follow the return place and nothing else may
    /// come between: an interface that let a caller interleave them would have
    /// an invariant to remember instead of a shape that holds it.
    pub fn new(name: Span, returns: TyId, parameters: impl IntoIterator<Item = TyId>) -> Self {
        let mut locals = vec![returns];
        locals.extend(parameters);
        let parameters = locals.len() - 1;

        Self {
            name,
            locals,
            parameters,
            body: Body::Defined(Vec::new()),
        }
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
        let mut declared = Self::new(name, returns, parameters);
        declared.body = Body::Declared;
        declared
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
        (1..=self.parameters as u32).map(LocalId)
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
#[derive(Clone, Debug, Default)]
pub struct TranslationUnit {
    functions: Vec<Function>,
    types: Vec<Ty>,
    /// So that one type has one id. Not part of the shape: an implementation
    /// detail of [`Self::push_type`].
    interned: HashMap<Ty, TyId>,
}

impl TranslationUnit {
    /// An empty unit.
    pub fn new() -> Self {
        Self::default()
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
mod tests {
    use super::*;
    use crate::source::SourceMap;

    /// A source map with one file, so that a span can be built at all.
    ///
    /// What these tests are about is the shape, so the file is a formality:
    /// nothing here reads the text back.
    fn spans() -> (SourceMap, Span) {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int add(int a, char b) { return a + b; }\n");
        let span = Span::new(file, 31, 36);
        (sources, span)
    }

    /// `int add(int a, char b) { return a + b; }`, built by hand and read back.
    ///
    /// This is the phase's third Done-when clause arriving before the first:
    /// there is no frontend in this test, and #73 is what will make that
    /// mechanical rather than true by accident.
    ///
    /// Mutation: have `Function::new` push its parameters before the return
    /// type. Local 0 stops being the return place and this fails, on the type
    /// of local 1.
    ///
    /// The second parameter is a `char` for that mutation's sake alone. With
    /// `int add(int, int)` every local holds one type, the reordering is
    /// invisible to every assertion here, and the mutation above passes: the
    /// ids `return_place` and `parameters` hand back are computed from a count
    /// rather than read from where the types went.
    #[test]
    fn a_function_is_built_and_read_back() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);

        let character = unit.push_type(Ty::Char);

        let mut add = Function::new(at, int, [int, character]);
        let [a, b] = add.parameters().collect::<Vec<_>>()[..] else {
            panic!("two parameters");
        };

        add.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(add.return_place()),
                value: Rvalue::Binary {
                    op: BinOp::Add,
                    lhs: Operand::Copy(Place::local(a)),
                    rhs: Operand::Copy(Place::local(b)),
                },
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });

        let id = unit.push_function(add);
        let add = unit.function(id);

        assert_eq!(add.name, at);
        assert_eq!(unit.functions().len(), 1);
        assert_eq!(add.return_place(), LocalId(0));
        assert_eq!(
            add.parameters().collect::<Vec<_>>(),
            [LocalId(1), LocalId(2)]
        );
        assert_eq!(add.locals().len(), 3);
        assert_eq!(add.local(add.return_place()), int);
        assert_eq!(add.local(LocalId(1)), int);
        assert_eq!(add.local(LocalId(2)), character);

        let [block] = add.blocks().collect::<Vec<_>>()[..] else {
            panic!("one block");
        };
        assert_eq!(block.terminator, Terminator::Return);
        assert_eq!(
            block.elements,
            [Element::Assign(Operation {
                place: Place::local(LocalId(0)),
                value: Rvalue::Binary {
                    op: BinOp::Add,
                    lhs: Operand::Copy(Place::local(LocalId(1))),
                    rhs: Operand::Copy(Place::local(LocalId(2))),
                },
                origin: Origin::Written(at),
            })]
        );
    }

    /// An edge no statement produced can be built, which is what ADR-0010
    /// decided and what it is confirmed by.
    ///
    /// Mutation: delete `Terminator::Abnormal`. This stops compiling, which is
    /// the strongest form the guard can take and the reason the variant is
    /// here before anything produces one.
    #[test]
    fn an_edge_no_statement_produced_can_be_built() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let void = unit.push_type(Ty::Void);
        let mut function = Function::new(at, void, []);

        let handler = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        let body = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Abnormal { to: handler },
        });

        assert_eq!(
            function.block(body).terminator,
            Terminator::Abnormal { to: handler }
        );
    }

    /// Where control can go from each way a block can end.
    ///
    /// The expected lists are written out rather than derived, for the reason
    /// RK-001 gives: a table built the way the code builds one compares the
    /// code with itself.
    ///
    /// Mutation: add a terminator kind. `Terminator::successors` stops
    /// compiling with `error[E0004]`, and so does every other walk over one,
    /// which is what ADR-0010 is for. Mutation: have the `Branch` arm push
    /// only `then`. This fails.
    #[test]
    fn every_terminator_says_where_control_can_go() {
        let (_sources, at) = spans();
        let one = BlockId(1);
        let two = BlockId(2);

        for (terminator, expected) in [
            (Terminator::Goto(one), vec![one]),
            (
                Terminator::Branch {
                    condition: Operand::Constant(0),
                    then: one,
                    otherwise: two,
                },
                vec![one, two],
            ),
            (
                Terminator::Call {
                    callee: FuncId(0),
                    arguments: Vec::new(),
                    destination: Some(Place::local(LocalId(0))),
                    then: two,
                    origin: Origin::Written(at),
                },
                vec![two],
            ),
            (Terminator::Return, Vec::new()),
            (Terminator::Abnormal { to: one }, vec![one]),
        ] {
            let mut successors = Vec::new();
            terminator.successors(&mut successors);

            assert_eq!(successors, expected, "{terminator:?}");
        }
    }

    /// `p` and `*p` are two places.
    ///
    /// An analysis that treated them as one would say a pointer is live when
    /// what it points at is not, which is the memory axis of
    /// `docs/safety-model.md` answering the wrong question.
    ///
    /// Mutation: give `Place` an equality that compares its local alone. This
    /// fails.
    #[test]
    fn a_place_is_not_the_place_it_points_at() {
        let p = Place::local(LocalId(1));
        let pointee = Place {
            local: LocalId(1),
            projection: vec![Projection::Deref],
        };

        assert_ne!(p, pointee);
        assert_eq!(pointee.local, p.local);
    }

    /// An id keeps naming its block after more are pushed.
    ///
    /// ADR-0008's guard, one layer down. The vectors are private for the same
    /// reason: holding a `&Block` across a push is `error[E0502]`.
    ///
    /// Mutation: take the id from `len()` after the push rather than before.
    /// Every id is off by one and this fails.
    #[test]
    fn an_id_still_names_its_block_after_more_are_pushed() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let void = unit.push_type(Ty::Void);
        let mut function = Function::new(at, void, []);

        let first = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        let second = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Goto(first),
        });

        assert_eq!(function.block(first).terminator, Terminator::Return);
        assert_eq!(function.block(second).terminator, Terminator::Goto(first));
    }

    /// One type has one id, however many times it is asked for.
    ///
    /// That is what lets a `TyId` be compared with `==` instead of walked, and
    /// what lets `Ty` derive `PartialEq` where the frontend's `ast::Type`
    /// refuses to.
    ///
    /// Mutation: have `push_type` append without looking in `interned`. The
    /// two `int`s become two ids, the two `int *`s become two more, and this
    /// fails.
    #[test]
    fn one_type_has_one_id() {
        let mut unit = TranslationUnit::new();

        let int = unit.push_type(Ty::Int);
        let also_int = unit.push_type(Ty::Int);
        let character = unit.push_type(Ty::Char);
        let pointer = unit.push_type(Ty::Pointer(int));
        let also_pointer = unit.push_type(Ty::Pointer(also_int));

        assert_eq!(int, also_int);
        assert_eq!(pointer, also_pointer);
        assert_ne!(int, character);
        assert_ne!(int, pointer);
        assert_eq!(unit.ty(pointer), Ty::Pointer(int));
    }

    /// An operation can say nobody wrote it, and still point somewhere.
    ///
    /// `docs/roadmap.md` asks for exactly this: a destructor at the end of a
    /// scope has "a location to blame and no source text". The two kinds carry
    /// the same span here, because what differs is not where to point but what
    /// a diagnostic may say about it.
    ///
    /// Mutation: delete `Origin::Generated` and the arm of `span` that reads
    /// it. This stops compiling. Mutation: have `Origin::Generated` mean the
    /// same as `Written`, by making the two operations below compare equal.
    /// The `assert_ne!` fails.
    #[test]
    fn an_operation_can_say_nobody_wrote_it() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(at, int, []);
        let temporary = function.push_local(int);

        let write = |origin| Operation {
            place: Place::local(temporary),
            value: Rvalue::Use(Operand::Constant(0)),
            origin,
        };
        let written = write(Origin::Written(at));
        let generated = write(Origin::Generated(at));

        assert_eq!(written.origin.span(), at);
        assert_eq!(generated.origin.span(), at);
        assert_ne!(written, generated);
    }

    /// Two functions with one name are two functions.
    ///
    /// C says so already: two `static` functions in different translation
    /// units share a name, and `docs/c-family.md` asks that identity in the IR
    /// be an id rather than a string for that reason. The name here is one
    /// span, which is the strongest version of the case: even the same text at
    /// the same place does not merge them.
    ///
    /// Mutation: have `push_function` hand back `FuncId(0)` rather than the
    /// length before the push. This fails, on the ids and on the second
    /// function's locals.
    ///
    /// Mutation: have `push_local` take its id from `len() - 1`. The local it
    /// hands back names the one before it and this fails.
    #[test]
    fn two_functions_with_one_name_have_two_ids() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let character = unit.push_type(Ty::Char);

        let first = Function::new(at, int, []);
        let mut second = Function::new(at, int, []);
        let scratch = second.push_local(character);

        let first = unit.push_function(first);
        let second = unit.push_function(second);

        assert_ne!(first, second);
        assert_eq!(unit.function(first).name, unit.function(second).name);
        assert_eq!(unit.function(first).locals().len(), 1);
        assert_eq!(unit.function(second).locals().len(), 2);
        assert_eq!(unit.function(second).local(scratch), character);
    }

    /// `&a[i]`: the two shapes the lifetime analysis is written against.
    ///
    /// `Rvalue::Address` is the only operation that turns a place into a
    /// value, and `Projection::Index` is how a place reaches an element, so an
    /// escape through an element goes through both at once. Nothing had built
    /// either, and a variant nothing builds is one that can be deleted in
    /// silence.
    ///
    /// Mutation: delete `Rvalue::Address`, or `Projection::Index`, or
    /// `UnOp::Neg`. Each stops this compiling. Mutation: have `Place::local`
    /// hand back a place with a `Deref` on it. The `assert_ne!` fails.
    #[test]
    fn an_address_can_be_taken_of_an_element() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let pointer = unit.push_type(Ty::Pointer(int));

        let mut function = Function::new(at, pointer, []);
        let array = function.push_local(int);
        let index = function.push_local(int);
        let p = function.push_local(pointer);

        let element = Place {
            local: array,
            projection: vec![Projection::Index(Operand::Copy(Place::local(index)))],
        };
        let taken = Operation {
            place: Place::local(p),
            value: Rvalue::Address(element.clone()),
            origin: Origin::Written(at),
        };
        let negated = Operation {
            place: Place::local(index),
            value: Rvalue::Unary {
                op: UnOp::Neg,
                operand: Operand::Copy(Place::local(index)),
            },
            origin: Origin::Written(at),
        };

        assert_ne!(element, Place::local(array));
        assert_eq!(taken.value, Rvalue::Address(element));
        assert_ne!(taken.value, negated.value);
    }

    /// A loop, which is a graph a finished block cannot be pushed into.
    ///
    /// The header names the body and the body names the header, so one of the
    /// two ids exists before its block does. That is what `reserve_block` is
    /// for, and without it the module could hold every straight-line function
    /// and no `while` at all.
    ///
    /// Mutation: delete `reserve_block` and `fill_block`. This stops compiling,
    /// and no way of ordering the pushes brings it back. Mutation: have
    /// `fill_block` push rather than write into the slot the id names. The back
    /// edge lands on the wrong block and this fails.
    #[test]
    fn a_loop_is_built_by_reserving_the_block_it_jumps_back_to() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);
        let mut function = Function::new(at, int, []);
        let counter = function.push_local(int);

        let header = function.reserve_block();
        let exit = function.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        let body = function.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(counter),
                value: Rvalue::Binary {
                    op: BinOp::Sub,
                    lhs: Operand::Copy(Place::local(counter)),
                    rhs: Operand::Constant(1),
                },
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Goto(header),
        });
        function.fill_block(
            header,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Branch {
                    condition: Operand::Copy(Place::local(counter)),
                    then: body,
                    otherwise: exit,
                },
            },
        );

        let mut successors = Vec::new();
        function
            .block(header)
            .terminator
            .successors(&mut successors);
        assert_eq!(successors, [body, exit]);

        successors.clear();
        function.block(body).terminator.successors(&mut successors);
        assert_eq!(successors, [header]);
        assert_eq!(function.blocks().len(), 3);
    }

    /// A call says where it was written, and may write its result nowhere.
    ///
    /// `free(p);` is both at once: `docs/safety-model.md` wants to say "p freed
    /// here", and there is no place the result goes.
    ///
    /// Mutation: delete `Terminator::Call`'s `origin`, or make `destination` a
    /// `Place` again. Each stops this compiling, and the first takes the span
    /// the memory analysis points at with it.
    #[test]
    fn a_call_says_where_it_is_and_may_write_nowhere() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let void = unit.push_type(Ty::Void);
        let int = unit.push_type(Ty::Int);
        let pointer = unit.push_type(Ty::Pointer(int));

        let free = unit.push_function(Function::declaration(at, void, [pointer]));
        let mut caller = Function::new(at, void, []);
        let p = caller.push_local(pointer);
        let after = caller.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        let call = caller.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Call {
                callee: free,
                arguments: vec![Operand::Copy(Place::local(p))],
                destination: None,
                then: after,
                origin: Origin::Written(at),
            },
        });

        let Terminator::Call {
            destination,
            origin,
            callee,
            ..
        } = &caller.block(call).terminator
        else {
            panic!("a call");
        };
        assert_eq!(*destination, None);
        assert_eq!(origin.span(), at);
        assert_eq!(callee.index(), 0);
    }

    /// A function whose body is elsewhere is not a function with no blocks.
    ///
    /// What an analysis may assume at a call turns on which of the two it is,
    /// and the unsound reading of an empty list is that the callee does
    /// nothing.
    ///
    /// Mutation: have `Function::declaration` return what `Function::new`
    /// returns. `is_defined` starts answering true and this fails.
    #[test]
    fn a_declaration_is_not_a_definition_with_no_blocks() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let void = unit.push_type(Ty::Void);
        let int = unit.push_type(Ty::Int);

        let declared = Function::declaration(at, void, [int]);
        let defined = Function::new(at, void, [int]);

        assert!(!declared.is_defined());
        assert!(defined.is_defined());
        assert_eq!(declared.locals().len(), defined.locals().len());
        assert_eq!(defined.blocks().len(), 0);
    }

    /// A place is what a dataflow lattice is keyed on.
    ///
    /// `p` and `*p` carry separate states, so a table indexed by [`LocalId`] is
    /// the wrong table and the key has to be the place itself.
    ///
    /// Mutation: take `Eq` or `Hash` off `Place`. This stops compiling.
    #[test]
    fn a_place_is_a_key() {
        let mut states = HashMap::new();
        let p = Place::local(LocalId(1));
        let pointee = Place {
            local: LocalId(1),
            projection: vec![Projection::Deref],
        };

        states.insert(p.clone(), "live");
        states.insert(pointee.clone(), "freed");

        assert_eq!(states.get(&p), Some(&"live"));
        assert_eq!(states.get(&pointee), Some(&"freed"));
    }

    /// A declaration becomes the definition it was standing in for.
    ///
    /// Mutation: have `fill_function` write to the first function rather than
    /// to the id it was given. The second keeps its declaration and this fails.
    #[test]
    fn a_declared_function_can_be_given_its_body() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);

        let first = unit.push_function(Function::declaration(at, int, []));
        let second = unit.push_function(Function::declaration(at, int, []));
        unit.fill_function(second, Function::new(at, int, []));

        assert!(!unit.function(first).is_defined());
        assert!(unit.function(second).is_defined());
    }

    /// A function is given a body once.
    ///
    /// A second one would leave every call that was checked against the first
    /// definition pointing at another, which is the defect `fill_block` refuses
    /// for a block.
    ///
    /// Mutation: drop the assertion in `fill_function`. Nothing panics and this
    /// fails.
    #[test]
    #[should_panic(expected = "already defined")]
    fn a_function_is_not_given_a_body_twice() {
        let (_sources, at) = spans();
        let mut unit = TranslationUnit::new();
        let int = unit.push_type(Ty::Int);

        let id = unit.push_function(Function::declaration(at, int, []));
        unit.fill_function(id, Function::new(at, int, []));
        unit.fill_function(id, Function::new(at, int, []));
    }
}
