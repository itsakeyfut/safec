//! Writing a translation unit out as textual LLVM IR.
//!
//! One `alloca` per local, and a `load` or a `store` around every use of one.
//! That is one-to-one with the Safety IR, so the two artifacts can be read side
//! by side, and it needs no dominance and no `phi`, which would be Phase 4's
//! dataflow framework written a phase early. `mem2reg` is what turns these back
//! into registers, and optimisation is somebody else's job.
//!
//! What this refuses is what the interpreter refuses, and the messages say so
//! in the same words. Two consumers of one IR that disagreed about which
//! programs it can express would make the IR mean two things.

use std::fmt::Write as _;

use safec_ir::ir::{
    BinOp, BlockId, Element, FuncId, Function, Operand, Operation, Place, Projection, Rvalue,
    Terminator, TranslationUnit, Ty, TyId, UnOp,
};
use safec_ir::print::quoted;
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::{Integer, Target};

/// Something the IR can express and this backend cannot.
///
/// The shape [`Trap`] has for the interpreter, and for the same reason: this
/// crate cannot see `Diagnostic`, which lives in `safec`. Whoever calls
/// [`functions`] turns one of these into whatever it reports with.
///
/// [`Trap`]: safec_ir::interp::Trap
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// What could not be written, as a sentence a reader can act on.
    pub why: String,
    /// Where, when the IR knew.
    ///
    /// A terminator other than a call carries no span, so this is `None` more
    /// often than a diagnostic would like. What the IR knows is in `ir.rs`.
    pub at: Option<Span>,
}

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
/// anything a file said, so nothing here is content in RK-002's sense.
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
    };

    for id in unit.functions() {
        emitter.function(id, out);
    }

    emitter.refusals
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
}

impl Emitter<'_> {
    /// Record what could not be written, and answer nothing.
    ///
    /// Generic in the answer so that a caller can `return self.refuse(...)`
    /// out of whatever it was computing.
    fn refuse<T>(&mut self, why: impl Into<String>) -> Option<T> {
        self.refusals.push(Refusal {
            why: why.into(),
            at: self.at,
        });
        None
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
            Ty::Int => format!("i{}", self.unit.target().int().bits()),
            Ty::Char => format!("i{}", self.unit.target().char().bits()),
            Ty::Void => "void".to_owned(),
            Ty::Pointer(_) => "ptr".to_owned(),
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
    /// stream, and RK-002 in the review knowledge bank is the entry.
    /// `print::shown` is the wrong escape here: LLVM reads `\xx` and nothing
    /// else, so a `\u{1b}` would arrive as six characters of name.
    fn name(&self, function: &Function) -> String {
        let spelled = quoted(self.sources, function.name);
        if plain(spelled) {
            format!("@{spelled}")
        } else {
            format!("@\"{}\"", escaped(spelled))
        }
    }

    /// Where a place is, as a `ptr`.
    ///
    /// Every local is an `alloca`, so a place with no projections is already
    /// an address. A `Deref` is one `load` of a pointer. Reading a place,
    /// writing one and taking its address all want this and then differ by a
    /// single instruction.
    fn address(&mut self, function: &Function, place: &Place, out: &mut String) -> Option<String> {
        let mut address = format!("%_{}", place.local.index());
        let mut ty = function.local(place.local);

        for projection in &place.projection {
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
                // `p[i]` lowers as `*(p + i)`, so nothing builds one of these.
                // Whoever does needs a width to scale by, and `ir::Ty` holds
                // none for a pointer: the same reason the interpreter refuses
                // pointer arithmetic, in the same words.
                Projection::Index(_) => {
                    return self
                        .refuse("an indexed place, which counts elements and so needs a width");
                }
            }
        }

        Some(address)
    }

    /// The value held in a place, and the type it has.
    fn read(
        &mut self,
        function: &Function,
        place: &Place,
        out: &mut String,
    ) -> Option<(String, TyId)> {
        let Some(ty) = self.unit.place_ty(function, place) else {
            return self.refuse("a place whose projections do not fit its type");
        };
        if matches!(self.unit.ty(ty), Ty::Void) {
            return self.refuse("a read of a place that holds nothing");
        }

        let address = self.address(function, place, out)?;
        let spelled = self.spell(ty);
        let loaded = self.temp();
        writeln!(out, "  {loaded} = load {spelled}, ptr {address}")
            .expect("writing to a string cannot fail");
        Some((loaded, ty))
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
    fn convert(&mut self, value: String, from: TyId, to: TyId, out: &mut String) -> Option<String> {
        let (source, wanted) = (self.spell(from), self.spell(to));
        if source == wanted {
            return Some(value);
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
        Some(result)
    }

    /// One operand, as the type `want` names.
    fn operand(
        &mut self,
        function: &Function,
        operand: &Operand,
        want: TyId,
        out: &mut String,
    ) -> Option<String> {
        match operand {
            Operand::Constant(value) => self.constant(*value, want),
            Operand::Copy(place) => {
                let (loaded, from) = self.read(function, place, out)?;
                self.convert(loaded, from, want, out)
            }
        }
    }

    /// A constant, as the type `want` names.
    fn constant(&mut self, value: i128, want: TyId) -> Option<String> {
        if let Some(int) = self.unit.integer(want) {
            return Some(int.convert(value).to_string());
        }
        // C17 6.3.2.3 p3 makes an integer constant expression with the value 0
        // a null pointer constant, and makes nothing else one. A number that is
        // not zero reaches a pointer only through a cast, which this frontend
        // cannot write and a hand-built unit would have to mean something by.
        if matches!(self.unit.ty(want), Ty::Pointer(_)) && value == 0 {
            return Some("null".to_owned());
        }
        let wanted = self.spell(want);
        self.refuse(format!("the constant {value} where a `{wanted}` is wanted"))
    }

    /// An `i1` from a comparison, as the type the destination wants.
    ///
    /// C17 6.5.8 p6 and 6.5.9 p3 make the result of a relational or an
    /// equality operator an `int`, which is what the lowering gives the
    /// destination. So the bit is widened rather than stored as one.
    fn widen(&mut self, comparison: String, to: TyId, out: &mut String) -> Option<String> {
        let wanted = self.spell(to);
        if self.unit.integer(to).is_none() {
            return self.refuse(format!("a comparison written into a `{wanted}`"));
        }

        let bit = self.temp();
        writeln!(out, "  {bit} = {comparison}").expect("writing to a string cannot fail");
        if wanted == "i1" {
            return Some(bit);
        }

        let widened = self.temp();
        writeln!(out, "  {widened} = zext i1 {bit} to {wanted}")
            .expect("writing to a string cannot fail");
        Some(widened)
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
    ) -> Option<String> {
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
                Some(result)
            }
            // Nor a `not`: flipping every bit is an `xor` with all ones.
            UnOp::BitNot => {
                let result = self.temp();
                writeln!(out, "  {result} = xor {wanted} {value}, -1")
                    .expect("writing to a string cannot fail");
                Some(result)
            }
        }
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
    ) -> Option<String> {
        // Whether two pointers name one object is defined and needs no width.
        // Everything else about a pointer counts elements, and `ir::Ty` holds
        // no width to count them with. The interpreter draws the line in the
        // same place and says so in the same words.
        let pointer = self
            .pointer(function, lhs)
            .or_else(|| self.pointer(function, rhs));
        if let Some(ty) = pointer {
            let predicate = match op {
                BinOp::Eq => "eq",
                BinOp::Ne => "ne",
                _ => {
                    return self.refuse(
                        "arithmetic on a pointer, which counts elements and so needs a width",
                    );
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
                Some(result)
            }
        }
    }

    /// One element of a block.
    fn element(&mut self, function: &Function, element: &Element, out: &mut String) -> Option<()> {
        // Written out rather than `..`, so that a field added to a storage
        // marker is `error[E0027]` here rather than something this silently
        // stops answering for. RK-018 is the entry.
        match element {
            Element::Assign(operation) => {
                self.at = Some(operation.origin.span());
                self.assign(function, operation, out)
            }
            // Nothing is written. LLVM has `llvm.lifetime.start` and `.end` for
            // exactly this, and they buy an optimiser something this backend
            // has no optimiser to give it to, while costing two intrinsic calls
            // in every byte-exact expectation. A local whose storage ends and
            // begins again keeps its `alloca` and so keeps its value, which no
            // defined C program can observe: C17 6.2.4 p6 gives the second
            // lifetime an indeterminate value, and reading one is undefined.
            Element::StorageLive {
                local: _,
                origin: _,
            }
            | Element::StorageDead {
                origin: _,
                local: _,
            } => Some(()),
        }
    }

    /// A place, and what is written into it.
    fn assign(
        &mut self,
        function: &Function,
        operation: &Operation,
        out: &mut String,
    ) -> Option<()> {
        let Some(to) = self.unit.place_ty(function, &operation.place) else {
            return self.refuse("a place whose projections do not fit its type");
        };
        if matches!(self.unit.ty(to), Ty::Void) {
            return self.refuse("a write to a place that holds nothing");
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
        };

        let address = self.address(function, &operation.place, out)?;
        let spelled = self.spell(to);
        writeln!(out, "  store {spelled} {value}, ptr {address}")
            .expect("writing to a string cannot fail");
        Some(())
    }

    /// How a block ends, and where control goes from it.
    fn terminator(
        &mut self,
        function: &Function,
        terminator: &Terminator,
        out: &mut String,
    ) -> Option<()> {
        // Only a call carries a span, so everything else refuses with nowhere
        // to point rather than with the last operation's place.
        self.at = None;

        match terminator {
            Terminator::Goto(to) => {
                writeln!(out, "  br label %bb{}", to.index())
                    .expect("writing to a string cannot fail");
                Some(())
            }
            Terminator::Branch {
                condition,
                then,
                otherwise,
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
                Some(())
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
                    *callee,
                    arguments,
                    destination.as_ref(),
                    *then,
                    out,
                )
            }
            Terminator::Return => {
                let place = Place::local(function.return_place());
                let ty = function.local(function.return_place());
                if matches!(self.unit.ty(ty), Ty::Void) {
                    writeln!(out, "  ret void").expect("writing to a string cannot fail");
                    return Some(());
                }
                let (value, _) = self.read(function, &place, out)?;
                let spelled = self.spell(ty);
                writeln!(out, "  ret {spelled} {value}").expect("writing to a string cannot fail");
                Some(())
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
    /// does not read. `while (1)` is the shape that reaches it.
    fn condition(
        &mut self,
        function: &Function,
        operand: &Operand,
        out: &mut String,
    ) -> Option<(String, String)> {
        match operand {
            Operand::Constant(value) => {
                let int = self.unit.target().int();
                Some((int.convert(*value).to_string(), format!("i{}", int.bits())))
            }
            Operand::Copy(place) => {
                let (value, ty) = self.read(function, place, out)?;
                Some((value, self.spell(ty)))
            }
        }
    }

    /// Enter another function, and come back.
    fn call(
        &mut self,
        function: &Function,
        callee: FuncId,
        arguments: &[Operand],
        destination: Option<&Place>,
        then: BlockId,
        out: &mut String,
    ) -> Option<()> {
        let unit = self.unit;
        let called = unit.function(callee);

        // A callee whose signature cannot be written is not in the module at
        // all, so a call to it would name nothing. The refusal points at the
        // call rather than at the callee, which is the span this walk holds.
        let (name, _, _) = self.signature(called)?;
        let returns = called.local(called.return_place());
        let parameters: Vec<TyId> = called
            .parameters()
            .map(|local| called.local(local))
            .collect();

        if arguments.len() != parameters.len() {
            return self.refuse(format!(
                "a call passing {} arguments to a function of {} parameters",
                arguments.len(),
                parameters.len()
            ));
        }

        // C17 6.5.2.2 p7 converts each argument to its parameter's type, which
        // is what the interpreter does on the way into a frame.
        let mut passed = Vec::with_capacity(arguments.len());
        for (argument, parameter) in arguments.iter().zip(&parameters) {
            let value = self.operand(function, argument, *parameter, out)?;
            passed.push(format!("{} {value}", self.spell(*parameter)));
        }
        let passed = passed.join(", ");

        let spelled = self.spell(returns);
        if spelled == "void" {
            writeln!(out, "  call void {name}({passed})").expect("writing to a string cannot fail");
        } else {
            let result = self.temp();
            writeln!(out, "  {result} = call {spelled} {name}({passed})")
                .expect("writing to a string cannot fail");
            self.deliver(function, result, returns, destination, out)?;
        }

        writeln!(out, "  br label %bb{}", then.index()).expect("writing to a string cannot fail");
        Some(())
    }

    /// Put a call's result where the call said, if it said anywhere.
    ///
    /// Two shapes reach here and both mean discard it. `Call::destination` is
    /// `None` where the IR says nothing wanted the value, and the lowering
    /// instead builds a local of the callee's return type, which is `void`
    /// when the callee returns nothing.
    fn deliver(
        &mut self,
        function: &Function,
        result: String,
        returns: TyId,
        destination: Option<&Place>,
        out: &mut String,
    ) -> Option<()> {
        let Some(place) = destination else {
            return Some(());
        };
        let Some(to) = self.unit.place_ty(function, place) else {
            return self.refuse("a place whose projections do not fit its type");
        };
        if matches!(self.unit.ty(to), Ty::Void) {
            return Some(());
        }

        let value = self.convert(result, returns, to, out)?;
        let address = self.address(function, place, out)?;
        let spelled = self.spell(to);
        writeln!(out, "  store {spelled} {value}, ptr {address}")
            .expect("writing to a string cannot fail");
        Some(())
    }

    /// What a function's type looks like to LLVM, and the name it goes by.
    ///
    /// `void` is a result type and nothing else: LLVM answers
    /// `void type only allowed for function results` to a parameter of it, and
    /// a `declare` cannot carry one either. So this is the one refusal that
    /// leaves nothing at all behind rather than a declaration, and a call to
    /// such a function is refused in turn, because there is nothing to call.
    fn signature(&mut self, function: &Function) -> Option<(String, String, Vec<String>)> {
        let name = self.name(function);
        let returns = self.spell(function.local(function.return_place()));
        let mut parameters = Vec::new();

        for local in function.parameters() {
            let spelled = self.spell(function.local(local));
            if spelled == "void" {
                return self.refuse(format!("{name}, which has a parameter that holds nothing"));
            }
            parameters.push(spelled);
        }

        Some((name, returns, parameters))
    }

    /// One function, appended to the module.
    fn function(&mut self, id: FuncId, out: &mut String) {
        let unit = self.unit;
        let function = unit.function(id);

        // The name is where a refusal about the whole function points, since
        // it is the only span a `Function` carries.
        self.at = Some(function.name);
        let Some((name, returns, parameters)) = self.signature(function) else {
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
        match self.body(function) {
            Some(body) => {
                let named: Vec<String> = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| format!("{ty} %arg{index}"))
                    .collect();
                writeln!(out, "define {returns} {name}({}) {{", named.join(", "))
                    .expect("writing to a string cannot fail");
                out.push_str(&body);
                out.push_str("}\n");
            }
            None => out.push_str(&declared),
        }
    }

    /// Everything between a definition's braces.
    fn body(&mut self, function: &Function) -> Option<String> {
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
            let spelled = self.spell(ty);
            writeln!(&mut body, "  %_{} = alloca {spelled}", local.index())
                .expect("writing to a string cannot fail");
        }

        // A parameter arrives as a value and the rest of the body wants a
        // place, which is what makes this the first thing a function does.
        for (index, local) in function.parameters().enumerate() {
            let spelled = self.spell(function.local(local));
            writeln!(
                &mut body,
                "  store {spelled} %arg{index}, ptr %_{}",
                local.index()
            )
            .expect("writing to a string cannot fail");
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

        Some(body)
    }
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
mod tests {
    use super::*;

    use safec_ir::ir::{Block, Element, Operation, Origin};

    // Every unit here is built by hand, and that is the point rather than a
    // convenience: this crate has no frontend to compile C with, so a test
    // that emits is a test that the IR can be read on its own. ADR-0011 is the
    // boundary these are on the far side of. The programs that go through the
    // lexer, the parser and the lowering first are in `safec`'s corpus, which
    // is the only side that can compile one.

    /// A target by name, because `Target` has no other way to be one.
    fn target(triple: &str) -> Target {
        Target::from_triple(triple).expect("a known triple")
    }

    /// A source map holding one name, and the span that covers it.
    fn named(name: &str) -> (SourceMap, Span) {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", name);
        let end = u32::try_from(name.len()).expect("a short name");
        (sources, Span::new(file, 0, end))
    }

    /// The whole module, and whatever could not be written.
    fn module(sources: &SourceMap, unit: &TranslationUnit) -> (String, Vec<Refusal>) {
        let mut out = header(unit.target());
        let refusals = functions(sources, unit, &mut out);
        (out, refusals)
    }

    /// The whole module, byte for byte, from a unit nothing compiled.
    ///
    /// One assertion over the whole text rather than a `contains` per line,
    /// because the artifact is an interface: a reader redirects it into
    /// `clang` and diffs two runs of it, so what it looks like is the claim.
    ///
    /// Mutation: drop the `store` that puts a parameter into its local. The
    /// text stops matching, and `clang` stops accepting the module, which is
    /// two tests for two claims.
    #[test]
    fn a_unit_built_by_hand_becomes_a_module() {
        let (sources, at) = named("twice");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        let mut twice = Function::new(at, int, [int]);
        let n = twice.parameters().next().expect("one parameter");
        twice.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(twice.return_place()),
                value: Rvalue::Binary {
                    op: BinOp::Add,
                    lhs: Operand::Copy(Place::local(n)),
                    rhs: Operand::Copy(Place::local(n)),
                },
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        unit.push_function(twice);

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(refusals, Vec::new());
        assert_eq!(
            out,
            concat!(
                "target triple = \"x86_64-pc-windows-msvc\"\n",
                "\n",
                "define i32 @twice(i32 %arg0) {\n",
                "entry:\n",
                "  %_0 = alloca i32\n",
                "  %_1 = alloca i32\n",
                "  store i32 %arg0, ptr %_1\n",
                "  br label %bb0\n",
                "\n",
                "bb0:\n",
                "  %t0 = load i32, ptr %_1\n",
                "  %t1 = load i32, ptr %_1\n",
                "  %t2 = add nsw i32 %t0, %t1\n",
                "  store i32 %t2, ptr %_0\n",
                "  %t3 = load i32, ptr %_0\n",
                "  ret i32 %t3\n",
                "}\n",
            )
        );
    }

    /// Each type is spelled the way the target says, and a `void` local gets no
    /// storage: `alloca void` is not a thing, and nothing reads one.
    ///
    /// Mutation: spell `Ty::Char` as the target's `int`. The `i8` lines become
    /// `i32` and this fails. Mutation: give every local an `alloca`. The `void`
    /// return place gets one and this fails.
    #[test]
    fn each_type_is_spelled_as_the_target_says() {
        let (sources, at) = named("f");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        let char = unit.push_type(Ty::Char);
        let pointer = unit.push_type(Ty::Pointer(int));
        let void = unit.push_type(Ty::Void);
        let mut f = Function::new(at, void, [int, char, pointer]);
        f.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        unit.push_function(f);

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(refusals, Vec::new());
        assert!(
            out.contains("define void @f(i32 %arg0, i8 %arg1, ptr %arg2) {\n"),
            "{out}"
        );
        assert!(out.contains("  %_1 = alloca i32\n"), "{out}");
        assert!(out.contains("  %_2 = alloca i8\n"), "{out}");
        assert!(out.contains("  %_3 = alloca ptr\n"), "{out}");
        assert!(!out.contains("alloca void"), "{out}");
        assert!(out.contains("  ret void\n"), "{out}");
    }

    /// Widening a narrower type carries the sign only where that type has one,
    /// which is the target's to say. `char` is the only type here that differs
    /// between the measured machines, and it is reachable from ordinary C:
    /// `char c; int i; i = c;`.
    ///
    /// Mutation: always `sext`. The unsigned half of this fails. Mutation:
    /// always `zext`. The signed half does.
    #[test]
    fn a_narrower_type_widens_the_way_its_own_target_says() {
        for (triple, instruction) in [
            ("x86_64-pc-windows-msvc", "sext"),
            ("aarch64-unknown-linux-gnu", "zext"),
        ] {
            let (sources, at) = named("f");
            let mut unit = TranslationUnit::new(target(triple));
            let int = unit.push_type(Ty::Int);
            let char = unit.push_type(Ty::Char);
            let mut f = Function::new(at, int, [char]);
            let c = f.parameters().next().expect("one parameter");
            f.push_block(Block {
                elements: vec![Element::Assign(Operation {
                    place: Place::local(f.return_place()),
                    value: Rvalue::Use(Operand::Copy(Place::local(c))),
                    origin: Origin::Written(at),
                })],
                terminator: Terminator::Return,
            });
            unit.push_function(f);

            let (out, refusals) = module(&sources, &unit);

            assert_eq!(refusals, Vec::new());
            assert!(
                out.contains(&format!("= {instruction} i8 ")),
                "{triple}: {out}"
            );
        }
    }

    /// A division, a shift right and an addition mean different things on a
    /// signed type and on an unsigned one, and which a `char` is follows the
    /// target. C17 6.2.5 p9 is why the addition is in this list: an unsigned
    /// computation cannot overflow, so `nsw` there would make a defined
    /// program poison.
    ///
    /// Nothing the frontend builds reaches this: C17 6.3.1.1 p2 promotes a
    /// `char` operand, so an operation never happens at `char` in a unit this
    /// compiler lowered. A hand-built one can, which is the case a Clang
    /// adapter is, and is why this crate is the side of ADR-0011's arrow that
    /// can be tested without a frontend.
    ///
    /// Mutation: use `sdiv`, `ashr` and `add nsw` whatever the type. The
    /// unsigned half fails. Mutation: use `udiv`, `lshr` and a plain `add`
    /// whatever the type. The signed half does.
    #[test]
    fn an_operation_on_an_unsigned_type_is_unsigned() {
        for (triple, division, shift, addition) in [
            ("x86_64-pc-windows-msvc", "sdiv", "ashr", "add nsw"),
            ("aarch64-unknown-linux-gnu", "udiv", "lshr", "add"),
        ] {
            let (sources, at) = named("f");
            let mut unit = TranslationUnit::new(target(triple));
            let char = unit.push_type(Ty::Char);
            let mut f = Function::new(at, char, [char, char]);
            let mut parameters = f.parameters();
            let a = parameters.next().expect("two parameters");
            let b = parameters.next().expect("two parameters");
            let operation = |op| {
                Element::Assign(Operation {
                    place: Place::local(f.return_place()),
                    value: Rvalue::Binary {
                        op,
                        lhs: Operand::Copy(Place::local(a)),
                        rhs: Operand::Copy(Place::local(b)),
                    },
                    origin: Origin::Written(at),
                })
            };
            f.push_block(Block {
                elements: vec![
                    operation(BinOp::Div),
                    operation(BinOp::Shr),
                    operation(BinOp::Add),
                ],
                terminator: Terminator::Return,
            });
            unit.push_function(f);

            let (out, refusals) = module(&sources, &unit);

            assert_eq!(refusals, Vec::new());
            assert!(
                out.contains(&format!("= {division} i8 ")),
                "{triple}: {out}"
            );
            assert!(out.contains(&format!("= {shift} i8 ")), "{triple}: {out}");
            assert!(
                out.contains(&format!("= {addition} i8 ")),
                "{triple}: {out}"
            );
        }
    }

    /// A name LLVM would not take unquoted is quoted, and every byte that is
    /// not plainly printable inside the quotes becomes `\xx`.
    ///
    /// No C identifier needs this, which is the point: the IR is meant to be
    /// reachable from a Clang adapter, and a C++ name is not a C identifier.
    ///
    /// Mutation: write every name bare. LLVM stops parsing the module and this
    /// fails on the text. Mutation: escape with `print::shown`. The escape
    /// arrives as `\u{1b}`, which LLVM reads as `\u` and four more characters.
    #[test]
    fn a_name_llvm_would_not_take_is_quoted() {
        let (sources, at) = named("a\u{1b}b");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        unit.push_function(Function::declaration(at, int, []));

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(refusals, Vec::new());
        assert!(out.contains("declare i32 @\"a\\1Bb\"()\n"), "{out}");
    }

    /// A declaration is a `declare` and has no body, because the definition is
    /// not in this unit and saying otherwise would claim a call does something
    /// this compiler has not seen.
    ///
    /// Mutation: emit a `define` with an empty body for a declaration. The
    /// text stops matching and LLVM refuses a function with no blocks.
    #[test]
    fn a_declaration_has_no_body() {
        let (sources, at) = named("other");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        unit.push_function(Function::declaration(at, int, [int]));

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(refusals, Vec::new());
        assert!(out.ends_with("declare i32 @other(i32)\n"), "{out}");
        assert!(!out.contains("define"), "{out}");
    }

    /// An edge no statement produced is refused, and the function it was in
    /// becomes a declaration rather than half a definition.
    ///
    /// ADR-0010 put the variant in the IR so that every walk has to answer for
    /// it before anything builds one. This is this backend's answer.
    ///
    /// Mutation: write a `br` to the abnormal edge's target. Nothing is
    /// refused and this fails on the refusal. Mutation: keep the body that was
    /// written before the refusal. The module holds a definition and this
    /// fails on `declare`.
    #[test]
    fn an_edge_no_statement_produced_is_refused() {
        let (sources, at) = named("f");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        let mut f = Function::new(at, int, []);
        let block = f.reserve_block();
        f.fill_block(
            block,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Abnormal { to: block },
            },
        );
        unit.push_function(f);

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(
            refusals,
            vec![Refusal {
                why: "an edge no statement produced, which has no LLVM spelling".to_owned(),
                at: None,
            }]
        );
        assert!(out.ends_with("declare i32 @f()\n"), "{out}");
    }

    /// Arithmetic on a pointer is refused, in the interpreter's own words,
    /// and the refusal says where.
    ///
    /// `p + 1` counts elements and `ir::Ty` holds no width to count them with,
    /// which ADR-0013 decided. Two consumers of one IR that disagreed about
    /// which programs it can express would make the IR mean two things.
    ///
    /// Mutation: emit a `getelementptr`. Nothing is refused and this fails.
    #[test]
    fn arithmetic_on_a_pointer_is_refused() {
        let (sources, at) = named("f");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        let pointer = unit.push_type(Ty::Pointer(int));
        let mut f = Function::new(at, pointer, [pointer]);
        let p = f.parameters().next().expect("one parameter");
        f.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(f.return_place()),
                value: Rvalue::Binary {
                    op: BinOp::Add,
                    lhs: Operand::Copy(Place::local(p)),
                    rhs: Operand::Constant(1),
                },
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        unit.push_function(f);

        let (_, refusals) = module(&sources, &unit);

        assert_eq!(
            refusals,
            vec![Refusal {
                why: "arithmetic on a pointer, which counts elements and so needs a width"
                    .to_owned(),
                at: Some(at),
            }]
        );
    }

    /// Whether two pointers name one object is defined and needs no width, so
    /// it is written rather than refused. The interpreter answers the same
    /// question the same way.
    ///
    /// Mutation: refuse every operator on a pointer. This fails on the
    /// refusal. Mutation: store the `i1` rather than widening it. The module
    /// stops matching, and LLVM refuses a `store i1` into an `i32`.
    #[test]
    fn two_pointers_can_be_compared() {
        let (sources, at) = named("f");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        let pointer = unit.push_type(Ty::Pointer(int));
        let mut f = Function::new(at, int, [pointer, pointer]);
        let mut parameters = f.parameters();
        let p = parameters.next().expect("two parameters");
        let q = parameters.next().expect("two parameters");
        f.push_block(Block {
            elements: vec![Element::Assign(Operation {
                place: Place::local(f.return_place()),
                value: Rvalue::Binary {
                    op: BinOp::Eq,
                    lhs: Operand::Copy(Place::local(p)),
                    rhs: Operand::Copy(Place::local(q)),
                },
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Return,
        });
        unit.push_function(f);

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(refusals, Vec::new());
        assert!(out.contains("= icmp eq ptr %t0, %t1\n"), "{out}");
        assert!(out.contains("= zext i1 %t2 to i32\n"), "{out}");
    }

    /// A call whose result nothing wanted writes nowhere, and a call to a
    /// function that returns nothing has no result to write.
    ///
    /// Both shapes reach the same place: the IR allows `destination: None`,
    /// and the lowering instead builds a local of the callee's return type,
    /// which is `void` when the callee returns nothing.
    ///
    /// Mutation: give the `call void` a result name. LLVM refuses a value of
    /// type `void`, and this fails on the text first.
    #[test]
    fn a_call_that_returns_nothing_stores_nothing() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "v f");
        let (v, at) = (Span::new(file, 0, 1), Span::new(file, 2, 3));

        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        let void = unit.push_type(Ty::Void);
        let callee = unit.push_function(Function::declaration(v, void, []));

        let mut f = Function::new(at, int, []);
        // Reserved before the block it goes to, because a block's id is where
        // it sits and `Function::entry` is the first id handed out. Pushing the
        // return block first would make *it* the entry and the call would never
        // be reached.
        let first = f.reserve_block();
        let after = f.push_block(Block {
            elements: Vec::new(),
            terminator: Terminator::Return,
        });
        f.fill_block(
            first,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Call {
                    callee,
                    arguments: Vec::new(),
                    destination: None,
                    then: after,
                    origin: Origin::Written(at),
                },
            },
        );
        assert_eq!(first, f.entry());
        unit.push_function(f);

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(refusals, Vec::new());
        assert!(out.contains("  call void @v()\n"), "{out}");
        assert!(!out.contains("= call"), "{out}");
    }

    /// A parameter that holds nothing cannot be written at all, so the function
    /// it belongs to leaves neither a definition nor a declaration.
    ///
    /// `void` is a result type and nothing else: LLVM answers `void type only
    /// allowed for function results` to `declare i32 @g(void)` just as it does
    /// to a definition. This is the one refusal that leaves nothing behind,
    /// and it is why a caller of such a function is refused in turn.
    ///
    /// Mutation: spell the parameter and carry on. The module holds
    /// `declare i32 @g(void)`, `clang` refuses to parse it, and this fails on
    /// both the refusal and the text. Mutation: leave a `declare` behind after
    /// refusing. This fails on the text.
    #[test]
    fn a_parameter_that_holds_nothing_leaves_no_function() {
        let (sources, at) = named("g");
        let mut unit = TranslationUnit::new(target("x86_64-pc-windows-msvc"));
        let int = unit.push_type(Ty::Int);
        let void = unit.push_type(Ty::Void);
        unit.push_function(Function::declaration(at, int, [void]));

        let (out, refusals) = module(&sources, &unit);

        assert_eq!(
            refusals,
            vec![Refusal {
                why: "@g, which has a parameter that holds nothing".to_owned(),
                at: Some(at),
            }]
        );
        assert!(!out.contains("void"), "{out}");
        assert!(!out.contains("@g"), "{out}");
    }

    /// The header names the target and is written once, which is what makes an
    /// artifact with several inputs in it one module rather than a parse error
    /// at the second `target triple`.
    ///
    /// Mutation: have `functions` write the header too. An artifact built from
    /// two inputs holds two of them, and LLVM stops parsing at the second.
    #[test]
    fn the_header_names_the_target_once() {
        let (sources, at) = named("f");
        let mut unit = TranslationUnit::new(target("wasm32-unknown-unknown"));
        let int = unit.push_type(Ty::Int);
        unit.push_function(Function::declaration(at, int, []));

        let mut out = header(unit.target());
        functions(&sources, &unit, &mut out);
        functions(&sources, &unit, &mut out);

        assert_eq!(out.matches("target triple").count(), 1, "{out}");
        assert!(
            out.starts_with("target triple = \"wasm32-unknown-unknown\"\n"),
            "{out}"
        );
    }
}
