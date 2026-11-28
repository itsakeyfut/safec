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
//! **Every place is still rooted at a local**, and an object with static
//! storage duration is reached through its address: [`Rvalue::Static`] puts
//! it in a local of pointer type, and the place is `*t`. The objects are the
//! unit's, [`TranslationUnit::statics`], each with what it starts with, so a
//! global and a local of one type are two different things to every reader
//! of the IR. See ADR-0115.
//!
//! Nothing here reads an IR. `crates/safec-c/src/lowering.rs` builds one from
//! the typed AST; the printer is [`crate::print`] and the interpreter is
//! [`crate::interp`], and what this module owes them is a shape they do not
//! have to agree about first.
//!
//! [ADR-0010]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0010-give-the-graph-an-edge-no-statement-produced.md
//! [ADR-0012]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0012-say-a-local-s-storage-began-and-ended-in-the-block.md

use std::collections::{HashMap, HashSet};

use crate::record;
use crate::source::{SourceMap, Span};
use crate::target::{Integer, Target};

/// Four of the library functions C says do not return to their caller, with
/// the clause that says so.
///
/// `abort` (C17 7.22.4.1), `exit` (7.22.4.4), `_Exit` (7.22.4.5) and
/// `quick_exit` (7.22.4.7). `longjmp` (7.13.2.1) and `thrd_exit` (7.26.5.5)
/// do not return either: the first takes a `jmp_buf`, which this frontend
/// cannot declare, and the second belongs to the threads that
/// `docs/roadmap.md` places in Phase 8.
///
/// **Believed by name because 7.1.3 reserves the name**: p1 reserves each for
/// use as an identifier with external linkage, and p2 makes a program that
/// defines one undefined. So the lowering gives a call to one no continuation,
/// and names refuses a definition of one, wherever it is, rather than
/// honouring it: a definition in another file of the run would otherwise be
/// believed away in silence (#343). One list for both, so the names believed
/// and the names refused cannot drift apart. See ADR-0051.
///
/// **Here rather than in the frontend**, because the memory check asks it too:
/// a call to one inside a larger full expression keeps its edge, and a path
/// through that edge still ends at the call. See `memory::assigned`.
pub const DOES_NOT_RETURN: &[(&str, &str)] = &[
    ("abort", "7.22.4.1"),
    ("exit", "7.22.4.4"),
    ("_Exit", "7.22.4.5"),
    ("quick_exit", "7.22.4.7"),
];

/// An integer type's rank, C17 6.3.1.1 p1, from the lowest: what an integer
/// type is, before the target says how wide it is. See ADR-0116.
///
/// **Only the ranks a program can write**, so that one added later is
/// `error[E0004]` at every site that answers for an integer, which is where
/// each has to say what it is to it: a rank no program could write yet was
/// answered at every site already, and gave the next one nothing to be asked.
/// See ADR-0116.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rank {
    /// `char`, `signed char` and `unsigned char`.
    Char,
    /// `short`.
    Short,
    /// `int`.
    Int,
    /// `long`.
    Long,
    /// `long long`.
    LongLong,
}

/// How an integer type is signed: `Plain` is `char` alone, which C17 6.2.5
/// p15 makes a third type beside `signed char` and `unsigned char`, signed or
/// not as the target says; every other integer type is `Signed` unless
/// written `unsigned`, whether or not `signed` was written. Only the signs a
/// program can write, for the reason [`Rank`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sign {
    /// Plain `char`, and nothing else.
    Plain,
    /// Signed, written or not.
    Signed,
    /// `unsigned`.
    Unsigned,
}

/// What C calls the integer type of this rank and sign, the way `clang`
/// spells it in a diagnostic: `unsigned int`, `long long`, `signed char`.
/// One table for `--emit ast` and `--emit safety-ir`, which spell a type
/// alike.
///
/// # Panics
///
/// For `Plain` with a rank other than `char`, which is no type.
pub fn integer_name(rank: Rank, sign: Sign) -> &'static str {
    match (rank, sign) {
        (Rank::Char, Sign::Plain) => "char",
        (Rank::Char, Sign::Signed) => "signed char",
        (Rank::Char, Sign::Unsigned) => "unsigned char",
        (Rank::Short, Sign::Signed) => "short",
        (Rank::Short, Sign::Unsigned) => "unsigned short",
        (Rank::Int, Sign::Signed) => "int",
        (Rank::Int, Sign::Unsigned) => "unsigned int",
        (Rank::Long, Sign::Signed) => "long",
        (Rank::Long, Sign::Unsigned) => "unsigned long",
        (Rank::LongLong, Sign::Signed) => "long long",
        (Rank::LongLong, Sign::Unsigned) => "unsigned long long",
        (Rank::Short | Rank::Int | Rank::Long | Rank::LongLong, Sign::Plain) => {
            unreachable!("{rank:?} is never plain: `Plain` is `char`'s alone")
        }
    }
}

impl Ty {
    /// `int`.
    pub const INT: Ty = Ty::Integer {
        rank: Rank::Int,
        sign: Sign::Signed,
    };
    /// Plain `char`.
    pub const CHAR: Ty = Ty::Integer {
        rank: Rank::Char,
        sign: Sign::Plain,
    };
}

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

/// Which struct, within one [`TranslationUnit`]: a type of its own, so that
/// two structs with the same members are two types (C17 6.2.7 p1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StructId(u32);

impl StructId {
    /// The index this handle refers to, for printing and for a side table.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Which function type, within one [`TranslationUnit`]: the [`Signature`]
/// a [`Ty::Function`] names. Interned as a type is, so two equal signatures
/// are one id and two function types compare as their ids do (#1212).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SignatureId(u32);

impl SignatureId {
    /// The index this handle refers to, for printing and for a side table.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What a function takes and returns, as a call through a pointer to it
/// has to pass them: the shape [`Function`] lays out for a declaration,
/// held where no [`Function`] is named (#1212).
///
/// **Lowered, not as written**: a struct returned by value is a pointer
/// parameter first and `void` returned, and a struct parameter is the
/// address of the caller's copy, each marked by its [`Passes`], so the
/// backend reads a call through a pointer as it reads a direct one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Signature {
    /// What it returns, after a struct result is moved into a parameter.
    pub returns: TyId,
    /// Each parameter's type and how it is passed, in order.
    pub parameters: Vec<(TyId, Passes)>,
    /// Whether the parameters were declared. `false` for `()`, which C17
    /// 6.7.6.3 p14 makes say nothing about them, so a call through a
    /// pointer to one passes its arguments as they are.
    pub prototyped: bool,
    /// Whether it takes `...` after its parameters, so a call through a
    /// pointer to it passes the arguments past them promoted (#1213).
    pub variadic: bool,
}

/// A struct as the target lays it out: each member's type and byte offset,
/// and the whole's size and alignment. Worked out once, by
/// [`TranslationUnit::push_struct`], and the one answer the backend and the
/// memory check read. See ADR-0124.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructLayout {
    /// Each member, in the order declared.
    pub members: Vec<MemberLayout>,
    /// How many bytes the whole takes, trailing padding included.
    pub size: u64,
    /// The strictest of its members' alignments.
    pub align: u64,
}

/// One member as [`TranslationUnit::complete_record`] takes it: its
/// declared type, its width where it is a bit-field, and whether it has a
/// name, which an unnamed bit-field may not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Declared {
    /// The type it was declared with.
    pub ty: TyId,
    /// Its width in bits where it is a bit-field.
    pub width: Option<u64>,
    /// Whether it has a name.
    pub named: bool,
}

/// Where a bit-field of `width` bits starting at bit `bit` is read and
/// written, as its byte offset and its size in bytes: the narrowest of 1,
/// 2, 4 and 8 bytes that holds it whole, ends inside a whole of `size`
/// bytes, and starts on a boundary of its own size or of the whole's
/// alignment `align`, whichever is smaller. See [`Bits`].
///
/// **The whole's alignment where it is smaller** for `i686`, whose `long
/// long` is aligned to 4 and takes 8: a bit-field of one may span bytes 4
/// to 8, which no unit on a boundary of 8 holds.
///
/// # Panics
///
/// Where no unit of 8 bytes or fewer holds a bit-field that is read, which
/// no layout of [`record::lay_out`] gives: each packs a bit-field into a
/// unit of its type's size on a boundary of its alignment.
fn access_unit(bit: u64, width: u64, size: u64, align: u64) -> (u64, u64) {
    // **A bit-field nothing reads, zero-width or unnamed,** may stand at the
    // very end of the whole, or be wider than it, where no unit fits: a byte
    // inside it is all it needs to name (#1206 review). Mutation: ask it for
    // a unit like any other; `the_layout_of_every_bit_field_shape_is_clangs`
    // panics at the first shape that ends in one.
    if width == 0 {
        return ((bit / 8).min(size.saturating_sub(1)), 1);
    }
    for unit in [1, 2, 4, 8] {
        let boundary = unit.min(align);
        let offset = bit / (boundary * 8) * boundary;
        if bit + width <= (offset + unit) * 8 && offset + unit <= size {
            return (offset, unit);
        }
    }
    // A layout above never asks for more: every target packs a bit-field
    // into a unit of its type's size on a boundary of its alignment, and
    // the whole is a multiple of that alignment. Said rather than guessed.
    panic!(
        "no unit of 8 bytes or fewer holds a bit-field at bit {bit}, {width} wide, in {size} bytes"
    )
}

/// One member of a [`StructLayout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberLayout {
    /// Its type: for a bit-field, the unsigned integer it is read and
    /// written through, its unit, and not the type it was declared with.
    pub ty: TyId,
    /// Where it starts, in bytes from the start of the struct: for a
    /// bit-field, where its unit starts.
    pub offset: u64,
    /// Which bits of its unit a bit-field is (#1206), `None` for any other
    /// member. See [`Bits`].
    pub bits: Option<Bits>,
}

/// The bits of its unit a bit-field takes: `width` of them, from bit `at`
/// counted from the unit's least significant, which is where every target
/// here starts a unit's first bit-field.
///
/// **A bit-field is read and written through its unit**, the narrowest
/// unsigned integer that holds the bit-field whole and stays inside the
/// struct, on a boundary of its own size or of the struct's alignment where
/// that is smaller (`access_unit`): so it is a slot of its object as any
/// member is (ADR-0124), a write of one is a read of its unit and a write
/// of it with the other bits kept, and no access reaches past the struct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bits {
    /// The first bit, from the unit's least significant.
    pub at: u32,
    /// How many bits; `0` for a zero-width bit-field. A zero-width or an
    /// unnamed one is read by nothing, and its unit is any byte in the
    /// whole, which `at` may lie past.
    pub width: u32,
}

/// How many elements an array has: a count known when the program is
/// compiled, or the local that holds it when the array is declared, which a
/// variable length array's is (C17 6.7.6.2 p4, #1209).
///
/// **A local of the function the array is declared in**, so an array type
/// with one is read in that function alone, as every local is. The length is
/// worked out once, where the declaration is reached (p5), and the local
/// holds it from there; nothing writes it again. A type whose length is a
/// local has no size this unit can give ([`TranslationUnit::size_of`] says
/// `None`), so every check that counts against a size concludes nothing
/// about one rather than something wrong.
///
/// **A type with one means something only beside its function.** Types are
/// interned for the whole unit, so `int[_2]` in two functions is one
/// [`TyId`] naming a different local in each. Only a local's own type holds
/// one today; a signature, a static or a struct member that did would hand
/// a caller a local of another function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Length {
    /// This many elements.
    Constant(u64),
    /// As many as this local holds where the array is declared.
    Local(LocalId),
}

impl Length {
    /// The count, where it is known when the program is compiled.
    pub fn constant(self) -> Option<u64> {
        match self {
            Length::Constant(length) => Some(length),
            Length::Local(_) => None,
        }
    }
}

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
    /// analysis is. `FileId::index` in `crates/safec-ir/src/source.rs` is the same
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
/// integer is worth is asked of the unit, which carries the target:
/// [`TranslationUnit::integer`]. An integer is a rank and a sign, which is
/// what C names one by, and its width is the target's (ADR-0013), so `long`
/// can be 32 bits on one machine and 64 on another. See ADR-0116.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// An integer type, C17 6.2.5 p4 to p7: `int` is `Integer { Int, Signed }`
    /// and `char` `Integer { Char, Plain }`.
    Integer {
        /// Which of C's standard integer types, by rank (6.3.1.1 p1).
        rank: Rank,
        /// How it is signed: see [`Sign`].
        sign: Sign,
    },
    /// `void`, which is what a function returns when it returns nothing.
    Void,
    /// A pointer to the type this id names.
    Pointer(TyId),
    /// `length` objects of the type `element` names, one after another,
    /// which is what C17 6.2.5 p20 makes an array. A constant `length` is
    /// zero only for a flexible array member, as the field says: the lowering
    /// builds any other only for a constant length the type check gave a
    /// positive value, and 6.7.6.2 p1 makes any other constant length a
    /// constraint violation. A variable length array's is a local, worked
    /// out where it is declared (#1209): see [`Length`].
    ///
    /// **Never a value.** C reads an array only by converting it to a pointer
    /// to its first element (6.3.2.1 p3) and gives it no assignment, so no
    /// operation reads or writes a place of this type whole: the lowering
    /// converts it where it is used, and the backend refuses a read or a
    /// write of one.
    Array {
        /// The type of each element.
        element: TyId,
        /// How many elements there are. Zero only for a flexible array member
        /// (C17 6.7.2.1 p18, #1208), the last member of a struct, which takes
        /// no bytes of it and whose elements are as many as were allocated
        /// past it: the memory check bounds a walk through one by the
        /// allocation rather than by the member. See [`Length`].
        length: Length,
    },
    /// A struct, laid out in [`TranslationUnit::struct_layout`], or a union.
    ///
    /// **A union is this too** (#1203), told apart by
    /// [`TranslationUnit::is_union`] and nowhere else: everything a struct
    /// is to every reader, a union is, its members' offsets aside, and those
    /// are in its layout. A second variant made every reader answer for both,
    /// and one that forgot, in an `if let`, compiled and passed: the memory
    /// check lost a report about a union holding a pointer and no test saw
    /// it.
    ///
    /// **Never a value yet**, as an array is not: no operation reads or
    /// writes one whole, which is assignment's, and a call's and a return's
    /// (#1201). A member is reached through its address, a
    /// [`Projection::Field`] under `Rvalue::Address`.
    Struct(StructId),
    /// A function of the type its [`Signature`] says, which is what a
    /// function pointer points at (#1212).
    ///
    /// **Never a value**, as an array is not: a function is no object (C17
    /// 6.2.5 p1), so no place has this type and no operation reads or writes
    /// one. It is reached only as a [`Ty::Pointer`]'s pointee.
    Function(SignatureId),
}

impl Ty {
    /// Whether a value of this type is a pointer.
    ///
    /// **An exhaustive `match`, the one answer every analysis reads**, so
    /// that a kind of type added later, a function pointer, is
    /// `error[E0004]` here rather than "not a pointer" at each reader that
    /// spelled `matches!(.., Ty::Pointer(_))`. What a kind nobody has added
    /// yet is decides what the memory and nullability checks follow, so a
    /// `_` arm would answer it without anybody deciding.
    ///
    /// **An array answers no.** No whole array is a value: C converts it to
    /// a pointer to its first element where it is used (C17 6.3.2.1 p3).
    pub fn is_pointer(self) -> bool {
        match self {
            Ty::Pointer(_) => true,
            Ty::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            }
            | Ty::Void
            | Ty::Array { .. }
            | Ty::Struct(_)
            | Ty::Function(_) => false,
        }
    }
}

/// A step from a local towards the thing an operation means.
///
/// `docs/safety-model.md`'s Place is a local plus however many of these it
/// takes to reach what is being read or written: `*p` is one [`Deref`], and
/// an array's element is one [`Index`], which the lowering builds only as a
/// constant zero to convert a local array to a pointer to its first element:
/// `a[i]` is `*(a + i)`. A struct's member is one [`Field`], which the
/// lowering builds only under `Rvalue::Address` too: `s.m` is `*&s.m`, so
/// every read and write is through a pointer, as an element's is.
///
/// [`Ord`] for the reason [`Place`] gives.
///
/// [`Deref`]: Projection::Deref
/// [`Index`]: Projection::Index
/// [`Field`]: Projection::Field
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Projection {
    /// What the pointer points at.
    Deref,
    /// The element this operand selects.
    Index(Operand),
    /// The member of a struct at this index, counting from zero in the order
    /// declared: the slot of the object at the member's offset and width.
    /// See ADR-0124.
    Field(u32),
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
    ///
    /// **Private, so that a reader asks [`Self::shape`]** and a new kind of
    /// step is a compile error wherever a place is read rather than a branch
    /// nobody revisits (#1092). See ADR-0113.
    projection: Vec<Projection>,
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
            Self::Field(_) => "Field",
        }
    }
}

impl Operand {
    /// What this kind of operand is called, for `--emit safety-ir`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Copy(_) => "Copy",
            Self::Constant(_) => "Constant",
            Self::Function(_) => "Function",
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

    /// A place reached from `local` by `steps`, for the lowering and for IR
    /// built by hand.
    pub fn new(local: LocalId, steps: Vec<Projection>) -> Self {
        Self {
            local,
            projection: steps,
        }
    }

    /// What this place's steps amount to, which a reader matches with every
    /// arm written so that a new kind of step is `E0004` wherever a place is
    /// read. See ADR-0113.
    pub fn shape(&self) -> Shape {
        let mut derefs = 0;
        for step in &self.projection {
            match step {
                Projection::Deref => derefs += 1,
                Projection::Index(_) | Projection::Field(_) => return Shape::Indexed,
            }
        }
        if derefs == 0 {
            Shape::Local
        } else {
            Shape::Deref(derefs)
        }
    }

    /// This place with one more dereference: what it points at.
    #[must_use]
    pub fn deref(&self) -> Self {
        let mut place = self.clone();
        place.projection.push(Projection::Deref);
        place
    }

    /// This place's first `steps` steps.
    #[must_use]
    pub fn prefix(&self, steps: usize) -> Self {
        Self {
            local: self.local,
            projection: self.projection[..steps].to_vec(),
        }
    }

    /// The place this one dereferences last, where its last step is a
    /// dereference: the pointer a write through it goes through.
    pub fn without_last_deref(&self) -> Option<Self> {
        match self.projection.split_last() {
            Some((Projection::Deref, rest)) => Some(Self {
                local: self.local,
                projection: rest.to_vec(),
            }),
            Some((Projection::Index(_) | Projection::Field(_), _)) | None => None,
        }
    }

    /// The steps themselves, read-only, for a walk that matches every
    /// [`Projection`] on its own: printing, the interpreter, liveness, the
    /// backend. An analysis asks [`Self::shape`] instead.
    pub fn steps(&self) -> &[Projection] {
        &self.projection
    }
}

/// What a place's steps amount to, for a reader that has to answer each.
///
/// **No `PartialEq`, so it is matched rather than compared**, and a new kind
/// of step is a new variant here: `E0004` at every reader that has not said
/// what it means. `every_reader_of_a_place_answers_every_shape` holds the
/// readers to a `match`. See ADR-0113.
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// The local itself: no steps.
    Local,
    /// Nothing but dereferences, this many, at least one.
    ///
    /// **The only place a dereference count comes from**, so that a reader
    /// that counts levels answers every other shape itself: a count of `0`
    /// for a place with any other step was a named answer no new kind of
    /// step would have made anybody revisit (found by review of #1092).
    /// A place with an `Index` in it is an array's element and is not one of
    /// these. No program this compiler accepts tells that apart from
    /// following it too: the lowering builds an `Index` only under
    /// `Rvalue::Address`, which is no place read or written, and `a[i]` is
    /// `*(a + i)` (#621). See ADR-0045.
    Deref(usize),
    /// A step that is not a dereference: an `Index` or a `Field`, which the
    /// lowering builds only under `Rvalue::Address` (#621, #1200). The name
    /// predates the second, and ADR-0113 records it.
    Indexed,
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
    /// The address of a function: a function designator converted to a
    /// pointer to it (C17 6.3.2.1 p4), to the function type
    /// [`TranslationUnit::designator_signature`] gives. A function is no object, so
    /// this names nothing the memory check follows. See ADR-0125.
    Function(FuncId),
}

/// What a call calls (#1212).
///
/// **One terminator for both**, so that what a call does with its
/// arguments, its result and its continuation is one body whichever way the
/// callee is named, and every reader of a callee answers both arms. See
/// ADR-0125.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Called {
    /// A function this unit names. Also what a call through a pointer that
    /// is a function designator is: the lowering never writes
    /// `Through(Operand::Function(_))`.
    Direct(FuncId),
    /// Whatever function the pointer this operand holds points at, whose
    /// type is the pointer's pointee, a [`Ty::Function`]. Read by the memory
    /// check as a call to code it cannot read.
    Through(Operand),
    /// One of the operations on a list of variadic arguments, which no
    /// function is: what `__builtin_va_start`, `__builtin_va_end` and
    /// `__builtin_va_copy` are (#1214). Its arguments are the lists'
    /// addresses. Read by the memory check as a call to code it cannot read
    /// handed them. See ADR-0126.
    Intrinsic(Intrinsic),
}

/// Which operation on a list of variadic arguments a
/// [`Called::Intrinsic`] is, each taking the lists' addresses (C17 7.16.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
    /// Start the list its one argument points at, at the first argument past
    /// the function's parameters.
    VaStart,
    /// End the list its one argument points at.
    VaEnd,
    /// Copy the list its second argument points at into the first.
    VaCopy,
}

impl Intrinsic {
    /// What it is called, for `--emit safety-ir`.
    pub fn name(self) -> &'static str {
        match self {
            Self::VaStart => "VaStart",
            Self::VaEnd => "VaEnd",
            Self::VaCopy => "VaCopy",
        }
    }
}

impl Called {
    /// The function this names, where it names one. A reader that asks this
    /// answers a call through a pointer as one it cannot name, and says why
    /// that is enough where it does.
    pub fn direct(&self) -> Option<FuncId> {
        match self {
            Called::Direct(id) => Some(*id),
            Called::Through(_) | Called::Intrinsic(_) => None,
        }
    }

    /// The pointer a call through one reads, which every reader of what a
    /// call reads asks beside its arguments: calling through a pointer reads
    /// it as a dereference would.
    pub fn operand(&self) -> Option<&Operand> {
        match self {
            Called::Direct(_) | Called::Intrinsic(_) => None,
            Called::Through(operand) => Some(operand),
        }
    }
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

/// Where an object with static storage duration is in a [`TranslationUnit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StaticId(u32);

impl StaticId {
    /// Where this id points in the unit's table.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An object with static storage duration: C17 6.2.4 p3, an object declared
/// at file scope or with `static`, or a string literal (6.4.5 p6). It lives
/// as long as the program does, so no function's frame holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Static {
    /// The name the backend gives its symbol: the name a file-scope object was
    /// declared with, or one made unique for a `static` local.
    pub name: String,
    /// Its type.
    pub ty: TyId,
    /// What other translation units may name it as.
    pub linkage: Linkage,
    /// What it holds before the program begins (6.7.9 p10).
    pub init: StaticInit,
    /// Whether the program may not modify it: one defined `const` at its
    /// innermost element (C17 6.7.3 p6), or a string literal (6.4.5 p7).
    /// See ADR-0120.
    pub read_only: bool,
}

/// What another translation unit may name an object with static storage
/// duration as. C17 6.2.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkage {
    /// Defined here, and other units may name it.
    External,
    /// Defined here, and only this unit names it: `static` at file scope, a
    /// `static` local, a string literal.
    Internal,
    /// Named here with `extern` and defined in another unit.
    Declared,
}

/// What an object with static storage duration holds before the program
/// begins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StaticInit {
    /// Zero, which is what one with no initializer holds (C17 6.7.9 p10).
    Zero,
    /// An integer constant, converted to the object's type.
    Int(i128),
    /// The first elements of an array of character type, one byte each, and
    /// zero for every element past them: a string literal's bytes, its
    /// terminating zero included (C17 6.4.5 p6), or an array a literal
    /// initializes, cut to its length or with its zeros left to whoever
    /// writes the object (6.7.9 p14 and p21), so that a large array is not
    /// as large in the IR.
    Bytes(Vec<u8>),
    /// A struct's members or an array's elements an initializer writes,
    /// each by its index, in increasing order, which the backend lays them
    /// out by, and zero for every one not listed: a constant shaped like
    /// its type, and **sparse**, so that
    /// `static int t[1 << 24] = {[(1 << 24) - 1] = 1};` is one entry and
    /// not sixteen million (#1242). A union's is the one member its
    /// initializer leaves it holding (`initializer::Shapes::resolve`). See
    /// ADR-0137.
    Aggregate(Vec<(u64, StaticInit)>),
    /// The address of the object `of` plus `offset` bytes: an address
    /// constant (C17 6.6 p9), which has no value until the program is
    /// linked.
    Address {
        /// The object it points into.
        of: StaticId,
        /// How far into it, in bytes.
        offset: u64,
        /// The initializer that wrote it, where a report about the store it
        /// stands for points (ADR-0138).
        at: Span,
    },
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
    /// The address of an object with static storage duration, a global, a
    /// `static` local or a string literal, which is how every use of one is
    /// reached: `g = 1` is `t = Static(g); *t = 1`. Never null, and pointing
    /// to storage no function owns. See ADR-0115.
    Static(StaticId),
    /// The operand's bits as the type the operation is written into, where
    /// one of the two is an integer and the other a pointer: C17 6.3.2.3 p5
    /// and p6, a cast in either direction. What the result points at is
    /// nothing an analysis can follow from the operand, so the memory check
    /// reports every one as unproven where it is written. See ADR-0117.
    Reinterpret(Operand),
    /// The next argument past a variadic function's parameters, read out of
    /// the list at this place as the type the operation is written into, and
    /// the list moved past it: what `__builtin_va_arg` is (C17 7.16.1.1;
    /// #1214). A pointer read so names nothing the memory check follows. See
    /// ADR-0126.
    VaArg(Place),
}

impl Rvalue {
    /// What this kind of value is called, for `--emit safety-ir`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Use(_) => "Use",
            Self::Unary { .. } => "Unary",
            Self::Binary { .. } => "Binary",
            Self::Address(_) => "Address",
            Self::Static(_) => "Static",
            Self::Reinterpret(_) => "Reinterpret",
            Self::VaArg(_) => "VaArg",
        }
    }
}

/// Whether the source wrote an operation, or something else caused it.
///
/// `docs/c-family.md` calls this attribution. Where code came from through a
/// macro is the file a span is in, an instance (ADR-0054), which left this
/// type as it is: a macro's body was written, so its operations are `Written`,
/// and a generated operation has no spelling to come from.
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
    /// The initializer at this span gives an object the value it starts
    /// with. Not an assignment, so the prohibition on modifying a `const`
    /// object (C17 6.7.3 p6) is not about it, and the read-only check does
    /// not ask it of the local it initializes (#1244 review).
    Initialized(Span),
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
            Self::Initialized(_) => "initialized",
        }
    }

    /// Where to point, whichever kind it is.
    pub fn span(self) -> Span {
        match self {
            Self::Written(span) | Self::Generated(span) | Self::Initialized(span) => span,
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
    ///
    /// **A variable length array's storage is made here** (#1209), as large
    /// as its [`Length::Local`] holds, since p7 begins its lifetime at the
    /// declaration: the backend reads that local at this marker. So a
    /// producer writes the length before this marker on every path to it,
    /// and pairs it with a [`Self::StorageDead`] on every way out of its
    /// scope and on every jump back to before it, which is where the
    /// backend gives the storage back.
    StorageLive {
        /// Whose storage.
        local: LocalId,
        /// Where the declaration that asked for it is written.
        origin: Origin,
    },
    /// This local's storage is gone from here on. For a variable length
    /// array, the stack it was made on is given back (#1209).
    StorageDead {
        /// What ended the storage, which is where control left the scope.
        ///
        /// The `}` of a scope control falls off the end of, or the `return`
        /// that leaves it early. A producer that points at the `}` of a scope
        /// a jump leaves is pointing at a brace control never reached.
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
    /// Emitted where ADR-0022's enclosure rule reaches the **call**, asked
    /// about the call rather than about the point: a call's arguments are
    /// unsequenced against each other, so a call below a `+` whose other
    /// operand may act gets none of these and its argument reads stay open.
    /// One below a node whose other operands cannot act, `x = g(*p)`, gets one
    /// (#178).
    ///
    /// **Half of what [`Self::Sequenced`] says, and the half is the design.**
    /// A read behind this is ordered before everything that follows. A *free*
    /// behind this is not concluded to be ordered against anything, because a
    /// call carries reads in its own operands and a consumer judges those after
    /// this element, while C puts them before it: concluding the other half
    /// made `g((free(p), 0), *p)` a proved use after free, about two arguments
    /// C leaves unsequenced.
    ///
    /// **What the omission can cost is bounded**, because anything in the same
    /// full expression that can follow one of these is one of four things. It
    /// is separated from it by a [`Self::Sequenced`], because the sequencing
    /// operators emit one: `free(p), *p = 42;` is the case to read, where the
    /// free is a comma's left operand, the write follows it, and the comma's
    /// own marker proves it. Or it is a sibling that cannot act, which reads
    /// no pointer and frees nothing. Or it is the store of an assignment whose
    /// place cannot act, which C17 6.5.16 p3 sequences after the value
    /// computations of both operands: a free behind this element that such a
    /// store needs ordered is ordered only by a marker of its own, and where
    /// it has none the store is a doubt C would have licensed as a proof,
    /// which a reader can see (#178). Or it is what the enclosing node does
    /// with the call's result, a call it is an argument of or a dereference of
    /// it, which comes after the call (6.5.2.2 p10, 6.5 p1).
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
    /// One operand of the node this stands before writes this local, and
    /// another operand of the same node reads or writes it: two arguments of
    /// a call, or two operands of an operator that does not sequence.
    ///
    /// C17 6.5 p2 makes that undefined whatever the local holds: 6.5 p3 leaves
    /// an operator's operands, and a call's arguments, unsequenced against
    /// each other, and 6.5.2.2 p10 orders the arguments only before the call.
    /// The IR lowers the operands in one order and reads a place where the
    /// node is built, which is an order C did not choose, so the conflict is
    /// said here rather than left to whatever that order reads. A frontend
    /// that finds it owes this element; one that does not leaves `g(q, q =
    /// &x)` and `*q = (free(q), q = r, 0)` asked about the `q` the other
    /// operand wrote. See [ADR-0052].
    ///
    /// [ADR-0052]: https://github.com/itsakeyfut/safec/blob/main/docs/adr/0052-an-argument-that-writes-a-pointer-another-argument-reads-is-reported-where-it-is-lowered.md
    UnsequencedAccess {
        /// The local both operands name.
        local: LocalId,
        /// Where one operand writes it.
        written: Span,
        /// Where another operand reads it, or writes it too.
        accessed: Span,
        /// Whether that other access is a write.
        both_write: bool,
        /// Whether the two are a call's arguments or an operator's operands,
        /// which is what a report names them as.
        between: Between,
        /// The node whose operands these are, named by its own span.
        origin: Origin,
    },
}

/// What the two accesses an [`Element::UnsequencedAccess`] names are
/// operands of.
///
/// Its own type rather than a `bool`, because a reader is choosing words by
/// it, and `true` says nothing about which words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Between {
    /// Two arguments of one call, which C17 6.5.2.2 p10 orders before the call
    /// and 6.5 p3 leaves unsequenced against each other.
    Arguments,
    /// Two operands of an operator that does not sequence: an assignment's
    /// place and value, a binary operator's two sides, a subscript's base and
    /// index, which C17 6.5 p3 leaves unsequenced.
    Operands,
    /// Two parameters' lengths, which a definition evaluates on entry and
    /// nothing orders: a parameter's declarator is not a full declarator, so
    /// its end is no sequence point (C17 6.7.6 p3). That the outermost length
    /// of a parameter adjusted to a pointer is evaluated at all is the
    /// lowering's reading of 6.9.1 p10, which names only a variably modified
    /// parameter; it is what clang does.
    Lengths,
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
            Self::UnsequencedAccess { .. } => "UnsequencedAccess",
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
/// is what `safec-c`'s did until it was measured. The fix is what C says the
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
        /// **The expression whose value decides, not the whole controlling
        /// expression.** C17 6.5.17 p2 makes a comma expression's value its
        /// right operand, so `if (c, *p)` underlines `*p`, and `c` is not
        /// what decided; `a_dereference_after_a_comma_in_a_condition` pins
        /// it.
        ///
        /// Whoever builds one owes that, and nothing downstream can recover it
        /// from a span that is already too wide. That is why the obligation is
        /// here and not beside the check that spends it.
        origin: Origin,
    },
    /// Enter another function, and come back.
    Call {
        /// What it calls: a function this unit names, or one through a
        /// pointer.
        callee: Called,
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
        /// Where control goes when it returns normally, or `None` where the
        /// callee does not return, which leaves the call with no edge out of
        /// it. See ADR-0051.
        then: Option<BlockId>,
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
    /// the callee unwinds. Spelled this way that field is `error[E0027]` here,
    /// and spelled `..` it would be silently dropped from the edge set while
    /// every walk kept compiling. Such an edge would be index 1 after a `then`
    /// and index 0 where there is none (ADR-0051), so an analysis reading it
    /// would have to match `then` as well as the kind.
    pub fn successors(&self, out: &mut Vec<BlockId>) {
        match self {
            Self::Goto(to) | Self::Abnormal { to } => out.push(*to),
            Self::Branch {
                condition: _,
                then,
                otherwise,
                origin: _,
            } => out.extend([*then, *otherwise]),
            // A call that does not return has no edge, so what follows it is
            // reached by no execution. See ADR-0051.
            Self::Call {
                callee: _,
                arguments: _,
                destination: _,
                then,
                origin: _,
            } => out.extend(*then),
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
    /// Whether it is promised not to be null, and why.
    ///
    /// A promise rather than a type: the body may believe it and every call
    /// is checked against it, which is ADR-0037. A [`Promise`] rather than a
    /// span, so that a parameter level 5 made non-null by default is told
    /// apart from one written `_Nonnull` by the type: a report about a call
    /// names the promise it broke, and the two are taken back differently.
    pub nonnull: Option<Promise>,
    /// The span of the name it was declared with, in the declaration the
    /// function's type is taken from, and `None` where that declaration names
    /// none, as `int f(int);` may. For a definition that is the definition;
    /// for a function only declared it is the standing declaration, the one
    /// its `_Nonnull` promises come from too, so `int g(int a); int g(int b);`
    /// names it `b` while [`Function::name`] is the first `g`.
    pub name: Option<Span>,
    /// Whether it is a pointer to `const`, so that a call cannot write
    /// through it (C17 6.7.3 p6 makes doing so after a cast undefined where
    /// the object is `const`). What a call handed read-only storage is
    /// asked by. See ADR-0120.
    pub reads_only: bool,
    /// What the C parameter it stands for is: itself, or a struct passed by
    /// value, which the IR passes as a pointer. See [`Passes`].
    pub passes: Passes,
}

/// What a parameter of the IR stands for in the C function it lowers.
///
/// **A struct crosses a call as a pointer in the IR** (#1201): an argument
/// is the address of a copy the caller makes, whose storage ends when the
/// call returns, and a result the address the caller wants it in, passed
/// first. Both are promised non-null as [`Promise::Made`]. The C frontend
/// copies an argument again into a local of the callee's own, which is
/// what the parameter's name is, so no check reads the callee's object as
/// one the caller handed over. The backend reads this and writes each
/// target's C ABI, where a struct may travel in registers instead: see
/// `safec_llvm::abi`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Passes {
    /// The parameter itself.
    Value,
    /// A struct passed by value, as the address of the caller's copy of it.
    StructCopy,
    /// Where a struct returned by value goes, the caller's, passed first;
    /// the function then returns nothing.
    StructReturn,
}

/// Why a pointer a function takes or returns is believed not to be null, which
/// is where a report about breaking the promise points.
///
/// **The IR says what was promised and not why the level made it so.** A
/// default is resolved where the IR is built, so nothing that reads this asks
/// what level a run is at, which is ADR-0011's boundary held for the safety
/// level as for everything else above this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Promise {
    /// `_Nonnull` was written here, on a declaration of it.
    ///
    /// `Declared` rather than `Written`, which [`Origin::Written`] already
    /// means for an operation the source wrote.
    Declared(Span),
    /// Nothing was written, and level 5 makes the pointer non-null unless it is
    /// written `_Nullable` (ADR-0050). The span is the name of what was
    /// promised: the function's, for the pointer it returns.
    Defaulted(Span),
    /// Nothing was written, and this compiler made the pointer: the one a
    /// struct passed or returned by value crosses as (see [`Passes`]), which
    /// every caller passes as the address of an object of its own. The span
    /// is the parameter's name, or the function's for a result.
    Made(Span),
}

/// A parameter declared with nothing but its type.
fn unannotated(ty: TyId) -> Parameter {
    Parameter {
        ty,
        nonnull: None,
        name: None,
        reads_only: false,
        passes: Passes::Value,
    }
}

/// One function's IR.
#[derive(Clone, Debug)]
pub struct Function {
    /// The span of its name, which is not its identity: [`FuncId`] is.
    pub name: Span,
    /// The library function this is, where this compiler made the
    /// declaration itself rather than reading one: a struct assignment is a
    /// call to `memcpy` whatever the source declares (#1201). Read by name
    /// through [`Function::spelled`], so the memory check reads it by its
    /// row and the backend declares it, and `name` is where it was needed.
    builtin: Option<&'static str>,
    /// Local 0 is the return place, then the parameters, then the rest.
    locals: Vec<TyId>,
    /// The span of the name each local was written with, in step with
    /// `locals`, and `None` for one the source never wrote: the return place
    /// and every temporary this compiler makes. What lets a reader of the IR
    /// say `p` rather than `_1`. Only [`Function::with_parameters`] and the
    /// two `push_` methods add to either, so the two are pushed together.
    names: Vec<Option<Span>>,
    /// One per parameter, in the order they were declared.
    nonnull: Vec<Option<Promise>>,
    /// One per parameter: [`Parameter::reads_only`].
    reads_only: Vec<bool>,
    /// One per parameter: [`Parameter::passes`].
    passes: Vec<Passes>,
    /// Whether this is a hatch.
    hatch: bool,
    /// Whether its name has internal linkage, `static` in C (C17 6.2.2 p3),
    /// so that no other unit's function of the name is this one.
    internal: bool,
    /// Whether the pointer it returns is promised not to be null.
    return_promise: Option<Promise>,
    /// The locals the program may not modify, each defined `const` at its
    /// innermost element (C17 6.7.3 p6). See ADR-0120.
    read_only: Vec<LocalId>,
    /// The locals that hold a variable length array type's length worked
    /// out where a typedef is reached (#1294), which no array's storage
    /// reads: the memory check doubts each where it begins, as it doubts a
    /// variable length array's declaration, since C17 6.7.6.2 p5 asks the
    /// length be above zero each time it is worked out.
    variable_lengths: Vec<LocalId>,
    /// Whether what it returns is a pointer to `const`, so that a caller
    /// cannot write through it: [`Parameter::reads_only`] for the result.
    returns_reads_only: bool,
    /// Per local, the levels at which its declared type points at `const`:
    /// bit `k` set where what `k` dereferences of it read points at an
    /// object defined `const` (bit `0`, a pointer to `const`). Such a level
    /// may name read-only storage whatever value it holds, and one that is
    /// not is a level a write through it is believed to be allowed at. See
    /// ADR-0120 and ADR-0121.
    const_levels: Vec<(LocalId, u32)>,
    /// Per temporary, the levels at which the value it was given points at
    /// `const`, where its own type says less: what a cast that drops `const`
    /// converted, a pointer moved by arithmetic, an array decayed. **Kept
    /// apart from [`Self::const_levels`]**, which say what a reader declared:
    /// read as one, `unp((char **)&c)` asked the cast's reader at the
    /// operand's levels and was believed (#1158). See ADR-0123.
    carried_levels: Vec<(LocalId, u32)>,
    /// Whether its parameters were declared: `false` for `()`, which says
    /// nothing about them (C17 6.7.6.3 p14). What [`Signature::prototyped`]
    /// is made from.
    prototyped: bool,
    /// Whether its prototype ends in `, ...`, so a call may pass arguments
    /// past its parameters, each after the default argument promotions
    /// (C17 6.5.2.2 p7). What [`Signature::variadic`] is made from (#1213).
    variadic: bool,
    /// Whether this is the unit's function before `main`. See
    /// [`Function::mark_initializes_statics`].
    initializes_statics: bool,
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
        let mut names = vec![None];
        let mut nonnull = Vec::new();
        let mut reads_only = Vec::new();
        let mut passes = Vec::new();
        for parameter in parameters {
            locals.push(parameter.ty);
            names.push(parameter.name);
            nonnull.push(parameter.nonnull);
            reads_only.push(parameter.reads_only);
            passes.push(parameter.passes);
        }

        Self {
            name,
            builtin: None,
            locals,
            names,
            nonnull,
            reads_only,
            passes,
            hatch: false,
            internal: false,
            return_promise: None,
            read_only: Vec::new(),
            variable_lengths: Vec::new(),
            returns_reads_only: false,
            const_levels: Vec::new(),
            carried_levels: Vec::new(),
            prototyped: true,
            variadic: false,
            initializes_statics: false,
            body: Body::Defined(Vec::new()),
        }
    }

    /// Say that this is the unit's function before `main`: a store per
    /// address a static's initializer holds, for the memory check to ask as
    /// it asks any store, which the backend writes no code for. See
    /// ADR-0138.
    pub fn mark_initializes_statics(&mut self) {
        self.initializes_statics = true;
    }

    /// Whether this is the unit's function before `main`. See
    /// [`Function::mark_initializes_statics`].
    pub fn initializes_statics(&self) -> bool {
        self.initializes_statics
    }

    /// This function, declared with `, ...` after its parameters (#1213).
    pub fn variadic(mut self) -> Self {
        self.variadic = true;
        self
    }

    /// Whether a call may pass arguments past its parameters. See
    /// [`Self::variadic`].
    pub fn is_variadic(&self) -> bool {
        self.variadic
    }

    /// This function, declared with `()`, which says nothing about its
    /// parameters (C17 6.7.6.3 p14).
    pub fn unprototyped(mut self) -> Self {
        self.prototyped = false;
        self
    }

    /// Whether its parameters were declared. See [`Self::unprototyped`].
    pub fn prototyped(&self) -> bool {
        self.prototyped
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

    /// This function, with internal linkage.
    pub fn with_internal_linkage(mut self) -> Self {
        self.internal = true;
        self
    }

    /// Whether its name has internal linkage.
    pub fn internal_linkage(&self) -> bool {
        self.internal
    }

    /// This function, promising that the pointer it returns is not null.
    ///
    /// The body is asked at every `return` whether it keeps the promise, and a
    /// call believes it of what it returns. Where nobody here can ask the
    /// body, because it is a declaration, the promise is believed unasked
    /// (ADR-0050). A hatch's body is asked, and what it could not prove is
    /// listed rather than reported, so its callers believe it unchecked too
    /// (ADR-0038).
    pub fn promising(mut self, promise: Promise) -> Self {
        self.return_promise = Some(promise);
        self
    }

    /// What it promised of the pointer it returns, or `None` where a call
    /// believes nothing about it.
    pub fn promised(&self) -> Option<Promise> {
        self.return_promise
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

    /// A declaration of the library function `library`, which this compiler
    /// made itself and the source need not spell: see [`Self::spelled`].
    /// `at` is where it was needed, which a report about the call points at.
    pub fn builtin(
        library: &'static str,
        at: Span,
        returns: TyId,
        parameters: impl IntoIterator<Item = TyId>,
    ) -> Self {
        let mut declared = Self::declaration(at, returns, parameters);
        declared.builtin = Some(library);
        declared
    }

    /// [`Function::builtin`], with what each parameter was declared as: a
    /// `const void *` the library only reads through, as `memcpy`'s source.
    pub fn builtin_with_parameters(
        library: &'static str,
        at: Span,
        returns: TyId,
        parameters: impl IntoIterator<Item = Parameter>,
    ) -> Self {
        let mut declared = Self::declaration_with_parameters(at, returns, parameters);
        declared.builtin = Some(library);
        declared
    }

    /// The library function this compiler made this as, or `None` for one
    /// the program declared or defined.
    pub fn made_as(&self) -> Option<&'static str> {
        self.builtin
    }

    /// The name this function is called by: the library function it is,
    /// where this compiler made it, and what its name's span spells
    /// otherwise. **Every reader of a function's name asks this**, so a
    /// declaration made with no source text behind it is named all the same.
    pub fn spelled<'s>(&self, sources: &'s SourceMap) -> &'s str {
        // With a `.`, which no C identifier holds, so nothing the program
        // names is spelled the same.
        if self.initializes_statics {
            return ".statics";
        }
        match self.builtin {
            Some(library) => library,
            None => sources.snippet(self.name),
        }
    }

    /// Where `_Nonnull` was written on the parameter `local` is, or `None` if
    /// it was not, or if `local` is not a parameter.
    ///
    /// # Panics
    ///
    /// If `local` came from a different [`Function`].
    pub fn nonnull(&self, local: LocalId) -> Option<Promise> {
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

    /// Add a local the source never wrote, and hand back the id that names it.
    pub fn push_local(&mut self, ty: TyId) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(ty);
        self.names.push(None);
        id
    }

    /// What the parameter at `index`, counted from 0, stands for:
    /// [`Passes::Value`] past the last.
    pub fn passes(&self, index: usize) -> Passes {
        self.passes.get(index).copied().unwrap_or(Passes::Value)
    }

    /// Whether this returns a struct by value, through a pointer passed
    /// first: see [`Passes::StructReturn`]. A function with no parameters
    /// answers `false`, by [`Function::passes`]'s answer past the last.
    pub fn returns_a_struct(&self) -> bool {
        self.passes(0) == Passes::StructReturn
    }

    /// Whether the parameter at `index`, counted from 0, points at `const`,
    /// so a call cannot write through it. `false` past the last, where an
    /// argument has no parameter to say so. See ADR-0120.
    pub fn reads_only_through(&self, index: usize) -> bool {
        self.reads_only.get(index).copied().unwrap_or(false)
    }

    /// Add `levels` to the levels at which `local` points at `const`. See
    /// [`Self::const_levels`].
    pub fn mark_const_levels(&mut self, local: LocalId, levels: u32) {
        if levels == 0 {
            return;
        }
        match self
            .const_levels
            .iter_mut()
            .find(|(each, _)| *each == local)
        {
            Some((_, marked)) => *marked |= levels,
            None => self.const_levels.push((local, levels)),
        }
    }

    /// The levels at which `local` points at `const`: bit `k` where what `k`
    /// dereferences of it read points at an object defined `const`. See
    /// ADR-0121.
    pub fn const_levels(&self, local: LocalId) -> u32 {
        self.const_levels
            .iter()
            .find(|(each, _)| *each == local)
            .map_or(0, |(_, levels)| *levels)
    }

    /// Add `levels` to what the value `local` was given points at `const`
    /// at, apart from its declared levels. See [`Self::carried_levels`].
    pub fn mark_carried_levels(&mut self, local: LocalId, levels: u32) {
        if levels == 0 {
            return;
        }
        match self
            .carried_levels
            .iter_mut()
            .find(|(each, _)| *each == local)
        {
            Some((_, marked)) => *marked |= levels,
            None => self.carried_levels.push((local, levels)),
        }
    }

    /// The levels at which the value `local` was given points at `const`
    /// beyond what its type declares: a cast's operand's, a moved pointer's,
    /// a decayed array's. **Never a reader's question**, which reads
    /// [`Self::const_levels`] alone. See ADR-0123.
    pub fn carried_levels(&self, local: LocalId) -> u32 {
        self.carried_levels
            .iter()
            .find(|(each, _)| *each == local)
            .map_or(0, |(_, levels)| *levels)
    }

    /// Every level at which what `local` holds may point at `const`: what
    /// its type declares and what its value carried. What the memory check
    /// marks a value with, never what it asks a reader. See ADR-0123.
    pub fn held_levels(&self, local: LocalId) -> u32 {
        self.const_levels(local) | self.carried_levels(local)
    }

    /// Whether `local` is declared as a pointer to `const`. See ADR-0120.
    pub fn points_at_const(&self, local: LocalId) -> bool {
        self.const_levels(local) & 1 != 0
    }

    /// Say that what this returns is a pointer to `const`. See ADR-0120.
    pub fn mark_returns_reads_only(&mut self) {
        self.returns_reads_only = true;
    }

    /// Whether what this returns is a pointer to `const`.
    pub fn returns_reads_only(&self) -> bool {
        self.returns_reads_only
    }

    /// Say that `local` may not be modified, being defined `const`. See
    /// ADR-0120.
    pub fn mark_read_only(&mut self, local: LocalId) {
        self.read_only.push(local);
    }

    /// Whether `local` may not be modified, being defined `const`.
    pub fn read_only(&self, local: LocalId) -> bool {
        self.read_only.contains(&local)
    }

    /// Say that `local` holds a variable length array type's length, worked
    /// out where a typedef is reached. See `Function::variable_lengths`.
    pub fn mark_variable_length(&mut self, local: LocalId) {
        self.variable_lengths.push(local);
    }

    /// Whether `local` holds a typedef's variable length.
    pub fn variable_length(&self, local: LocalId) -> bool {
        self.variable_lengths.contains(&local)
    }

    /// Add a local written with the name at `name`, and hand back its id.
    ///
    /// A method of its own rather than a parameter on [`Function::push_local`],
    /// whose callers are nearly all the compiler's own temporaries, which have
    /// no name to give.
    pub fn push_named_local(&mut self, ty: TyId, name: Span) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(ty);
        self.names.push(Some(name));
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

    /// Send each branch that leads into an empty re-branch on the same local
    /// straight to where that second branch sends the same value.
    ///
    /// **The shape a condition's `&&` or `||` leaves**: the operand that
    /// short-circuits jumps to a block whose only content is a branch on the
    /// operator's result, which the first branch has just decided. Joined
    /// there, the path that knows a pointer is not null and the path that
    /// knows nothing meet before the second branch, and the second branch
    /// cannot tell them apart: `if (p != 0 && q != 0) return *p;` doubted
    /// `*p` (#592).
    ///
    /// **Sound because nothing can happen between the two**: the first branch
    /// is its block's terminator and the second block has no elements, so the
    /// local both read holds the same value at each, and the second goes the
    /// way the first already went. Only a branch on a plain local, into a
    /// block with no elements whose terminator is a branch on the same plain
    /// local. The block passed over stays, for its other predecessors.
    /// **The same local is held by a unit test and by no C program**: the
    /// lowering branches on a temporary it computes in the same block, so an
    /// empty block that branches again is only ever the operator's own join,
    /// on its own result. Built by hand, a block that branches on another
    /// local is left alone.
    ///
    /// Repeated until nothing moves, which a chain of such blocks needs, for
    /// at most as many passes as there are blocks, which is what ends it
    /// whatever the shape: two such blocks sending each other round would
    /// move an edge between them on every pass. No C program builds one,
    /// since a loop's test computes into its block before it branches, so
    /// the bound is never what stops it on a program written here.
    pub fn thread_rebranches(&mut self) {
        let blocks = self.defined_mut();
        for _ in 0..blocks.len() {
            let mut moved = false;
            for index in 0..blocks.len() {
                let Some(Block {
                    terminator:
                        Terminator::Branch {
                            condition: Operand::Copy(tested),
                            then,
                            otherwise,
                            ..
                        },
                    ..
                }) = &blocks[index]
                else {
                    continue;
                };
                if !tested.projection.is_empty() {
                    continue;
                }
                let local = tested.local;
                let rebranch = |target: BlockId, taken: bool| match &blocks[target.index()] {
                    Some(Block {
                        elements,
                        terminator:
                            Terminator::Branch {
                                condition: Operand::Copy(again),
                                then,
                                otherwise,
                                ..
                            },
                    }) if elements.is_empty()
                        && again.projection.is_empty()
                        && again.local == local =>
                    {
                        Some(if taken { *then } else { *otherwise })
                    }
                    _ => None,
                };
                let (then, otherwise) = (*then, *otherwise);
                let new_then = rebranch(then, true).unwrap_or(then);
                let new_otherwise = rebranch(otherwise, false).unwrap_or(otherwise);
                if new_then == then && new_otherwise == otherwise {
                    continue;
                }
                if let Some(Block {
                    terminator:
                        Terminator::Branch {
                            then, otherwise, ..
                        },
                    ..
                }) = &mut blocks[index]
                {
                    *then = new_then;
                    *otherwise = new_otherwise;
                }
                moved = true;
            }
            if !moved {
                return;
            }
        }
    }

    /// The type of the local `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Function`].
    pub fn local(&self, id: LocalId) -> TyId {
        self.locals[id.index()]
    }

    /// The span of the name `id` was written with, or `None` for a local the
    /// source never wrote.
    pub fn name_of(&self, id: LocalId) -> Option<Span> {
        self.names[id.index()]
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
    /// The objects with static storage duration, by [`StaticId`].
    statics: Vec<Static>,
    types: Vec<Ty>,
    /// So that one type has one id. Not part of the shape: an implementation
    /// detail of [`Self::push_type`].
    interned: HashMap<Ty, TyId>,
    /// Every struct's layout, by [`StructId`], or `None` for one declared
    /// and not completed: a struct nothing in this unit defines, reached
    /// only through pointers, and one whose members are being laid out.
    structs: Vec<Option<StructLayout>>,
    /// Which of [`Self::structs`] are unions. See [`Ty::Struct`].
    unions: HashSet<StructId>,
    /// Every function type's signature, by [`SignatureId`].
    signatures: Vec<Signature>,
    /// So that one signature has one id, as [`Self::interned`] is for types.
    interned_signatures: HashMap<Signature, SignatureId>,
}

impl TranslationUnit {
    /// An empty unit, for that machine.
    pub fn new(target: Target) -> Self {
        Self {
            target,
            functions: Vec::new(),
            statics: Vec::new(),
            types: Vec::new(),
            interned: HashMap::new(),
            structs: Vec::new(),
            unions: HashSet::new(),
            signatures: Vec::new(),
            interned_signatures: HashMap::new(),
        }
    }

    /// The id of this signature, the one it already has if it has one.
    pub fn push_signature(&mut self, signature: Signature) -> SignatureId {
        if let Some(id) = self.interned_signatures.get(&signature) {
            return *id;
        }
        let id = SignatureId(self.signatures.len() as u32);
        self.signatures.push(signature.clone());
        self.interned_signatures.insert(signature, id);
        id
    }

    /// The signature `id` names.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`TranslationUnit`].
    pub fn signature(&self, id: SignatureId) -> &Signature {
        &self.signatures[id.index()]
    }

    /// The function type of `function`, which [`Operand::Function`] points
    /// at: built from its declaration, so a pointer to it is converted as
    /// the function it names was declared.
    pub fn designator_signature(&self, function: FuncId) -> Signature {
        let declared = &self.functions[function.0 as usize];
        Signature {
            returns: declared.locals[0],
            parameters: declared
                .parameters()
                .enumerate()
                .map(|(index, local)| (declared.locals[local.index()], declared.passes(index)))
                .collect(),
            prototyped: declared.prototyped,
            variadic: declared.variadic,
        }
    }

    /// Lay out a struct of these members, in order, and say which it is.
    /// `None` where a member has no size or alignment this unit can give,
    /// which the lowering refuses before it asks. The layout is
    /// [`record::lay_out`]'s.
    pub fn push_struct(&mut self, members: &[TyId]) -> Option<StructId> {
        let id = self.declare_struct();
        self.complete_struct(id, members)?;
        Some(id)
    }

    /// Lay out a union of these members, and say which it is: as
    /// [`Self::push_struct`], by [`record::lay_out`].
    pub fn push_union(&mut self, members: &[TyId]) -> Option<StructId> {
        let id = self.declare_union();
        self.complete_struct(id, members)?;
        Some(id)
    }

    /// A union with no members yet: as [`Self::declare_struct`], and laid
    /// out by [`record::lay_out`] when [`Self::complete_struct`] gives it
    /// members.
    pub fn declare_union(&mut self) -> StructId {
        let id = self.declare_struct();
        self.unions.insert(id);
        id
    }

    /// Whether `id` is a union: every member at offset 0, and spelled
    /// `union` where it is printed. See [`Ty::Struct`].
    pub fn is_union(&self, id: StructId) -> bool {
        self.unions.contains(&id)
    }

    /// A struct with no members yet, so that a pointer to it can be a type
    /// before its members are laid out: `struct L { struct L *next; }` is a
    /// pointer to `L` inside `L` (C17 6.7.2.3 p8), and one nothing in this
    /// unit defines is only ever pointed at. Its size and alignment are
    /// `None` until [`Self::complete_struct`] gives it members.
    pub fn declare_struct(&mut self) -> StructId {
        let id = StructId(u32::try_from(self.structs.len()).expect("fewer than 2^32 structs"));
        self.structs.push(None);
        id
    }

    /// Lay out the declared struct `id` with these members, in order, by
    /// [`record::lay_out`], as a union where it was declared one.
    /// `None` where a member has no size or alignment this unit can give, the
    /// struct staying incomplete.
    ///
    /// # Panics
    ///
    /// If `id` already has members: one struct is laid out once.
    pub fn complete_struct(&mut self, id: StructId, members: &[TyId]) -> Option<()> {
        let members: Vec<Declared> = members
            .iter()
            .map(|&ty| Declared {
                ty,
                width: None,
                named: true,
            })
            .collect();
        self.complete_record(id, &members)
    }

    /// Lay out the declared struct `id` with these members, bit-fields
    /// among them, by [`record::lay_out`]: as [`Self::complete_struct`].
    ///
    /// # Panics
    ///
    /// If `id` already has members.
    pub fn complete_record(&mut self, id: StructId, members: &[Declared]) -> Option<()> {
        assert!(
            self.structs[id.0 as usize].is_none(),
            "struct {} is already laid out",
            id.0
        );
        let fields = members
            .iter()
            .map(|member| {
                Some(record::Field {
                    size: self.size_of(member.ty)?,
                    align: self.align_of(member.ty)?,
                    width: member.width,
                    named: member.named,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let laid = record::lay_out(self.target, self.is_union(id), &fields)?;
        let mut placed = Vec::with_capacity(members.len());
        for (member, &bit) in members.iter().zip(&laid.bits) {
            placed.push(match member.width {
                None => MemberLayout {
                    ty: member.ty,
                    offset: u64::try_from(bit / 8).expect("an offset inside the whole"),
                    bits: None,
                },
                Some(width) => {
                    let bit = u64::try_from(bit).ok()?;
                    // An unnamed bit-field is read by nothing, as a
                    // zero-width one is, and may be wider than the whole
                    // around it: `union { char a : 5; long long : 11; }`
                    // takes one byte on Itanium (#1206 review).
                    let reads = if member.named { width } else { 0 };
                    let (offset, unit) = access_unit(bit, reads, laid.size, laid.align);
                    let rank = [Rank::Char, Rank::Short, Rank::Int, Rank::LongLong]
                        .into_iter()
                        .find(|&rank| {
                            u64::from(self.target.integer(rank, Sign::Unsigned).bits()) == unit * 8
                        })
                        .expect("every target has an integer of 1, 2, 4 and 8 bytes");
                    MemberLayout {
                        ty: self.push_type(Ty::Integer {
                            rank,
                            sign: Sign::Unsigned,
                        }),
                        offset,
                        bits: Some(Bits {
                            at: u32::try_from(bit - offset * 8).expect("within a unit of 8 bytes"),
                            width: u32::try_from(width).expect("no wider than its type"),
                        }),
                    }
                }
            });
        }
        self.structs[id.0 as usize] = Some(StructLayout {
            members: placed,
            size: laid.size,
            align: laid.align,
        });
        Some(())
    }

    /// The layout of the struct `id` names, where it is complete.
    pub fn layout_of(&self, id: StructId) -> Option<&StructLayout> {
        self.structs[id.0 as usize].as_ref()
    }

    /// The layout of the complete struct `id` names: what an object, a
    /// member or a size is asked of, none of which an incomplete struct has.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`TranslationUnit`], or names a struct
    /// with no members: the type check refuses an object of one, so an
    /// object of one reaching here is a gap it left.
    pub fn struct_layout(&self, id: StructId) -> &StructLayout {
        self.layout_of(id)
            .unwrap_or_else(|| panic!("struct {} is laid out before it is asked", id.0))
    }

    /// Where member `member` of the struct `id` starts, in bytes. See
    /// ADR-0124: the one answer to it, which the backend writes as a byte
    /// offset and the memory check reads as a position.
    ///
    /// # Panics
    ///
    /// If the struct has no such member.
    pub fn offset_of(&self, id: StructId, member: u32) -> u64 {
        self.struct_layout(id).members[member as usize].offset
    }

    /// The alignment in bytes of an object of this type on this unit's
    /// target, or `None` for `void`. An array is its element's, and a struct
    /// its strictest member's (C17 6.7.2.1 p15).
    pub fn align_of(&self, id: TyId) -> Option<u64> {
        match self.ty(id) {
            Ty::Integer { rank, .. } => Some(self.target.alignment(Some(rank))),
            Ty::Pointer(_) => Some(self.target.alignment(None)),
            Ty::Void => None,
            Ty::Array { element, .. } => self.align_of(element),
            Ty::Struct(layout) => Some(self.layout_of(layout)?.align),
            // A function is no object, so it has none (C17 6.5.3.4 p1 keeps
            // `_Alignof` from a function type).
            Ty::Function(_) => None,
        }
    }

    /// Add an object with static storage duration and say where it went.
    pub fn push_static(&mut self, object: Static) -> StaticId {
        let id = StaticId(u32::try_from(self.statics.len()).expect("fewer than 2^32 statics"));
        self.statics.push(object);
        id
    }

    /// Give the object `id` the value it starts with, once every object an
    /// address in it may name has an id: the C frontend works out a
    /// file-scope initializer after it has made every file-scope object.
    pub fn set_static_init(&mut self, id: StaticId, init: StaticInit) {
        self.statics[id.index()].init = init;
    }

    /// The object with static storage duration an id names.
    pub fn static_object(&self, id: StaticId) -> &Static {
        &self.statics[id.index()]
    }

    /// Every object with static storage duration, in the order they were
    /// added.
    pub fn statics(&self) -> impl Iterator<Item = StaticId> + '_ {
        (0..self.statics.len())
            .map(|index| StaticId(u32::try_from(index).expect("fewer than 2^32 statics")))
    }

    /// The machine this unit is for.
    ///
    /// See ADR-0013 for why it is here rather than in the backend. A
    /// pointer's width is among what it says since ADR-0060.
    pub fn target(&self) -> Target {
        self.target
    }

    /// What this type is worth on this unit's target, if it is an integer.
    ///
    /// `None` for `void` and for a pointer, which are the two that are not
    /// integers: a `void` is not a value, and a pointer is not converted or
    /// range-checked as one. What a pointer takes in memory is
    /// [`Self::size_of`]'s to say. A caller that wants to convert or to check
    /// a range has to say what it does about those, which is the point of
    /// answering `Option` rather than panicking.
    pub fn integer(&self, id: TyId) -> Option<Integer> {
        match self.ty(id) {
            Ty::Integer { rank, sign } => Some(self.target.integer(rank, sign)),
            // No array, struct or function is a value, so none is converted
            // or range-checked.
            Ty::Void | Ty::Pointer(_) | Ty::Array { .. } | Ty::Struct(_) | Ty::Function(_) => None,
        }
    }

    /// Whether an lvalue of type `access` may reach an object of type
    /// `object`, by C17 6.5 p7's first items: the type itself, or the signed
    /// or unsigned type corresponding to it, which is the one of equal rank
    /// (6.2.5 p6). The character types are its last item, and a caller asks
    /// them apart, as bytes.
    ///
    /// Guarded by `an_integer_is_reached_through_its_counterpart_and_no_other_rank`.
    /// Nothing the memory check reports turns on it yet: an integer local is
    /// never asked whether it was written, and a write that lands on one
    /// object is placed whatever its type.
    pub fn same_or_counterpart(&self, access: TyId, object: TyId) -> bool {
        access == object
            || match (self.ty(access), self.ty(object)) {
                (Ty::Integer { rank: left, .. }, Ty::Integer { rank: right, .. }) => left == right,
                _ => false,
            }
    }

    /// How many bytes an object of this type takes on this unit's target, or
    /// `None` for `void`, which is not an object type, and for an array too
    /// large to count in a `u64`.
    ///
    /// What the memory check compares an access with, so that a pointer
    /// written into four bytes is asked about eight where pointers take eight.
    /// A `match` rather than a fallback, so that a kind of type added later is
    /// `error[E0004]` here until it says what it is worth. See ADR-0060.
    pub fn size_of(&self, id: TyId) -> Option<u64> {
        let bits = match self.ty(id) {
            Ty::Integer { rank, sign } => self.target.integer(rank, sign).bits(),
            Ty::Pointer(_) => self.target.pointer_bits(),
            Ty::Void => return None,
            // `length` elements and no padding between them, C17 6.5.3.4 p7.
            // `None` on overflow, which the memory check reads as an object
            // it knows nothing of the size of, and so `Unknown`; and `None`
            // for a variable length array, whose size is its length local's
            // value, known only when it runs (#1209). See `Length`.
            Ty::Array { element, length } => {
                return self.size_of(element)?.checked_mul(length.constant()?);
            }
            // Its members and their padding, laid out once.
            Ty::Struct(layout) => return Some(self.layout_of(layout)?.size),
            // A function is no object, and `sizeof` of one is a constraint
            // violation (C17 6.5.3.4 p1), so the memory check measures none.
            Ty::Function(_) => return None,
        };
        // In C's bytes, which are `char`s (C17 6.5.3.4 p4), so a `char` is
        // one whatever its width.
        Some(u64::from(
            bits / self.target.integer(Rank::Char, Sign::Plain).bits(),
        ))
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
                // The element an index selects. The lowering builds one only
                // as a constant zero under `Rvalue::Address`, the conversion of
                // a local array to a pointer to its first element; `p[i]` is
                // still `*(p + i)`. See `docs/c-family.md`.
                (Projection::Index(_), Ty::Array { element, .. }) => element,
                (Projection::Index(_), _) => return None,
                // The member's type, which the layout keeps.
                (Projection::Field(member), Ty::Struct(layout)) => {
                    self.layout_of(layout)?.members.get(*member as usize)?.ty
                }
                (Projection::Field(_), _) => return None,
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
