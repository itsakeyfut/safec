//! Writing a translation unit out as textual LLVM IR.
//!
//! One `alloca` per local, and a `load` or a `store` around every use of one.
//! That is one-to-one with the Safety IR, so the two artifacts can be read side
//! by side, and it needs no dominance and no `phi`, which would be Phase 4's
//! dataflow framework written a phase early. `mem2reg` is what turns these back
//! into registers, and optimisation is somebody else's job.
//!
//! What this refuses is what the interpreter refuses, so that two consumers of
//! one IR do not disagree about which programs it can express, **except
//! arrays, pointer arithmetic and ordering two pointers**: this writes all
//! three (#621, #1211), and the interpreter, whose pointer is a local with no
//! element in it, refuses them until it is given arrays of its own. Where the *reason* for a refusal is
//! the same it is said in the same words, and where it is not it is not.

use safec_ir::ir::{Rank, Sign};
use std::fmt::Write as _;

use crate::abi::{self, Class, Piece};
use safec_ir::ir::{
    BinOp, BlockId, Called, Element, FuncId, Function, Length, Linkage, LocalId, Operand,
    Operation, Passes, Place, Projection, Rvalue, Signature, StaticId, StaticInit, Terminator,
    TranslationUnit, Ty, TyId, UnOp,
};

/// What a call passes and returns, as the IR lays it out: the one shape
/// [`Emitter::crossings`] and [`Emitter::call`] read, whether the callee is
/// a function this unit names or one a pointer points at (#1212).
struct Callable {
    /// What it returns, after a struct result is moved into a parameter.
    returns: TyId,
    /// Each parameter's type and how it is passed.
    parameters: Vec<(TyId, Passes)>,
}

impl Callable {
    /// A function's, from its locals.
    fn of(function: &Function) -> Self {
        Self {
            returns: function.local(function.return_place()),
            parameters: function
                .parameters()
                .enumerate()
                .map(|(index, local)| (function.local(local), function.passes(index)))
                .collect(),
        }
    }

    /// A function type's, from its signature, which the lowering lays out
    /// as it lays out a function's own parameters.
    fn signed(signature: &Signature) -> Self {
        Self {
            returns: signature.returns,
            parameters: signature.parameters.clone(),
        }
    }
}
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::{Extend, Integer, Target};

/// Something the IR can express and this backend cannot.
///
/// The shape [`Trap`] has for the interpreter, and for the same reason: this
/// crate cannot see `Diagnostic`, which lives in `safec-c`. Whoever calls
/// [`functions`] turns one of these into whatever it reports with.
///
/// [`Trap`]: safec_ir::interp::Trap
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// What could not be written, as a sentence a reader can act on.
    pub why: String,
    /// Where, when this file has one to give.
    ///
    /// A call's span is the only one a refusal spends, so this is `None` more
    /// often than a diagnostic would like. A branch carries one too, and it is
    /// deliberately not read: what a backend refuses is a shape, and the
    /// expression that decided which way control went is not what it could not
    /// write. What the IR knows is in `ir.rs`.
    pub at: Option<Span>,
}

/// That a refusal was recorded, which is the only way a step of writing may
/// give up.
///
/// A function here that could not write what it was asked answers
/// `Err(Refused)`, and its caller writes a `declare` in place of the
/// definition. A `declare` with no recorded refusal behind it is a run that
/// exits 0 having dropped a function's body, so the field is private to this
/// module and [`record`](refused::record) is the one thing that makes one.
/// Writing `Refused(())` outside it does not compile: `error[E0423]` once
/// imported, `error[E0603]` through its path. Making the field `pub(super)`
/// compiles, and is the change that would bring that silence back.
///
/// `record` takes the [`Emitter`] rather than a list, so that what it pushes
/// to is the list [`functions`] hands back: given any `Vec`, a fresh one would
/// make a `Refused` that nobody ever reads.
mod refused {
    pub(super) struct Refused(());

    /// Keep `refusal` in what `emitter` hands back, and answer the proof that
    /// it was kept.
    pub(super) fn record(emitter: &mut super::Emitter<'_>, refusal: super::Refusal) -> Refused {
        emitter.refusals.push(refusal);
        Refused(())
    }
}

use refused::Refused;

/// What a step of writing answers: what it wrote, or that it refused and said
/// why.
type Refuses<T> = Result<T, Refused>;

/// The line every module begins with, and which may appear only once in one.
///
/// Separate from [`functions`] because an artifact holds **one** module however
/// many translation units were appended to it: two `target triple` lines is not
/// a module with a duplicate, it is a parse error at the second one. So this is
/// written once, where the artifact is created, and `functions` is called once
/// per input.
///
/// Every unit appended after it has to be for this target. The driver is what
/// makes that true, by lowering every input for `--target`.
///
/// The triple is written with `{:?}`, the way `print::dump_ir` writes the same
/// string. It is one of a fixed table of measured ASCII triples rather than
/// anything a file said, so nothing here is source text that could carry an
/// escape sequence.
pub fn header(target: Target) -> String {
    format!("target triple = {:?}\n", target.triple())
}

/// Append this unit's functions to a module.
///
/// Every function that can be written is, whatever the ones beside it did. A
/// function this cannot express becomes a `declare` and one or more
/// [`Refusal`]s, so the module is still something LLVM accepts and a caller of
/// that function still reads. A program is not one thing that fails, which is
/// the same reason the frontend's lowering carries on past a function it could
/// not build.
pub fn functions(sources: &SourceMap, unit: &TranslationUnit, out: &mut String) -> Vec<Refusal> {
    let mut emitter = Emitter {
        sources,
        unit,
        refusals: Vec::new(),
        next_temp: 0,
        at: None,
        returned_in: None,
        stack_saved: false,
    };

    for id in unit.statics() {
        emitter.global(id, out);
    }

    for id in unit.functions() {
        emitter.function(id, out);
    }

    // Once a variable length array has been made: the module holds one
    // unit, so once here is once in the module.
    if emitter.stack_saved {
        out.push_str("\ndeclare ptr @llvm.stacksave.p0()\n");
        out.push_str("declare void @llvm.stackrestore.p0(ptr)\n");
    }

    emitter.refusals
}

/// Whether `ty` is a variable length array, whose length is a local.
fn is_variable(unit: &TranslationUnit, ty: TyId) -> bool {
    matches!(
        unit.ty(ty),
        Ty::Array {
            length: Length::Local(_),
            ..
        }
    )
}

/// What one [`BinOp`] becomes, which is two different shapes.
///
/// A comparison answers `i1` and has to be widened to the destination's type;
/// everything else answers that type already. One `match` returning this,
/// rather than two matches that have to agree about which operators are which:
/// a variant added to [`BinOp`] is then `error[E0004]` in one place.
#[derive(Clone, Copy)]
enum Spelled {
    /// An `icmp` predicate.
    Compare(&'static str),
    /// An instruction, with whatever flags it carries.
    Compute(&'static str),
}

/// What is being written, and what it could not write.
struct Emitter<'a> {
    sources: &'a SourceMap,
    unit: &'a TranslationUnit,
    refusals: Vec<Refusal>,
    /// Numbered per function, so that two functions do not share a name and
    /// neither depends on how much came before it.
    next_temp: u32,
    /// Where the element being written is, for whatever it refuses.
    ///
    /// Held here rather than threaded through a dozen signatures, because it
    /// is one fact about the element rather than an argument to each step of
    /// writing it. Set in exactly two places, both of which are the only two
    /// shapes in the IR that carry an `Origin`: an operation and a call.
    at: Option<Span>,
    /// Where the function being written puts a struct it returns in
    /// registers, and the pieces it returns it as, set by the prologue
    /// `Emitter::body` writes for each function and read at its `return`s.
    returned_in: Option<(String, Vec<Piece>, bool)>,
    /// Whether a variable length array's storage was made, so the module
    /// needs `llvm.stacksave` and `llvm.stackrestore` declared (#1209).
    stack_saved: bool,
}

/// What a struct parameter or result crosses a call as, with the struct's
/// size and alignment: see [`abi::classify`].
type Crossing = Option<(Class, u64, u64)>;

/// The layout of the struct a parameter of type `pointer` points at, or
/// `None` where it is incomplete or not a struct.
fn pointed_at_struct(unit: &TranslationUnit, pointer: TyId) -> Option<&safec_ir::ir::StructLayout> {
    let Ty::Pointer(pointee) = unit.ty(pointer) else {
        return None;
    };
    let Ty::Struct(id) = unit.ty(pointee) else {
        return None;
    };
    unit.layout_of(id)
}

/// Whether a struct laid out as `layout` holds a flexible array member, an
/// array of none (C17 6.7.2.1 p18), as its own last member or in a member
/// struct or union, however deep: what Win64 and System V pass in memory
/// whatever the size (#1208 review). A recursion over members by value,
/// which no struct holds itself through.
fn holds_flexible_array(unit: &TranslationUnit, layout: &safec_ir::ir::StructLayout) -> bool {
    layout
        .members
        .iter()
        .any(|member| match unit.ty(member.ty) {
            Ty::Array {
                length: Length::Constant(0),
                ..
            } => true,
            Ty::Struct(inner) => unit
                .layout_of(inner)
                .is_some_and(|inner| holds_flexible_array(unit, inner)),
            _ => false,
        })
}

/// Whether `function` takes or returns a struct by value that is never
/// completed. See `Emitter::function`.
fn takes_an_incomplete_struct(unit: &TranslationUnit, function: &Function) -> bool {
    function.parameters().enumerate().any(|(index, local)| {
        function.passes(index) != Passes::Value
            && pointed_at_struct(unit, function.local(local)).is_none()
    })
}

/// The alignment a `byval` argument is marked with: the struct's own, but
/// at least 8, which System V gives every argument on the stack and `clang`
/// writes.
fn byval_align(align: u64) -> u64 {
    align.max(8)
}

impl Emitter<'_> {
    /// Record what could not be written, and answer that it was refused.
    ///
    /// Generic in the answer so that a caller can `return self.refuse(...)`
    /// out of whatever it was computing.
    fn refuse<T>(&mut self, why: impl Into<String>) -> Refuses<T> {
        Err(refused::record(
            self,
            Refusal {
                why: why.into(),
                at: self.at,
            },
        ))
    }

    /// How each parameter of `function` crosses a call, `None` for one that
    /// is not a struct: see [`abi::classify`]. Refused where a struct is
    /// incomplete, which C allows in a declaration (C17 6.7.6.3 p4) and
    /// nobody can pass or return, or where the target's C ABI is not written
    /// here yet.
    fn crossings(&mut self, callable: &Callable) -> Refuses<Vec<Crossing>> {
        let unit = self.unit;
        let mut layouts = Vec::new();
        for &(ty, passes) in &callable.parameters {
            if passes == Passes::Value {
                layouts.push(None);
                continue;
            }
            let Some(layout) = pointed_at_struct(unit, ty) else {
                return self
                    .refuse("a struct passed or returned by value whose type is incomplete");
            };
            layouts.push(Some((passes, layout)));
        }
        let parameters: Vec<abi::Parameter> = layouts
            .iter()
            .map(|layout| match layout {
                None => abi::Parameter::Scalar,
                Some((passes, layout)) => {
                    abi::Parameter::Struct(*passes, layout, holds_flexible_array(unit, layout))
                }
            })
            .collect();
        let Some(classes) = abi::classify(unit.target(), &parameters) else {
            return self.refuse(format!(
                "a struct passed or returned by value on `{}`, whose C ABI this backend does not write yet",
                unit.target().triple()
            ));
        };
        Ok(classes
            .into_iter()
            .zip(&layouts)
            .map(|(class, layout)| {
                class
                    .zip(layout.as_ref())
                    .map(|(class, (_, layout))| (class, layout.size, layout.align))
            })
            .collect())
    }

    /// The pieces of the struct at `from`, each read as the bytes it carries
    /// and widened to its register.
    ///
    /// **Read at alignment 1**: a struct of `char`s is 1-aligned and its
    /// piece an `i64`, and a load that says nothing is one LLVM may assume is
    /// aligned to its type. **Read as the bytes it carries**, so nothing past
    /// the struct's last byte is read for a piece wider than what is left.
    fn read_pieces(&mut self, from: &str, pieces: &[Piece], out: &mut String) -> Vec<String> {
        let mut values = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let at = self.temp();
            writeln!(
                out,
                "  {at} = getelementptr i8, ptr {from}, i64 {}",
                piece.at
            )
            .expect("writing to a string cannot fail");
            let value = self.temp();
            writeln!(
                out,
                "  {value} = load {}, ptr {at}, align 1",
                piece.carried()
            )
            .expect("writing to a string cannot fail");
            if piece.carried() == piece.spelled() {
                values.push(value);
            } else {
                let wide = self.temp();
                writeln!(
                    out,
                    "  {wide} = zext {} {value} to {}",
                    piece.carried(),
                    piece.spelled()
                )
                .expect("writing to a string cannot fail");
                values.push(wide);
            }
        }
        values
    }

    /// Write `values`, one per piece, into the struct at `into`: the
    /// inverse of [`Emitter::read_pieces`], each narrowed to the bytes it
    /// carries.
    fn write_pieces(&mut self, into: &str, pieces: &[Piece], values: &[String], out: &mut String) {
        for (piece, value) in pieces.iter().zip(values) {
            let narrow = if piece.carried() == piece.spelled() {
                value.clone()
            } else {
                let narrow = self.temp();
                writeln!(
                    out,
                    "  {narrow} = trunc {} {value} to {}",
                    piece.spelled(),
                    piece.carried()
                )
                .expect("writing to a string cannot fail");
                narrow
            };
            let at = self.temp();
            writeln!(
                out,
                "  {at} = getelementptr i8, ptr {into}, i64 {}",
                piece.at
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "  store {} {narrow}, ptr {at}, align 1",
                piece.carried()
            )
            .expect("writing to a string cannot fail");
        }
    }

    /// One value of [`Class::aggregate`]'s type holding `values`.
    fn pack(
        &mut self,
        pieces: &[Piece],
        together: bool,
        values: Vec<String>,
        out: &mut String,
    ) -> String {
        if let [one] = &values[..] {
            return one.clone();
        }
        let spelled = Class::aggregate(pieces, together);
        let mut packed = "undef".to_owned();
        for (index, (piece, value)) in pieces.iter().zip(values).enumerate() {
            let next = self.temp();
            writeln!(
                out,
                "  {next} = insertvalue {spelled} {packed}, {} {value}, {index}",
                piece.spelled()
            )
            .expect("writing to a string cannot fail");
            packed = next;
        }
        packed
    }

    /// The values one of [`Class::aggregate`]'s type holds, one per piece.
    fn unpack(
        &mut self,
        pieces: &[Piece],
        together: bool,
        value: &str,
        out: &mut String,
    ) -> Vec<String> {
        if pieces.len() == 1 {
            return vec![value.to_owned()];
        }
        let spelled = Class::aggregate(pieces, together);
        let mut values = Vec::with_capacity(pieces.len());
        for index in 0..pieces.len() {
            let piece = self.temp();
            writeln!(out, "  {piece} = extractvalue {spelled} {value}, {index}")
                .expect("writing to a string cannot fail");
            values.push(piece);
        }
        values
    }

    /// A name no other value in this function has.
    fn temp(&mut self) -> String {
        let name = format!("%t{}", self.next_temp);
        self.next_temp += 1;
        name
    }

    /// How LLVM spells this type.
    ///
    /// A pointer is `ptr` and says nothing about what it points at, which is
    /// what LLVM 15 and later have and what ADR-0013 leaves the IR free to do:
    /// a pointer has no width here and nothing needs one.
    fn spell(&self, id: TyId) -> String {
        match self.unit.ty(id) {
            Ty::Integer { rank, sign } => {
                format!("i{}", self.unit.target().integer(rank, sign).bits())
            }
            Ty::Void => "void".to_owned(),
            Ty::Pointer(_) => "ptr".to_owned(),
            Ty::Array {
                element,
                length: Length::Constant(length),
            } => format!("[{length} x {}]", self.spell(element)),
            // **A variable length array has no LLVM type, and none is
            // asked** (#1209): its local is a pointer to storage made where
            // it is declared (`Emitter::begin_variable`), spelled by `body`
            // without asking here, and an index into it starts at that
            // pointer. Anything else that asks, a step of a pointer to one
            // above all, would be given a size that is not its own, so it
            // stops here rather than being given `ptr`'s.
            Ty::Array {
                length: Length::Local(_),
                ..
            } => unreachable!("a variable length array's type is spelled nowhere"),
            // **Bytes, and no LLVM struct type**: the IR lays a struct out and
            // nothing else does, so its storage is its size in bytes and a
            // member is reached at its offset into them. See ADR-0124.
            Ty::Struct(layout) => format!("[{} x i8]", self.unit.struct_layout(layout).size),
            // **A function is no value, so its type is spelled nowhere**: a
            // pointer to one is `ptr`, and a call through one spells its
            // parameters one by one (`Self::call`).
            Ty::Function(_) => unreachable!("a function's type is spelled nowhere"),
        }
    }

    /// `, align N` for storage of a struct type, or an array of them, from the
    /// layout, and nothing for any other. **A struct is bytes to LLVM**, which aligns `[N x i8]`
    /// to one, so its alignment is said wherever its storage is made, a
    /// local's and a global's alike: a global left at one put an 8-byte
    /// member at an address 1 mod 8 (#1200 review). Every other type is
    /// one LLVM already aligns as the target does. Mutation: answer nothing
    /// for a struct; `a_struct_global_is_aligned_as_its_layout_says`
    /// loses its `align 8`.
    fn struct_align(&self, ty: TyId) -> String {
        match self.unit.ty(ty) {
            // One nothing defines is declared as `i8`, by `Self::global`.
            Ty::Struct(layout) => self
                .unit
                .layout_of(layout)
                .map_or_else(String::new, |layout| format!(", align {}", layout.align)),
            // An array of them is aligned as its element (#1202).
            Ty::Array { element, .. } => self.struct_align(element),
            Ty::Integer { .. } | Ty::Void | Ty::Pointer(_) | Ty::Function(_) => String::new(),
        }
    }

    /// How LLVM spells what the target asks of this type when it crosses a
    /// call.
    ///
    /// The spelling is all this decides. **Which values a machine widens and
    /// which way is the machine's to say**, and `Target::extension` is where
    /// that lives: a WASM backend and a C backend need the same answer and
    /// write it differently, and three copies of the rule would be three
    /// chances to disagree. `Ty::Void` and a pointer answer no `Integer`, so
    /// they never reach it.
    fn extension(&self, id: TyId) -> Option<&'static str> {
        let value = self.unit.integer(id)?;
        Some(match self.unit.target().extension(value)? {
            Extend::Sign => "signext",
            Extend::Zero => "zeroext",
        })
    }

    /// A result's type, with whatever the ABI asks of it in front.
    ///
    /// In front for a result and after for a parameter, which is where LLVM
    /// puts them and not a choice: `define signext i8 @f(i8 signext %0)`.
    fn result(&self, id: TyId) -> String {
        let spelled = self.spell(id);
        match self.extension(id) {
            Some(extension) => format!("{extension} {spelled}"),
            None => spelled,
        }
    }

    /// A parameter's type, with whatever the ABI asks of it after.
    fn parameter(&self, id: TyId) -> String {
        let spelled = self.spell(id);
        match self.extension(id) {
            Some(extension) => format!("{spelled} {extension}"),
            None => spelled,
        }
    }

    /// A function's name as LLVM takes it.
    ///
    /// Every identifier this project's lexer makes is `[A-Za-z_][A-Za-z0-9_]*`
    /// (`lexer.rs::is_identifier_start`), which is inside LLVM's unquoted set,
    /// so a name out of a C file never needs the quoted form. That is a fact
    /// about one frontend and not about the IR, and the crate boundary is why
    /// it matters: ADR-0011 exists so that a Clang adapter can build a unit,
    /// and a C++ name carries `$`, `.` and `::`.
    ///
    /// So this is the fifth place a name out of a source file reaches a
    /// stream, and every such place has to answer for what the name holds.
    /// `print::shown` is the wrong escape here: LLVM reads `\xx` and nothing
    /// else, so a `\u{1b}` would arrive as six characters of name.
    fn name(&self, function: &Function) -> String {
        let spelled = function.spelled(self.sources);
        if plain(spelled) {
            format!("@{spelled}")
        } else {
            format!("@\"{}\"", escaped(spelled))
        }
    }

    /// An object with static storage duration's symbol, quoted as a function's
    /// name is where it holds what LLVM's unquoted set does not: a `static`
    /// local's name is made unique with a `.`, which that set has.
    fn global_name(&self, id: StaticId) -> String {
        let name = &self.unit.static_object(id).name;
        if plain(name) {
            format!("@{name}")
        } else {
            format!("@\"{}\"", escaped(name))
        }
    }

    /// One object with static storage duration, as a global: defined with what
    /// it holds before the program begins, or declared for another unit to
    /// define. A value this cannot write is a refusal, and the global is then
    /// declared, so a function that reaches it still reads.
    fn global(&mut self, id: StaticId, out: &mut String) {
        self.at = None;
        let object = self.unit.static_object(id);
        let name = self.global_name(id);
        // **A declaration of a struct nothing defines is `i8`**: C allows
        // `extern struct O g;` and `&g` with `O` incomplete (C17 6.2.5 p22,
        // 6.5.3.2 p1), and an external global's type says nothing of its
        // size to LLVM, which only the definition elsewhere has. Spelled from
        // a layout it has none of, this panicked (#1202 review). Mutation:
        // spell it; `an_object_of_a_struct_nothing_defines_is_declared`
        // panics.
        let spelled = if self.unit.size_of(object.ty).is_none() {
            "i8".to_owned()
        } else {
            self.spell(object.ty)
        };
        let init = match &object.init {
            StaticInit::Zero => Ok("zeroinitializer".to_owned()),
            StaticInit::Int(value) => self.constant(*value, object.ty),
            // A string literal's array: a constant no other module names,
            // whose address nothing compares (C17 6.4.5 p7 leaves whether two
            // literals share storage unspecified).
            StaticInit::Bytes(bytes) => {
                writeln!(
                    out,
                    "{name} = private unnamed_addr constant {spelled} c\"{}\"",
                    bytes_spelled(bytes)
                )
                .expect("writing to a string cannot fail");
                return;
            }
        };
        // **A `const` object is a `constant`**, as `clang` writes one, so it
        // lands in read-only memory and a write to it faults as it does in
        // `clang`'s build (C17 6.7.3 p6). See ADR-0120.
        let kind = if object.read_only {
            "constant"
        } else {
            "global"
        };
        let line = match (object.linkage, init) {
            (Linkage::Declared, _) | (_, Err(_)) => {
                format!("{name} = external {kind} {spelled}")
            }
            (Linkage::External, Ok(init)) => format!("{name} = {kind} {spelled} {init}"),
            (Linkage::Internal, Ok(init)) => {
                format!("{name} = internal {kind} {spelled} {init}")
            }
        };
        let line = format!("{line}{}", self.struct_align(object.ty));
        writeln!(out, "{line}").expect("writing to a string cannot fail");
    }

    /// Where a place is, as a `ptr`.
    ///
    /// Every local is an `alloca`, so a place with no projections is already
    /// an address. A `Deref` is one `load` of a pointer. Reading a place,
    /// writing one and taking its address all want this and then differ by a
    /// single instruction.
    fn address(&mut self, function: &Function, place: &Place, out: &mut String) -> Refuses<String> {
        let mut address = format!("%_{}", place.local.index());
        let mut ty = function.local(place.local);
        // **A variable length array is where its local points** (#1209):
        // its storage is made where it is declared, and the local holds the
        // address. Mutation: take the local's own slot; the run of
        // `a_variable_length_array_is_sized_when_declared` fails.
        if is_variable(self.unit, ty) {
            let loaded = self.temp();
            writeln!(out, "  {loaded} = load ptr, ptr {address}")
                .expect("writing to a string cannot fail");
            address = loaded;
        }

        for projection in place.steps() {
            match projection {
                Projection::Deref => match self.unit.ty(ty) {
                    Ty::Pointer(pointee) => {
                        let loaded = self.temp();
                        writeln!(out, "  {loaded} = load ptr, ptr {address}")
                            .expect("writing to a string cannot fail");
                        address = loaded;
                        ty = pointee;
                    }
                    _ => return self.refuse("a dereference of something that is not a pointer"),
                },
                // **An array's first element**, which is the one index the
                // lowering builds: the conversion of a local array to a pointer
                // to its first element (C17 6.3.2.1 p3), under `Rvalue::Address`.
                // `p[i]` is `*(p + i)`, which the arm for pointer arithmetic
                // below writes. Any other index would need its own scaling and
                // nothing builds one; the interpreter refuses every index.
                Projection::Index(Operand::Constant(0)) => match self.unit.ty(ty) {
                    // A variable length array's first element is where it
                    // starts, and it has no type to step through.
                    Ty::Array {
                        element,
                        length: Length::Local(_),
                    } => {
                        ty = element;
                    }
                    Ty::Array { element, .. } => {
                        let spelled = self.spell(ty);
                        let first = self.temp();
                        writeln!(
                            out,
                            "  {first} = getelementptr {spelled}, ptr {address}, i64 0, i64 0"
                        )
                        .expect("writing to a string cannot fail");
                        address = first;
                        ty = element;
                    }
                    _ => return self.refuse("an index into something that is not an array"),
                },
                Projection::Index(_) => {
                    return self.refuse("an index other than an array's first element");
                }
                // **A member is its offset in bytes from its object**, which
                // the IR's layout gives, so the backend lays out nothing. See
                // ADR-0124. Mutation: take every offset as zero;
                // `a_struct_is_laid_out_and_read_through_its_members` exits
                // with another value.
                Projection::Field(member) => match self.unit.ty(ty) {
                    Ty::Struct(layout) => {
                        let offset = self.unit.offset_of(layout, *member);
                        let at = self.temp();
                        writeln!(
                            out,
                            "  {at} = getelementptr i8, ptr {address}, i64 {offset}"
                        )
                        .expect("writing to a string cannot fail");
                        address = at;
                        ty = self.unit.struct_layout(layout).members[*member as usize].ty;
                    }
                    _ => return self.refuse("a member of something that is not a struct"),
                },
            }
        }

        Ok(address)
    }

    /// The value held in a place, and the type it has.
    fn read(
        &mut self,
        function: &Function,
        place: &Place,
        out: &mut String,
    ) -> Refuses<(String, TyId)> {
        let Some(ty) = self.unit.place_ty(function, place) else {
            return self.refuse("a place whose projections do not fit its type");
        };
        if matches!(self.unit.ty(ty), Ty::Void) {
            return self.refuse("a read of a place that holds nothing");
        }
        // C reads an array only by converting it to a pointer to its first
        // element (C17 6.3.2.1 p3), which the lowering does where it is used.
        if matches!(self.unit.ty(ty), Ty::Struct(_)) {
            return self.refuse("a read of a whole struct, which is not lowered yet");
        }
        if matches!(self.unit.ty(ty), Ty::Array { .. }) {
            return self.refuse("a read of a whole array, which C converts rather than reads");
        }

        let address = self.address(function, place, out)?;
        let spelled = self.spell(ty);
        let loaded = self.temp();
        writeln!(out, "  {loaded} = load {spelled}, ptr {address}")
            .expect("writing to a string cannot fail");
        Ok((loaded, ty))
    }

    /// The same value, as the type `to` names.
    ///
    /// C17 6.3.1.3 is what this is: a narrower type widens and a wider one
    /// truncates. Whether the widening carries the sign follows the type being
    /// widened *from*, which is the target's to say and not this crate's.
    ///
    /// Two types LLVM spells the same need no instruction at all, which covers
    /// a pointer to one type becoming a pointer to another, and two integer
    /// types of one width.
    fn convert(
        &mut self,
        value: String,
        from: TyId,
        to: TyId,
        out: &mut String,
    ) -> Refuses<String> {
        let (source, wanted) = (self.spell(from), self.spell(to));
        if source == wanted {
            return Ok(value);
        }

        let (Some(narrow), Some(wide)) = (self.unit.integer(from), self.unit.integer(to)) else {
            return self.refuse(format!("a conversion from `{source}` to `{wanted}`"));
        };
        let instruction = if wide.bits() > narrow.bits() {
            if narrow.signed() { "sext" } else { "zext" }
        } else {
            "trunc"
        };

        let result = self.temp();
        writeln!(
            out,
            "  {result} = {instruction} {source} {value} to {wanted}"
        )
        .expect("writing to a string cannot fail");
        Ok(result)
    }

    /// The operand's bits as `to`, where one of the two is a pointer and the
    /// other an integer (C17 6.3.2.3 p5 and p6): `ptrtoint` and `inttoptr`,
    /// as `clang` spells a cast. An integer narrower than a pointer is
    /// widened first by its own sign, as `clang` does, since `inttoptr`
    /// alone would fill with zeros.
    fn reinterpret(
        &mut self,
        function: &Function,
        from: &Operand,
        to: TyId,
        out: &mut String,
    ) -> Refuses<String> {
        let pointer = format!("i{}", self.unit.target().pointer_bits());
        let (value, from) = match from {
            Operand::Copy(place) => self.read(function, place, out)?,
            // A function pointer's value is its symbol: as another pointer,
            // the symbol as it is, and as an integer, its address.
            Operand::Function(id) => {
                if self.unit.ty(to).is_pointer() {
                    return self.function_address(*id, to);
                }
                let Some(int) = self.unit.integer(to) else {
                    return self.refuse(
                        "a function's address converted to neither a pointer nor an integer",
                    );
                };
                let name = self.name(self.unit.function(*id));
                let result = self.temp();
                writeln!(out, "  {result} = ptrtoint ptr {name} to i{}", int.bits())
                    .expect("writing to a string cannot fail");
                return Ok(result);
            }
            // `(void *)-1`: the integer's bits at a pointer's width, which
            // is what widening it by its sign or its zeros came to when the
            // constant was folded, and `inttoptr` of them.
            Operand::Constant(value) => {
                if !matches!(self.unit.ty(to), Ty::Pointer(_)) {
                    return self.constant(*value, to);
                }
                let bits = self.unit.target().pointer_bits();
                let wrapped = value.rem_euclid(1i128 << bits);
                let result = self.temp();
                writeln!(out, "  {result} = inttoptr {pointer} {wrapped} to ptr")
                    .expect("writing to a string cannot fail");
                return Ok(result);
            }
        };
        let source = self.spell(from);
        let wanted = self.spell(to);
        match (self.unit.integer(from), self.unit.integer(to)) {
            (None, Some(_)) if matches!(self.unit.ty(from), Ty::Pointer(_)) => {
                let result = self.temp();
                writeln!(out, "  {result} = ptrtoint ptr {value} to {wanted}")
                    .expect("writing to a string cannot fail");
                Ok(result)
            }
            (Some(int), None) if matches!(self.unit.ty(to), Ty::Pointer(_)) => {
                let widened = if int.bits() < self.unit.target().pointer_bits() {
                    let widened = self.temp();
                    let instruction = if int.signed() { "sext" } else { "zext" };
                    writeln!(
                        out,
                        "  {widened} = {instruction} {source} {value} to {pointer}"
                    )
                    .expect("writing to a string cannot fail");
                    (widened, pointer)
                } else {
                    (value, source)
                };
                let result = self.temp();
                writeln!(
                    out,
                    "  {result} = inttoptr {} {} to ptr",
                    widened.1, widened.0
                )
                .expect("writing to a string cannot fail");
                Ok(result)
            }
            _ => self.refuse(format!("`{source}` reinterpreted as `{wanted}`")),
        }
    }

    /// One operand, as the type `want` names.
    fn operand(
        &mut self,
        function: &Function,
        operand: &Operand,
        want: TyId,
        out: &mut String,
    ) -> Refuses<String> {
        match operand {
            Operand::Constant(value) => self.constant(*value, want),
            Operand::Copy(place) => {
                let (loaded, from) = self.read(function, place, out)?;
                self.convert(loaded, from, want, out)
            }
            Operand::Function(id) => self.function_address(*id, want),
        }
    }

    /// A function's address, as the pointer type `want` names: its symbol,
    /// which LLVM's `ptr` holds as it is (#1212).
    fn function_address(&mut self, id: FuncId, want: TyId) -> Refuses<String> {
        let function = self.unit.function(id);
        // **One left out of the module has no symbol to take**: see
        // `Self::function`, which declares nothing for it.
        if !function.is_defined() && takes_an_incomplete_struct(self.unit, function) {
            return self.refuse("the address of a function whose struct parameter is incomplete");
        }
        if !self.unit.ty(want).is_pointer() {
            return self
                .refuse("a function's address converted to something that is not a pointer");
        }
        Ok(self.name(function))
    }

    /// A constant, as the type `want` names.
    fn constant(&mut self, value: i128, want: TyId) -> Refuses<String> {
        if let Some(int) = self.unit.integer(want) {
            return Ok(int.convert(value).to_string());
        }
        // C17 6.3.2.3 p3 makes an integer constant expression with the value 0
        // a null pointer constant, and makes nothing else one. A number that is
        // not zero reaches a pointer only through a cast, which this frontend
        // cannot write and a hand-built unit would have to mean something by.
        if matches!(self.unit.ty(want), Ty::Pointer(_)) && value == 0 {
            return Ok("null".to_owned());
        }
        let wanted = self.spell(want);
        self.refuse(format!("the constant {value} where a `{wanted}` is wanted"))
    }

    /// An `i1` from a comparison, as the type the destination wants.
    ///
    /// C17 6.5.8 p6 and 6.5.9 p3 make the result of a relational or an
    /// equality operator an `int`, which is what the lowering gives the
    /// destination. So the bit is widened rather than stored as one.
    fn widen(&mut self, comparison: String, to: TyId, out: &mut String) -> Refuses<String> {
        let wanted = self.spell(to);
        if self.unit.integer(to).is_none() {
            return self.refuse(format!("a comparison written into a `{wanted}`"));
        }

        let bit = self.temp();
        writeln!(out, "  {bit} = {comparison}").expect("writing to a string cannot fail");
        if wanted == "i1" {
            return Ok(bit);
        }

        let widened = self.temp();
        writeln!(out, "  {widened} = zext i1 {bit} to {wanted}")
            .expect("writing to a string cannot fail");
        Ok(widened)
    }

    /// Whether this operand is a pointer, and the type it has if it is.
    fn pointer(&self, function: &Function, operand: &Operand) -> Option<TyId> {
        let Operand::Copy(place) = operand else {
            return None;
        };
        let ty = self.unit.place_ty(function, place)?;
        matches!(self.unit.ty(ty), Ty::Pointer(_)).then_some(ty)
    }

    /// One operand under one operator.
    fn unary(
        &mut self,
        function: &Function,
        op: UnOp,
        operand: &Operand,
        to: TyId,
        out: &mut String,
    ) -> Refuses<String> {
        // `!p` is the one unary operator a pointer has. C17 6.5.3.3 p5 makes
        // `!E` mean `(0 == E)`, which needs no width. The interpreter answers
        // 0 for it because nothing in its model is a null pointer; that is a
        // limit of the model rather than a disagreement about C, and a
        // compiled program has real null pointers to compare against.
        if op == UnOp::Not {
            if let Some(ty) = self.pointer(function, operand) {
                let value = self.operand(function, operand, ty, out)?;
                return self.widen(format!("icmp eq ptr {value}, null"), to, out);
            }
        }

        let wanted = self.spell(to);
        if self.unit.integer(to).is_none() {
            return self.refuse(format!("`{}` of a `{wanted}`", op.name()));
        }
        let value = self.operand(function, operand, to, out)?;

        match op {
            // C17 6.5.3.3 p5 again: `!x` is `(0 == x)`, whatever `x` is.
            UnOp::Not => self.widen(format!("icmp eq {wanted} {value}, 0"), to, out),
            // There is no `neg` instruction: LLVM spells it as a subtraction
            // from zero, and it carries `nsw` on the same terms a `sub` does.
            UnOp::Neg => {
                let signed = self.unit.integer(to).is_some_and(Integer::signed);
                let flag = if signed { " nsw" } else { "" };
                let result = self.temp();
                writeln!(out, "  {result} = sub{flag} {wanted} 0, {value}")
                    .expect("writing to a string cannot fail");
                Ok(result)
            }
            // Nor a `not`: flipping every bit is an `xor` with all ones.
            UnOp::BitNot => {
                let result = self.temp();
                writeln!(out, "  {result} = xor {wanted} {value}, -1")
                    .expect("writing to a string cannot fail");
                Ok(result)
            }
        }
    }

    /// A pointer plus an integer, either way round, or a pointer minus one:
    /// C17 6.5.6 p8, the pointer moved by that many elements.
    ///
    /// **`getelementptr` over the element's type does the scaling**, so this
    /// computes no width. The integer is widened to `i64` first, by its own
    /// type's sign, and negated for a subtraction: one index width for a
    /// constant and a register alike, which `getelementptr` takes on every
    /// target here, a 32-bit one included, and which holds every `int` this
    /// compiler has. It is written without `inbounds`, which would let LLVM
    /// assume an access stays inside its object where the memory check only
    /// concluded `Unknown`.
    ///
    /// A pointer minus a pointer is [`Self::difference`]'s (6.5.6 p9).
    /// Arithmetic on a `void *` is refused, which 6.5.6 p2 forbids and the
    /// type check refuses first.
    fn moved(
        &mut self,
        function: &Function,
        op: BinOp,
        lhs: &Operand,
        rhs: &Operand,
        to: TyId,
        out: &mut String,
    ) -> Refuses<String> {
        let (pointer, count) = match (self.pointer(function, lhs), self.pointer(function, rhs)) {
            (Some(ty), None) => ((lhs, ty), rhs),
            (None, Some(ty)) if op == BinOp::Add => ((rhs, ty), lhs),
            (Some(ty), Some(_)) if op == BinOp::Sub => {
                return self.difference(function, lhs, rhs, ty, to, out);
            }
            (Some(_), Some(_)) => return self.refuse("a pointer plus a pointer"),
            (None, _) => return self.refuse("an integer minus a pointer"),
        };
        let (pointer, ty) = pointer;
        let Ty::Pointer(element) = self.unit.ty(ty) else {
            unreachable!("`Self::pointer` answers only a pointer's type")
        };
        if matches!(self.unit.ty(element), Ty::Void) {
            return self.refuse("arithmetic on a `void *`, which has no element to count");
        }
        if !matches!(self.unit.ty(to), Ty::Pointer(_)) {
            let wanted = self.spell(to);
            return self.refuse(format!("a moved pointer written into a `{wanted}`"));
        }

        let base = self.operand(function, pointer, ty, out)?;
        let mut index = match count {
            // Written as the `i64` it is: a constant is an `int` the source
            // wrote, which every `i64` holds.
            Operand::Constant(value) => match i64::try_from(*value) {
                Ok(value) => value.to_string(),
                Err(_) => return self.refuse(format!("a pointer moved by {value}")),
            },
            Operand::Function(_) => {
                return self.refuse("a pointer moved by a function's address");
            }
            Operand::Copy(place) => {
                let Some(counted) = self.unit.place_ty(function, place) else {
                    return self.refuse("a place whose projections do not fit its type");
                };
                let Some(int) = self.unit.integer(counted) else {
                    return self.refuse("a pointer moved by something that is not an integer");
                };
                let mut index = self.operand(function, count, counted, out)?;
                if int.bits() < 64 {
                    let widened = self.temp();
                    let how = if int.signed() { "sext" } else { "zext" };
                    writeln!(out, "  {widened} = {how} i{} {index} to i64", int.bits())
                        .expect("writing to a string cannot fail");
                    index = widened;
                }
                index
            }
        };
        if op == BinOp::Sub {
            let negated = self.temp();
            writeln!(out, "  {negated} = sub i64 0, {index}")
                .expect("writing to a string cannot fail");
            index = negated;
        }
        let spelled = self.spell(element);
        let result = self.temp();
        writeln!(
            out,
            "  {result} = getelementptr {spelled}, ptr {base}, i64 {index}"
        )
        .expect("writing to a string cannot fail");
        Ok(result)
    }

    /// A pointer minus a pointer, C17 6.5.6 p9: how many elements apart
    /// they are, as `ptrdiff_t`, which `to` is.
    ///
    /// **The byte distance divided by the element's size from the IR**
    /// (ADR-0124's one layout), not by LLVM's: the memory check counts in
    /// those bytes too, so the two cannot disagree about a struct's padding.
    /// Both addresses become integers of `to`'s width, which is a pointer's
    /// on every target here (`Target::ptrdiff_type`), and the quotient is
    /// signed, since either may be the lower. Plain `sdiv`, not `exact`: a
    /// distance that is no multiple is undefined (the pointers are into no
    /// one array) and the memory check doubts it, `Unknown` by
    /// `SubtractedApart`, where it can tell (see `memory::compared_apart`),
    /// rather than this making
    /// it poison. Mutation: divide by 1;
    /// `the_difference_of_two_pointers_into_one_array_is_its_subscripts`
    /// returns the byte distance.
    fn difference(
        &mut self,
        function: &Function,
        lhs: &Operand,
        rhs: &Operand,
        ty: TyId,
        to: TyId,
        out: &mut String,
    ) -> Refuses<String> {
        let Ty::Pointer(element) = self.unit.ty(ty) else {
            unreachable!("`Self::pointer` answers only a pointer's type")
        };
        let Some(size) = self.unit.size_of(element).filter(|&size| size > 0) else {
            return self.refuse("a pointer minus a pointer to something with no size");
        };
        let Some(int) = self.unit.integer(to) else {
            let wanted = self.spell(to);
            return self.refuse(format!(
                "a difference of pointers written into a `{wanted}`"
            ));
        };
        let wide = format!("i{}", int.bits());
        let left = self.operand(function, lhs, ty, out)?;
        let right = self.operand(function, rhs, ty, out)?;
        let (from, at) = (self.temp(), self.temp());
        writeln!(out, "  {from} = ptrtoint ptr {left} to {wide}")
            .expect("writing to a string cannot fail");
        writeln!(out, "  {at} = ptrtoint ptr {right} to {wide}")
            .expect("writing to a string cannot fail");
        let bytes = self.temp();
        writeln!(out, "  {bytes} = sub {wide} {from}, {at}")
            .expect("writing to a string cannot fail");
        let result = self.temp();
        writeln!(out, "  {result} = sdiv {wide} {bytes}, {size}")
            .expect("writing to a string cannot fail");
        Ok(result)
    }

    /// Two operands under one operator.
    ///
    /// Both are taken as the destination's type, which is the type the
    /// operation happens at: `ir::Operation` states that obligation on whoever
    /// built the unit, and C17 6.3.1.1 p2 is why it exists. So a `char`
    /// operand of an `int` operation widens here rather than being computed
    /// with at its own width.
    fn binary(
        &mut self,
        function: &Function,
        op: BinOp,
        lhs: &Operand,
        rhs: &Operand,
        to: TyId,
        out: &mut String,
    ) -> Refuses<String> {
        // Whether two pointers name one object is defined and needs no width.
        // A pointer plus or minus an integer counts elements, and
        // `getelementptr` over the element's type is what scales it, so no
        // width reaches `ir::Ty` (ADR-0013). The interpreter still refuses
        // every one, which is its own change.
        let pointer = self
            .pointer(function, lhs)
            .or_else(|| self.pointer(function, rhs));
        if let Some(ty) = pointer {
            let predicate = match op {
                BinOp::Eq => "eq",
                BinOp::Ne => "ne",
                BinOp::Add | BinOp::Sub => {
                    return self.moved(function, op, lhs, rhs, to, out);
                }
                // **Unsigned, as an address is** (#1211): C17 6.5.8 p5 orders
                // pointers into one object by where they are in it, and the
                // memory check doubts one that may be between two objects
                // rather than this deciding what that means. Mutation: put the
                // refusal back; `a_comparison_of_pointers_into_one_object_runs`
                // is refused.
                BinOp::Lt => "ult",
                BinOp::Le => "ule",
                BinOp::Gt => "ugt",
                BinOp::Ge => "uge",
                _ => {
                    return self
                        .refuse("arithmetic on a pointer other than moving it by an integer");
                }
            };
            let left = self.operand(function, lhs, ty, out)?;
            let right = self.operand(function, rhs, ty, out)?;
            return self.widen(format!("icmp {predicate} ptr {left}, {right}"), to, out);
        }

        let wanted = self.spell(to);
        let Some(int) = self.unit.integer(to) else {
            return self.refuse(format!("`{}` written into a `{wanted}`", op.name()));
        };
        let signed = int.signed();

        let spelled = match op {
            BinOp::Mul => Spelled::Compute(if signed { "mul nsw" } else { "mul" }),
            BinOp::Div => Spelled::Compute(if signed { "sdiv" } else { "udiv" }),
            BinOp::Rem => Spelled::Compute(if signed { "srem" } else { "urem" }),
            // `nsw` because C17 6.5 p5 leaves a signed overflow undefined and
            // the interpreter stops rather than wrapping. A plain `add` would
            // decide what C declined to, in the direction of wrapping; this is
            // the spelling that says the program must not overflow.
            //
            // Only where the type has a sign to overflow. C17 6.2.5 p9 says a
            // computation on unsigned operands can never overflow and wraps
            // instead, so `nsw` there would make a defined program poison.
            BinOp::Add => Spelled::Compute(if signed { "add nsw" } else { "add" }),
            BinOp::Sub => Spelled::Compute(if signed { "sub nsw" } else { "sub" }),
            BinOp::Shl => Spelled::Compute("shl"),
            // An arithmetic shift keeps the sign and a logical one does not,
            // which is the difference between `-8 >> 1` answering -4 and
            // answering a very large number.
            BinOp::Shr => Spelled::Compute(if signed { "ashr" } else { "lshr" }),
            BinOp::Lt => Spelled::Compare(if signed { "slt" } else { "ult" }),
            BinOp::Gt => Spelled::Compare(if signed { "sgt" } else { "ugt" }),
            BinOp::Le => Spelled::Compare(if signed { "sle" } else { "ule" }),
            BinOp::Ge => Spelled::Compare(if signed { "sge" } else { "uge" }),
            BinOp::Eq => Spelled::Compare("eq"),
            BinOp::Ne => Spelled::Compare("ne"),
            BinOp::BitAnd => Spelled::Compute("and"),
            BinOp::BitXor => Spelled::Compute("xor"),
            BinOp::BitOr => Spelled::Compute("or"),
        };

        let left = self.operand(function, lhs, to, out)?;
        let right = self.operand(function, rhs, to, out)?;

        match spelled {
            Spelled::Compare(predicate) => self.widen(
                format!("icmp {predicate} {wanted} {left}, {right}"),
                to,
                out,
            ),
            Spelled::Compute(instruction) => {
                let result = self.temp();
                writeln!(out, "  {result} = {instruction} {wanted} {left}, {right}")
                    .expect("writing to a string cannot fail");
                Ok(result)
            }
        }
    }

    /// One element of a block.
    fn element(&mut self, function: &Function, element: &Element, out: &mut String) -> Refuses<()> {
        // Written out rather than `..`, so that a field added to a storage
        // marker is `error[E0027]` here rather than something this silently
        // stops answering for.
        match element {
            Element::Assign(operation) => {
                self.at = Some(operation.origin.span());
                self.assign(function, operation, out)
            }
            // Nothing is written, and in this subset nothing has to be. C
            // does perform the read, which an earlier comment here denied:
            // 6.3.2.1 p2 does not except a void expression from lvalue
            // conversion and `clang -O0` emits the load, measured. It is also
            // unobservable, so `clang -O2` drops it again, and every type this
            // backend can hold is in that position.
            //
            // `volatile` is where it stops. 6.7.3 p7 requires an expression
            // referring to such an object to be evaluated strictly, and
            // `clang -O2` keeps a `load atomic volatile` for `*q;`, measured.
            // The parser reads no qualifier, so nothing reaching here can be
            // one yet, and whoever adds the qualifier comes back to this arm.
            Element::Evaluate {
                place: _,
                origin: _,
            } => Ok(()),
            // Nothing is written, and nothing has to be. A sequence point is a
            // constraint on the orders an implementation may choose, and this
            // backend hands LLVM the one order the element list already spells
            // out. Emitting an ordering barrier for it would forbid
            // rearrangements C permits, which is slower code for no defined
            // program's benefit. Both markers, because both are a constraint
            // on an order and neither asks for anything to be emitted.
            Element::Sequenced { origin: _ } | Element::ArgumentsEvaluated { origin: _ } => Ok(()),
            // Nothing is written either. The conflict is reported by the memory
            // check, and a call it is about is undefined wherever both accesses
            // run, so no order emitted for it is wrong there. See ADR-0052.
            Element::UnsequencedAccess {
                local: _,
                written: _,
                accessed: _,
                both_write: _,
                between: _,
                origin: _,
            } => Ok(()),
            // Nothing is written. LLVM has `llvm.lifetime.start` and `.end` for
            // exactly this, and they buy an optimiser something this backend
            // has no optimiser to give it to, while costing two intrinsic calls
            // in every byte-exact expectation. A local whose storage ends and
            // begins again keeps its `alloca` and so keeps its value, which no
            // defined C program can observe: C17 6.2.4 p6 gives the second
            // lifetime an indeterminate value, and reading one is undefined.
            //
            // **Except for a variable length array** (#1209), which is the
            // one local these do write something for, since they make and
            // end its storage: the stack as it was is kept where it begins,
            // storage of its length's elements is made there, and the stack
            // is put back where it ends, so one in a loop, or reached again
            // by a jump back, takes no more each time round. The stack and
            // not the heap, as C has it and `clang` does it: nothing to free
            // on a path that leaves by `return` or a jump, and nothing to
            // fail on but the stack itself. Mutation: skip the restore;
            // `a_variable_length_array_is_made_and_given_back` loses its
            // `stackrestore`, and the loop in
            // `a_variable_length_array_is_sized_when_declared` overflows the
            // stack.
            Element::StorageLive { local, origin: _ } => {
                if is_variable(self.unit, function.local(*local)) {
                    self.begin_variable(function, *local, out)?;
                }
                Ok(())
            }
            Element::StorageDead { origin: _, local } => {
                if is_variable(self.unit, function.local(*local)) {
                    // `%_saved{N}`, which `body` made beside the local and
                    // `begin_variable` wrote.
                    let saved = self.temp();
                    writeln!(out, "  {saved} = load ptr, ptr %_saved{}", local.index())
                        .expect("writing to a string cannot fail");
                    writeln!(out, "  call void @llvm.stackrestore.p0(ptr {saved})")
                        .expect("writing to a string cannot fail");
                }
                Ok(())
            }
        }
    }

    /// Make the storage of the variable length array `local`, where its
    /// declaration is reached: keep the stack as it is in `%_saved{N}`, then
    /// make room for as many elements as its length local holds, and put
    /// the address in the local's own slot, `%_{N}` (#1209). Both slots are
    /// `body`'s, made at entry; the length is the lowering's, written before
    /// this marker on every path to it (see `Element::StorageLive`).
    ///
    /// **As many of the element's own type**, not bytes, so that LLVM aligns
    /// the storage as it aligns one of them, which is what `clang` gives: a
    /// `long long` on `i686` gets eight where its ABI alignment is four. A
    /// struct is bytes here, so its alignment is written out, and where the
    /// target sets a floor (`Target::variable_array_align`) that is written
    /// instead. Mutation: write `align 1`;
    /// `a_variable_length_array_is_made_and_given_back` changes.
    fn begin_variable(
        &mut self,
        function: &Function,
        local: LocalId,
        out: &mut String,
    ) -> Refuses<()> {
        let Ty::Array {
            element,
            length: Length::Local(held),
        } = self.unit.ty(function.local(local))
        else {
            unreachable!("asked of a variable length array");
        };
        if self.unit.size_of(element).is_none() {
            return self.refuse("a variable length array of elements with no size");
        }
        let align = match self.unit.target().variable_array_align() {
            Some(floor) => {
                format!(
                    ", align {}",
                    floor.max(self.unit.align_of(element).unwrap_or(1))
                )
            }
            None => self.struct_align(element),
        };
        self.stack_saved = true;
        let saved = self.temp();
        writeln!(out, "  {saved} = call ptr @llvm.stacksave.p0()")
            .expect("writing to a string cannot fail");
        writeln!(out, "  store ptr {saved}, ptr %_saved{}", local.index())
            .expect("writing to a string cannot fail");
        // The length is a `size_t`, which the lowering made it; widened to
        // the `i64` the count of an `alloca` is written in, where it is
        // narrower, as it is on a 32-bit target.
        let spelled = self.spell(function.local(held));
        let (count, _) = self.read(function, &Place::local(held), out)?;
        let count = if spelled == "i64" {
            count
        } else {
            let wide = self.temp();
            writeln!(out, "  {wide} = zext {spelled} {count} to i64")
                .expect("writing to a string cannot fail");
            wide
        };
        let storage = self.temp();
        let spelled = self.spell(element);
        writeln!(out, "  {storage} = alloca {spelled}, i64 {count}{align}")
            .expect("writing to a string cannot fail");
        writeln!(out, "  store ptr {storage}, ptr %_{}", local.index())
            .expect("writing to a string cannot fail");
        Ok(())
    }

    /// A place, and what is written into it.
    fn assign(
        &mut self,
        function: &Function,
        operation: &Operation,
        out: &mut String,
    ) -> Refuses<()> {
        let Some(to) = self.unit.place_ty(function, &operation.place) else {
            return self.refuse("a place whose projections do not fit its type");
        };
        if matches!(self.unit.ty(to), Ty::Void) {
            return self.refuse("an assignment to a place that holds nothing");
        }
        // C17 6.5.16 p2: an array is not a modifiable lvalue.
        if matches!(self.unit.ty(to), Ty::Array { .. }) {
            return self.refuse("an assignment to a whole array, which C does not have");
        }
        if matches!(self.unit.ty(to), Ty::Struct(_)) {
            return self.refuse("an assignment to a whole struct, which is not lowered yet");
        }

        let value = match &operation.value {
            Rvalue::Use(from) => self.operand(function, from, to, out)?,
            Rvalue::Unary { op, operand } => self.unary(function, *op, operand, to, out)?,
            Rvalue::Binary { op, lhs, rhs } => self.binary(function, *op, lhs, rhs, to, out)?,
            Rvalue::Address(place) => {
                if !matches!(self.unit.ty(to), Ty::Pointer(_)) {
                    let wanted = self.spell(to);
                    return self.refuse(format!("an address written into a `{wanted}`"));
                }
                self.address(function, place, out)?
            }
            // A global's symbol is its address, as LLVM spells a pointer.
            Rvalue::Static(id) => {
                if !matches!(self.unit.ty(to), Ty::Pointer(_)) {
                    let wanted = self.spell(to);
                    return self.refuse(format!("an address written into a `{wanted}`"));
                }
                self.global_name(*id)
            }
            Rvalue::Reinterpret(from) => self.reinterpret(function, from, to, out)?,
        };

        let address = self.address(function, &operation.place, out)?;
        let spelled = self.spell(to);
        writeln!(out, "  store {spelled} {value}, ptr {address}")
            .expect("writing to a string cannot fail");
        Ok(())
    }

    /// How a block ends, and where control goes from it.
    fn terminator(
        &mut self,
        function: &Function,
        terminator: &Terminator,
        out: &mut String,
    ) -> Refuses<()> {
        // A call is the only terminator whose span a refusal here spends, so
        // everything else refuses with nowhere to point rather than with the
        // last operation's place, which would put a caret on innocent code.
        // A branch has a span as well and `Refusal::at` says why it is not it.
        self.at = None;

        match terminator {
            Terminator::Goto(to) => {
                writeln!(out, "  br label %bb{}", to.index())
                    .expect("writing to a string cannot fail");
                Ok(())
            }
            Terminator::Branch {
                condition,
                then,
                otherwise,
                // A span, for a diagnostic. LLVM is handed the same branch
                // whether or not anybody points at the expression behind it.
                origin: _,
            } => {
                // The IR's condition is a value and LLVM's is a bit, so this is
                // `!= 0`, which is what C17 6.8.4.1 p2 says an `if` tests.
                let (value, spelled) = self.condition(function, condition, out)?;
                let zero = if spelled == "ptr" { "null" } else { "0" };
                let bit = self.temp();
                writeln!(out, "  {bit} = icmp ne {spelled} {value}, {zero}")
                    .expect("writing to a string cannot fail");
                writeln!(
                    out,
                    "  br i1 {bit}, label %bb{}, label %bb{}",
                    then.index(),
                    otherwise.index()
                )
                .expect("writing to a string cannot fail");
                Ok(())
            }
            Terminator::Call {
                callee,
                arguments,
                destination,
                then,
                origin,
            } => {
                self.at = Some(origin.span());
                self.call(
                    function,
                    callee,
                    arguments,
                    destination.as_ref(),
                    *then,
                    out,
                )
            }
            Terminator::Return => {
                // A struct returned in registers is read out of the bytes it
                // was written into, piece by piece (#1201).
                if let Some((slot, pieces, together)) = self.returned_in.clone() {
                    let values = self.read_pieces(&slot, &pieces, out);
                    let value = self.pack(&pieces, together, values, out);
                    let spelled = Class::aggregate(&pieces, together);
                    writeln!(out, "  ret {spelled} {value}")
                        .expect("writing to a string cannot fail");
                    return Ok(());
                }
                let place = Place::local(function.return_place());
                let ty = function.local(function.return_place());
                if matches!(self.unit.ty(ty), Ty::Void) {
                    writeln!(out, "  ret void").expect("writing to a string cannot fail");
                    return Ok(());
                }
                let (value, _) = self.read(function, &place, out)?;
                let spelled = self.spell(ty);
                writeln!(out, "  ret {spelled} {value}").expect("writing to a string cannot fail");
                Ok(())
            }
            // Nothing builds one. ADR-0010 put it in the IR so that every walk
            // has to answer for it before the thing that produces one exists,
            // and this is the answer: an edge no statement produced is a
            // landing pad and an `invoke`, which is a shape this does not have.
            Terminator::Abnormal { to: _ } => {
                self.refuse("an edge no statement produced, which has no LLVM spelling")
            }
        }
    }

    /// A branch's condition, as a value and the type LLVM spells it with.
    ///
    /// Not converted to anything: a condition is tested against zero at its own
    /// type, and a constant one has no place to take a type from. The target's
    /// `int` is what C gives an integer constant, absent a suffix this frontend
    /// does not read. No C program reaches it any more: a statement's constant
    /// controlling expression is lowered to a `Goto` (#338), and every other
    /// branch is on a temporary. IR built by hand or by another frontend can,
    /// and `a_branch_on_a_constant_tests_the_constant` holds it.
    fn condition(
        &mut self,
        function: &Function,
        operand: &Operand,
        out: &mut String,
    ) -> Refuses<(String, String)> {
        match operand {
            Operand::Constant(value) => {
                let int = self.unit.target().integer(Rank::Int, Sign::Signed);
                Ok((int.convert(*value).to_string(), format!("i{}", int.bits())))
            }
            Operand::Copy(place) => {
                let (value, ty) = self.read(function, place, out)?;
                Ok((value, self.spell(ty)))
            }
            // A function's address is never null, so it is tested as the
            // pointer it is.
            Operand::Function(id) => Ok((self.name(self.unit.function(*id)), "ptr".to_owned())),
        }
    }

    /// Enter another function, and come back, or not where the IR says the
    /// callee does not return.
    fn call(
        &mut self,
        function: &Function,
        callee: &Called,
        arguments: &[Operand],
        destination: Option<&Place>,
        then: Option<BlockId>,
        out: &mut String,
    ) -> Refuses<()> {
        let unit = self.unit;
        // **A function this unit names, or the one a pointer points at**,
        // read as one shape so both cross the call as the target's C ABI
        // says (#1212). A call through a pointer calls the pointer's value,
        // read before the arguments are passed, and its type is the
        // pointee's signature, which the lowering lays out as it lays out a
        // function's own parameters.
        let (name, callable) = match callee {
            Called::Direct(id) => {
                let called = unit.function(*id);
                // A callee whose signature cannot be written is not in the
                // module at all, so a call to it would name nothing. The
                // refusal points at the call rather than at the callee, which
                // is the span this walk holds.
                let (name, _, _) = self.signature(called)?;
                (name, Callable::of(called))
            }
            Called::Through(pointer) => {
                let Operand::Copy(place) = pointer else {
                    return self.refuse("a call through something that is not a pointer's place");
                };
                let Some(ty) = unit.place_ty(function, place) else {
                    return self.refuse("a place whose projections do not fit its type");
                };
                let signature = match unit.ty(ty) {
                    Ty::Pointer(pointee) => match unit.ty(pointee) {
                        Ty::Function(signature) => unit.signature(signature),
                        Ty::Integer { .. }
                        | Ty::Void
                        | Ty::Pointer(_)
                        | Ty::Array { .. }
                        | Ty::Struct(_) => {
                            return self.refuse("a call through a pointer to no function");
                        }
                    },
                    Ty::Integer { .. }
                    | Ty::Void
                    | Ty::Array { .. }
                    | Ty::Struct(_)
                    | Ty::Function(_) => {
                        return self.refuse("a call through something that is not a pointer");
                    }
                };
                let callable = Callable::signed(signature);
                let (value, _) = self.read(function, place, out)?;
                (value, callable)
            }
        };
        let returns = callable.returns;
        let parameters: Vec<TyId> = callable.parameters.iter().map(|&(ty, _)| ty).collect();

        if arguments.len() != parameters.len() {
            return self.refuse(format!(
                "a call passing {} arguments to a function of {} parameters",
                arguments.len(),
                parameters.len()
            ));
        }

        // C17 6.5.2.2 p7 converts each argument to its parameter's type, which
        // is what the interpreter does on the way into a frame.
        //
        // **A struct argument is passed as its target's C ABI says**, read out
        // of the copy the IR passes the address of, and a struct result
        // returned in registers is written into where the caller said
        // (#1201). See `abi::classify`.
        let crossings = self.crossings(&callable)?;
        let mut passed = Vec::with_capacity(arguments.len());
        let mut returned_into = None;
        for (index, ((argument, parameter), crossing)) in
            arguments.iter().zip(&parameters).zip(crossings).enumerate()
        {
            let value = self.operand(function, argument, *parameter, out)?;
            let Some((class, size, align)) = crossing else {
                passed.push(format!("{} {value}", self.parameter(*parameter)));
                continue;
            };
            match (callable.parameters[index].1, class) {
                (Passes::StructReturn, Class::Registers { pieces, together }) => {
                    returned_into = Some((value, pieces, together));
                }
                (Passes::StructReturn, Class::Memory { .. }) => {
                    passed.push(format!("ptr sret([{size} x i8]) align {align} {value}"));
                }
                (_, Class::Registers { pieces, together }) => {
                    let values = self.read_pieces(&value, &pieces, out);
                    if together || pieces.len() == 1 {
                        let packed = self.pack(&pieces, together, values, out);
                        passed.push(format!("{} {packed}", Class::aggregate(&pieces, together)));
                    } else {
                        for (piece, value) in pieces.iter().zip(values) {
                            passed.push(format!("{} {value}", piece.spelled()));
                        }
                    }
                }
                (_, Class::Memory { byval: false }) => passed.push(format!("ptr {value}")),
                (_, Class::Memory { byval: true }) => {
                    passed.push(format!(
                        "ptr byval([{size} x i8]) align {} {value}",
                        byval_align(align)
                    ));
                }
            }
        }
        let passed = passed.join(", ");

        if let Some((into, pieces, together)) = returned_into {
            let result = self.temp();
            let returned = Class::aggregate(&pieces, together);
            writeln!(out, "  {result} = call {returned} {name}({passed})")
                .expect("writing to a string cannot fail");
            let values = self.unpack(&pieces, together, &result, out);
            self.write_pieces(&into, &pieces, &values, out);
            match then {
                Some(then) => writeln!(out, "  br label %bb{}", then.index()),
                None => writeln!(out, "  unreachable"),
            }
            .expect("writing to a string cannot fail");
            return Ok(());
        }

        // The attribute goes on the call as well as on the callee. LLVM reads a
        // call site whose attributes disagree with its callee's as undefined
        // rather than as a mistake, so this is the third of the three places
        // one appears, beside `define` and `declare`.
        let spelled = self.result(returns);
        if spelled == "void" {
            writeln!(out, "  call void {name}({passed})").expect("writing to a string cannot fail");
        } else {
            let result = self.temp();
            writeln!(out, "  {result} = call {spelled} {name}({passed})")
                .expect("writing to a string cannot fail");
            self.deliver(function, result, returns, destination, out)?;
        }

        // C says nothing is reached after a call that does not return, and a
        // block still needs a terminator, so it says so to LLVM. See ADR-0051.
        match then {
            Some(then) => writeln!(out, "  br label %bb{}", then.index()),
            None => writeln!(out, "  unreachable"),
        }
        .expect("writing to a string cannot fail");
        Ok(())
    }

    /// Put a call's result where the call said, if it said anywhere.
    ///
    /// Nothing is stored where `Call::destination` is `None`, which is what the
    /// C frontend writes for a callee that returns nothing, or where the place
    /// holds nothing, which the IR allows. A callee that returns nothing never
    /// reaches here: its `call void` has no result to deliver.
    fn deliver(
        &mut self,
        function: &Function,
        result: String,
        returns: TyId,
        destination: Option<&Place>,
        out: &mut String,
    ) -> Refuses<()> {
        let Some(place) = destination else {
            return Ok(());
        };
        let Some(to) = self.unit.place_ty(function, place) else {
            return self.refuse("a place whose projections do not fit its type");
        };
        if matches!(self.unit.ty(to), Ty::Void) {
            return Ok(());
        }

        let value = self.convert(result, returns, to, out)?;
        let address = self.address(function, place, out)?;
        let spelled = self.spell(to);
        writeln!(out, "  store {spelled} {value}, ptr {address}")
            .expect("writing to a string cannot fail");
        Ok(())
    }

    /// What a function's type looks like to LLVM, and the name it goes by.
    ///
    /// `void` is a result type and nothing else: LLVM answers
    /// `void type only allowed for function results` to a parameter of it, and
    /// a `declare` cannot carry one either. So this is the one refusal that
    /// leaves nothing at all behind rather than a declaration, and a call to
    /// such a function is refused in turn, because there is nothing to call.
    fn signature(&mut self, function: &Function) -> Refuses<(String, String, Vec<String>)> {
        let name = self.name(function);
        let mut returns = self.result(function.local(function.return_place()));
        let crossings = self.crossings(&Callable::of(function))?;
        let mut parameters = Vec::new();

        for (index, (local, crossing)) in function.parameters().zip(crossings).enumerate() {
            let ty = function.local(local);
            if matches!(self.unit.ty(ty), Ty::Void) {
                return self.refuse(format!("{name}, which has a parameter that holds nothing"));
            }
            // **A struct crosses as its target's C ABI says** (#1201): see
            // `abi::classify`.
            let Some((class, size, align)) = crossing else {
                parameters.push(self.parameter(ty));
                continue;
            };
            match (function.passes(index), class) {
                (Passes::StructReturn, Class::Registers { pieces, together }) => {
                    returns = Class::aggregate(&pieces, together);
                }
                (Passes::StructReturn, Class::Memory { .. }) => {
                    parameters.push(format!("ptr sret([{size} x i8]) align {align}"));
                }
                (_, Class::Registers { pieces, together }) => {
                    if together || pieces.len() == 1 {
                        parameters.push(Class::aggregate(&pieces, together));
                    } else {
                        parameters.extend(pieces.iter().map(|piece| piece.spelled()));
                    }
                }
                (_, Class::Memory { byval: false }) => parameters.push("ptr".to_owned()),
                (_, Class::Memory { byval: true }) => parameters.push(format!(
                    "ptr byval([{size} x i8]) align {}",
                    byval_align(align)
                )),
            }
        }

        Ok((name, returns, parameters))
    }

    /// One function, appended to the module.
    fn function(&mut self, id: FuncId, out: &mut String) {
        let unit = self.unit;
        let function = unit.function(id);

        // The name is where a refusal about the whole function points, since
        // it is the only span a `Function` carries.
        self.at = Some(function.name);
        // **A declaration nobody can call is left out**: one taking or
        // returning a struct that is never completed is valid C (C17 6.7.6.3
        // p4) and has no signature to write, and a call to it is refused
        // where it is, since that call has no struct to pass either.
        if !function.is_defined() && takes_an_incomplete_struct(unit, function) {
            return;
        }
        let Ok((name, returns, parameters)) = self.signature(function) else {
            return;
        };

        out.push('\n');

        let declared = format!("declare {returns} {name}({})\n", parameters.join(", "));
        if !function.is_defined() {
            out.push_str(&declared);
            return;
        }

        self.next_temp = 0;

        // Built aside and committed here, because a function this could not
        // finish must leave no half of itself in the module. What it leaves is
        // a declaration, which says the definition is somewhere else. That is
        // not true, and it is what keeps the module readable to LLVM: the exit
        // code says the run failed, and a link is where the symbol surfaces.
        // That the `Err` arm comes with a recorded refusal is `Refused`'s to
        // hold.
        match self.body(function) {
            Ok(body) => {
                let named: Vec<String> = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| format!("{ty} %arg{index}"))
                    .collect();
                // `internal` is what C17 6.2.2 p3's internal linkage is to
                // LLVM: a symbol no other module's reference resolves to.
                let linkage = if function.internal_linkage() {
                    "internal "
                } else {
                    ""
                };
                writeln!(
                    out,
                    "define {linkage}{returns} {name}({}) {{",
                    named.join(", ")
                )
                .expect("writing to a string cannot fail");
                out.push_str(&body);
                out.push_str("}\n");
            }
            Err(_) => out.push_str(&declared),
        }
    }

    /// Everything between a definition's braces.
    fn body(&mut self, function: &Function) -> Refuses<String> {
        let mut body = String::new();

        // Every local gets storage, in a block that does nothing else. A
        // `void` local gets none, because `alloca void` is not a thing and
        // nothing reads one: it is where a call whose result is discarded
        // writes.
        body.push_str("entry:\n");
        for local in function.locals() {
            let ty = function.local(local);
            if matches!(self.unit.ty(ty), Ty::Void) {
                continue;
            }
            if is_variable(self.unit, ty) {
                writeln!(
                    &mut body,
                    "  %_{0} = alloca ptr\n  %_saved{0} = alloca ptr",
                    local.index()
                )
                .expect("writing to a string cannot fail");
                continue;
            }
            let spelled = self.spell(ty);
            writeln!(
                &mut body,
                "  %_{} = alloca {spelled}{}",
                local.index(),
                self.struct_align(ty)
            )
            .expect("writing to a string cannot fail");
        }

        // A parameter arrives as a value and the rest of the body wants a
        // place, which is what makes this the first thing a function does.
        //
        // **A struct passed in registers is put back into bytes of its own**,
        // and the parameter is their address, which is what the IR passes
        // (#1201); one in memory arrives as that address already. A struct
        // returned in registers is written into bytes of the function's own,
        // read back out at `return`. `arg` counts LLVM's parameters, which a
        // struct in two System V registers is two of and one returned in
        // registers none of.
        self.returned_in = None;
        let crossings = self.crossings(&Callable::of(function))?;
        let mut arg = 0;
        for (index, (local, crossing)) in function.parameters().zip(crossings).enumerate() {
            let Some((class, size, align)) = crossing else {
                let spelled = self.spell(function.local(local));
                writeln!(
                    &mut body,
                    "  store {spelled} %arg{arg}, ptr %_{}",
                    local.index()
                )
                .expect("writing to a string cannot fail");
                arg += 1;
                continue;
            };
            match (function.passes(index), class) {
                (Passes::StructReturn, Class::Registers { pieces, together }) => {
                    let slot = format!("%_returned{}", local.index());
                    writeln!(&mut body, "  {slot} = alloca [{size} x i8], align {align}")
                        .expect("writing to a string cannot fail");
                    writeln!(&mut body, "  store ptr {slot}, ptr %_{}", local.index())
                        .expect("writing to a string cannot fail");
                    self.returned_in = Some((slot, pieces, together));
                }
                (_, Class::Registers { pieces, together }) => {
                    let copy = format!("%_copy{}", local.index());
                    writeln!(&mut body, "  {copy} = alloca [{size} x i8], align {align}")
                        .expect("writing to a string cannot fail");
                    let values = if together || pieces.len() == 1 {
                        let value = format!("%arg{arg}");
                        arg += 1;
                        self.unpack(&pieces, together, &value, &mut body)
                    } else {
                        let values = (arg..arg + pieces.len())
                            .map(|n| format!("%arg{n}"))
                            .collect();
                        arg += pieces.len();
                        values
                    };
                    self.write_pieces(&copy, &pieces, &values, &mut body);
                    writeln!(&mut body, "  store ptr {copy}, ptr %_{}", local.index())
                        .expect("writing to a string cannot fail");
                }
                (_, Class::Memory { .. }) => {
                    writeln!(&mut body, "  store ptr %arg{arg}, ptr %_{}", local.index())
                        .expect("writing to a string cannot fail");
                    arg += 1;
                }
            }
        }

        // A block of its own rather than the first real one, so that the
        // preamble does not depend on which block `Function::entry` names.
        writeln!(&mut body, "  br label %bb{}", function.entry().index())
            .expect("writing to a string cannot fail");

        for (index, block) in function.blocks().enumerate() {
            writeln!(&mut body, "\nbb{index}:").expect("writing to a string cannot fail");
            for element in &block.elements {
                self.element(function, element, &mut body)?;
            }
            self.terminator(function, &block.terminator, &mut body)?;
        }

        Ok(body)
    }
}

/// Bytes as LLVM's `c"..."` spells them: a printable ASCII character as
/// itself, and `"`, `\` and every other byte as `\` and two hex digits,
/// which LLVM reads for any byte.
fn bytes_spelled(bytes: &[u8]) -> String {
    let mut spelled = String::new();
    for &byte in bytes {
        if byte.is_ascii_graphic() && byte != b'"' && byte != b'\\' || byte == b' ' {
            spelled.push(char::from(byte));
        } else {
            write!(spelled, "\\{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    spelled
}

/// Whether LLVM takes this name without quotes.
///
/// Its unquoted name is `[-a-zA-Z$._][-a-zA-Z$._0-9]*`. A leading digit is not
/// in it, because `@0` is an unnamed value rather than a name.
fn plain(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    is_name(first) && !first.is_ascii_digit() && characters.all(is_name)
}

/// Whether this character may appear in an unquoted LLVM name.
fn is_name(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '$' | '.' | '_')
}

/// A name as it goes inside `@"..."`.
///
/// LLVM reads a `\` followed by exactly two hexadecimal digits inside a quoted
/// name, and reads nothing else as an escape. So every byte that is not plainly
/// printable becomes one, and so does the backslash that would begin one.
fn escaped(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        let printable = byte.is_ascii_graphic() && byte != b'"' && byte != b'\\';
        if printable || byte == b' ' {
            out.push(char::from(byte));
        } else {
            write!(&mut out, "\\{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    out
}

#[cfg(test)]
mod tests;
