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
//! Nothing here builds an IR or reads one. The lowering is #70, the printer is
//! #71 and the interpreter is #72; what this module owes them is a shape they
//! do not have to agree about first.
//!
//! [ADR-0010]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0010-give-the-graph-an-edge-no-statement-produced.md

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
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// Where it starts.
    pub local: LocalId,
    /// How to get from there to what is meant. Empty is the local itself.
    pub projection: Vec<Projection>,
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
#[derive(Clone, Debug, PartialEq)]
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

/// Whether the source wrote an operation, or something else caused it.
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
    /// Where to point, whichever kind it is.
    pub fn span(self) -> Span {
        match self {
            Self::Written(span) | Self::Generated(span) => span,
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
        /// Where its result is written.
        destination: Place,
        /// Where control goes when it returns normally.
        then: BlockId,
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
            } => out.push(*then),
            Self::Return => {}
        }
    }
}

/// A straight run of operations with one way in and one way out.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// What happens, in order.
    pub operations: Vec<Operation>,
    /// How it ends.
    pub terminator: Terminator,
}

/// One function's IR.
#[derive(Clone, Debug)]
pub struct Function {
    /// The span of its name, which is not its identity: [`FuncId`] is.
    pub name: Span,
    /// Local 0 is the return place, then the parameters, then the rest.
    locals: Vec<TyId>,
    parameters: usize,
    blocks: Vec<Block>,
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
            blocks: Vec::new(),
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
    pub fn push_block(&mut self, block: Block) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(block);
        id
    }

    /// The type of the local `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Function`].
    pub fn local(&self, id: LocalId) -> TyId {
        self.locals[id.index()]
    }

    /// How many locals there are, the return place included.
    pub fn locals(&self) -> usize {
        self.locals.len()
    }

    /// The block `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Function`].
    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id.index()]
    }

    /// Every block, in the order they were pushed.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
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
    pub fn push_function(&mut self, function: Function) -> FuncId {
        let id = FuncId(self.functions.len() as u32);
        self.functions.push(function);
        id
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

    /// Every function, in the order they were pushed.
    pub fn functions(&self) -> &[Function] {
        &self.functions
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
            operations: vec![Operation {
                place: Place::local(add.return_place()),
                value: Rvalue::Binary {
                    op: BinOp::Add,
                    lhs: Operand::Copy(Place::local(a)),
                    rhs: Operand::Copy(Place::local(b)),
                },
                origin: Origin::Written(at),
            }],
            terminator: Terminator::Return,
        });

        let id = unit.push_function(add);
        let add = unit.function(id);

        assert_eq!(add.return_place(), LocalId(0));
        assert_eq!(
            add.parameters().collect::<Vec<_>>(),
            [LocalId(1), LocalId(2)]
        );
        assert_eq!(add.locals(), 3);
        assert_eq!(add.local(add.return_place()), int);
        assert_eq!(add.local(LocalId(1)), int);
        assert_eq!(add.local(LocalId(2)), character);

        let [block] = add.blocks() else {
            panic!("{:?}", add.blocks());
        };
        assert_eq!(block.terminator, Terminator::Return);
        assert_eq!(
            block.operations,
            [Operation {
                place: Place::local(LocalId(0)),
                value: Rvalue::Binary {
                    op: BinOp::Add,
                    lhs: Operand::Copy(Place::local(LocalId(1))),
                    rhs: Operand::Copy(Place::local(LocalId(2))),
                },
                origin: Origin::Written(at),
            }]
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
            operations: Vec::new(),
            terminator: Terminator::Return,
        });
        let body = function.push_block(Block {
            operations: Vec::new(),
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
                    destination: Place::local(LocalId(0)),
                    then: two,
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
            operations: Vec::new(),
            terminator: Terminator::Return,
        });
        let second = function.push_block(Block {
            operations: Vec::new(),
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
        assert_eq!(unit.function(first).locals(), 1);
        assert_eq!(unit.function(second).locals(), 2);
        assert_eq!(unit.function(second).local(scratch), character);
    }
}
