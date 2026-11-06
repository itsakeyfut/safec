//! The transfer functions of the analysis, [`Allocations`], whose fields are
//! in `allocations.rs`: what each element, terminator and edge of a function
//! does to the [`Known`] value, and what the analysis asks of the IR to answer
//! that.

use std::collections::{BTreeMap, BTreeSet};

use crate::dataflow::Analysis;
use crate::ir::{
    BinOp, BlockId, DOES_NOT_RETURN, Element, FuncId, Function, LocalId, Operand, Place,
    Projection, Rvalue, Terminator, TranslationUnit, Ty, TyId,
};
use crate::source::Span;

use super::LostReason;
use super::allocations::{Allocations, handed_places, innermost_element};
use super::built::{built_from, named, replaced_by};
use super::fit::{Fit, fits, slots_from, slots_leaving};
use super::index::{Idx, Loc, LocalIdx, Site, Table};
use super::init::Wrote;
use super::known::{ArrayWrite, ELEMENTS_KEPT, Known};
use super::parts::{
    AS_TYPED, Cell, Extent, Freeing, Held, Init, Inits, Levels, Marks, Offset, PendingRead,
    Reached, Read, Realloced, SiteState, bounded,
};
use super::store::{Landing, StoreKind, Strong};
use crate::library::{Callee, Leaves, Library, Makes};
use crate::reads::{dereferenced_in_element, dereferenced_in_terminator, derefs};

/// What [`Allocations::land_contents`] is told of one write beyond where it
/// lands.
enum Pours {
    /// An assignment: whether what it stores may be a pointer, the
    /// allocations what it stores holds, and where it is.
    Store {
        pointer_stored: bool,
        carried: Vec<Site>,
        span: Span,
    },
    /// A library copy.
    Copy,
}

/// How a library copy's length covers the pointers it writes, which
/// [`Allocations::copied_pointers`] answers once for the two doors that ask:
/// what the copy leaves (`report::departed_at_call`) and what it writes of
/// what is unwritten ([`Allocations::initialised_by`]). One answer, so that
/// neither door reads the length differently from the other (#717, #745).
pub(super) enum CopiedPointers {
    /// A whole number of pointers, or a length this check cannot read,
    /// which `report::reached_by_library` already reports at the copy
    /// wherever the destination holds anything.
    Whole,
    /// At least one whole pointer, and part of the next.
    LastInPart,
    /// Less than one pointer.
    NoneWhole,
}

impl Allocations<'_> {
    /// How many bytes the object a call's result names takes, as far as this
    /// check can read it.
    ///
    /// **Exactly the size an allocation function was asked for, where it was
    /// asked with constants**: `malloc(n)` and `aligned_alloc(a, n)` make `n`
    /// bytes (C17 7.22.3.4 p2, 7.22.3.1 p2), `calloc(m, n)` makes `m * n`
    /// (7.22.3.2 p2), and `realloc(p, n)` makes `n` (7.22.3.5 p2). A size
    /// this check cannot read, a negative constant among them, which converts
    /// to a `size_t` nothing could allocate, is nothing read. Any other call's
    /// result is believed to point at one whole object of the type it
    /// returns, as a parameter is, and a `void *` at nothing. See ADR-0060.
    fn made_extent(
        &self,
        function: &Function,
        callee: FuncId,
        kind: Callee,
        handed: &[Operand],
        destination: &Place,
    ) -> Extent {
        let constant = |index: usize| self.constant_argument(callee, handed, index);
        match kind {
            Callee::Opaque => {
                return Extent::AtLeast(self.pointee_size(function, destination).unwrap_or(0));
            }
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Ends => {}
        }
        let asked = match self.library(callee).and_then(|row| row.makes) {
            Some(Makes::Argument(index)) => constant(index),
            Some(Makes::Product(m, n)) => constant(m)
                .zip(constant(n))
                .and_then(|(m, n)| m.checked_mul(n)),
            None => None,
        };
        asked.map_or(Extent::AtLeast(0), Extent::Exactly)
    }

    /// The constant a call hands this parameter, as the parameter receives
    /// it, or `None` where it is not a constant this check can read.
    ///
    /// **Converted to the parameter's type first**, as C17 6.5.2.2 p7 does
    /// "as if by assignment", because the IR carries the constant as the
    /// source spelled it and the backend converts it at the call: `void
    /// *malloc(char n);` handed 257 allocates one byte on every target here,
    /// and reading 257 proved a four-byte write inside it. A parameter that
    /// is not an integer, or one the declaration does not name, reads
    /// nothing; so does a value negative after the conversion, which no
    /// size this check compares is. Found by review. See ADR-0060.
    pub(super) fn constant_argument(
        &self,
        callee: FuncId,
        arguments: &[Operand],
        index: usize,
    ) -> Option<u64> {
        let Some(Operand::Constant(value)) = arguments.get(index) else {
            return None;
        };
        let declared = self.unit.function(callee);
        let parameter = declared.parameters().nth(index)?;
        let integer = self.unit.integer(declared.local(parameter))?;
        u64::try_from(integer.convert(*value)).ok()
    }

    /// How the length a library copy is handed, its third argument, covers
    /// the pointers it writes, counted in the target's pointer width: a
    /// copy of four bytes where pointers take eight writes half of one, and
    /// a reader of it reads a pointer built from four new bytes and four
    /// old ones (#717), or from four and four nothing wrote (#745).
    pub(super) fn copied_pointers(&self, callee: FuncId, arguments: &[Operand]) -> CopiedPointers {
        let target = self.unit.target();
        let pointer = u64::from(target.pointer_bits() / target.char().bits());
        match self.constant_argument(callee, arguments, 2) {
            Some(length) if length < pointer => CopiedPointers::NoneWhole,
            Some(length) if length % pointer != 0 => CopiedPointers::LastInPart,
            Some(_) | None => CopiedPointers::Whole,
        }
    }

    /// What this check can read in the name of the function being called.
    pub(super) fn callee(&self, id: FuncId) -> Callee {
        let name = self.sources.snippet(self.unit.function(id).name);
        if DOES_NOT_RETURN
            .iter()
            .any(|&(reserved, _)| reserved == name)
        {
            return Callee::Ends;
        }
        self.library(id).map_or(Callee::Opaque, |row| row.callee)
    }

    /// The row of `LIBRARY` this function's name is, if any, and none for a
    /// body this unit defines: that body is what runs, whatever it is called,
    /// so [`Self::callee`] answers [`Callee::Opaque`] for it and the transfer
    /// and every door read one kind. Defining a library name is undefined
    /// (C17 7.1.3 p2); a `memcpy` defined here once took the copy's transfer
    /// and an ordinary call's doors, and a program leaving its caller a freed
    /// pointer built silent (#800). See ADR-0071. **And none for a
    /// declaration of another shape than the row's**, read as any call by
    /// this check and the nullability check alike, since both ask
    /// [`crate::library::read_as_library`] (ADR-0073 rule 1).
    pub(super) fn library(&self, id: FuncId) -> Option<&'static Library> {
        crate::library::read_as_library(self.unit, self.sources, id)
    }

    /// What a write through a pointer carries: the sites the written value may
    /// hold.
    ///
    /// One answer for every reader of it: the write that lands in a followed
    /// local, and what a write into memory records as inside an allocation or exposes.
    /// One rule in two places drifts.
    ///
    /// **A `match` rather than an `if let`, so a fifth kind of rvalue has to
    /// answer here too.** Every other reader of `Rvalue` in this crate is
    /// exhaustive and `error[E0004]` is what asks them; this one was the
    /// exception, and what a missed arm would mean is that a write through a
    /// pointer silently carries nothing, which is a silence rather than a build
    /// error.
    fn carried(&self, function: &Function, written_value: &Rvalue, value: &Known) -> Held {
        match written_value {
            // `*pp = q + 1;` arrives here as a copy, not as the arithmetic:
            // the lowering puts the addition in a temporary and copies it
            // out, and the direct assignment in `Allocations::element` has
            // already given that temporary `q`'s sites.
            Rvalue::Use(Operand::Copy(source)) if source.projection.is_empty() => {
                value.points_to[source.local].clone()
            }
            // The arithmetic written straight into the place, which no C
            // reaches for the reason above and another frontend may. The
            // operands the types say contributed, for the reason the direct
            // assignment's `Rvalue::Binary` arm gives, and the same question
            // about the edge: this asked none of it until review built the
            // shape by hand, and carried an edge through `qq + 7` that the
            // direct assignment had just been taught to drop.
            Rvalue::Binary { op, lhs, rhs } => {
                let mut reached = built_from(
                    *op,
                    [lhs, rhs],
                    value,
                    |local| self.is_pointer(function, local),
                    |place| self.may_be_pointer(function, place),
                    |place| self.reads_a_byte(function, place),
                    |local, depth| self.reads_caller_memory(local, depth, value),
                    |local| self.is_array(function, local),
                );
                // What the arithmetic leaves of the edge, as the direct
                // assignment's `Rvalue::Binary` arm asks. See ADR-0019. No C
                // program reaches this arm, measured: the lowering puts the
                // sum in a temporary first.
                reached.moved_by_arithmetic(self.moved_by(function, *op, lhs, rhs));
                reached
            }
            // A read through a projection is not a pointer this check follows
            // to an allocation, and the target is given no site for it; but
            // it may be a pointer read out of memory, and says so. See
            // `Allocations::read_through`.
            Rvalue::Use(Operand::Copy(source)) => self.read_through(function, source, value),
            // A constant, a unary operator, an address. None is a pointer
            // this check follows to an allocation, so the target is given
            // nothing: a write this check cannot follow is not evidence that
            // the old contents are gone.
            Rvalue::Use(Operand::Constant(_)) | Rvalue::Unary { .. } | Rvalue::Address(_) => {
                Held::none(value.points_to.len())
            }
        }
    }

    /// The per-element record each local starts with: a bit set for every
    /// element of an array of at most `ELEMENTS_KEPT` pointers that is
    /// unwritten at entry, and none for any other local. See
    /// `Known::element_init`.
    fn elements_at_entry(&self) -> Vec<Inits> {
        let function = self.unit.function(self.function);
        let unwritten = self.unwritten_at_entry();
        function
            .locals()
            .map(|local| {
                let length = match self.unit.ty(function.local(local)) {
                    Ty::Array { .. } => self.innermost_elements(function.local(local)),
                    _ => None,
                };
                match length {
                    Some(length) if length <= ELEMENTS_KEPT && unwritten[local.index()] => {
                        Inits::unwritten(usize::try_from(length).unwrap_or(0))
                    }
                    _ => Inits::written(0),
                }
            })
            .collect()
    }

    /// How far `lhs op rhs` moves a pointer, counted in the innermost
    /// elements of the array it points into: a constant times how many of
    /// those one pointee holds. `None` where the move is not a constant, or
    /// is not an addition or a subtraction of one from a pointer. See
    /// `Held::element`.
    fn moved_by(
        &self,
        function: &Function,
        op: BinOp,
        lhs: &Operand,
        rhs: &Operand,
    ) -> Option<i128> {
        let (pointer, constant, sign) = match (op, lhs, rhs) {
            (BinOp::Add, Operand::Copy(place), Operand::Constant(c))
            | (BinOp::Add, Operand::Constant(c), Operand::Copy(place)) => (place, *c, 1),
            (BinOp::Sub, Operand::Copy(place), Operand::Constant(c)) => (place, *c, -1),
            _ => return None,
        };
        if !pointer.projection.is_empty() {
            return None;
        }
        let pointee = match self.unit.ty(function.local(pointer.local)) {
            Ty::Pointer(pointee) => pointee,
            _ => return None,
        };
        let stride = self.innermost_elements(pointee)?;
        constant.checked_mul(i128::from(stride))?.checked_mul(sign)
    }

    /// How many innermost elements one object of `ty` holds: an array's
    /// length times its element's, and one pointer. `None` where the
    /// innermost element is not a pointer: an element is counted for the
    /// read [`Self::reads_fresh`] follows, which is of a pointer, and `c - 8`
    /// with `c` a `char *` into an array of pointers moves one of them, not
    /// eight. Found by review of #667.
    fn innermost_elements(&self, ty: TyId) -> Option<u64> {
        match self.unit.ty(ty) {
            Ty::Array { element, length } => length.checked_mul(self.innermost_elements(element)?),
            Ty::Pointer(_) => Some(1),
            Ty::Int | Ty::Char | Ty::Void => None,
        }
    }

    /// Whether the local at `index` is an array, which pointer arithmetic
    /// keeps the edge to (#639). One answer for both places that build a
    /// value out of operands. See `Held::moved_by_arithmetic`.
    pub(super) fn is_array(&self, function: &Function, index: LocalIdx) -> bool {
        super::built::is_array(self.unit, function, index)
    }

    /// Whether this local holds a pointer.
    ///
    /// What tells the pointer operand of an addition from the integer beside
    /// it, which is the question [`built_from`] asks and C17 6.5.6 p8 answers.
    /// See ADR-0030.
    ///
    /// **Written out rather than as a `matches!`, because the answer for a kind
    /// nobody has added yet is not `false`.** A type this does not recognise is
    /// dropped from an addition that has a pointer beside it, and a dropped
    /// operand is a site nothing reports: `error[E0004]` here is what asks a
    /// fourth kind of type whether it is one. `E0004` makes somebody look and
    /// that is all it makes them do, and this is the case where a reader has
    /// something to decide rather than a line to fill in.
    pub(super) fn is_pointer(&self, function: &Function, local: LocalId) -> bool {
        match self.unit.ty(function.local(local)) {
            Ty::Pointer(_) => true,
            // No whole array is an operand of arithmetic: it is converted to
            // a pointer to its first element first (C17 6.3.2.1 p3).
            Ty::Int | Ty::Char | Ty::Void | Ty::Array { .. } => false,
        }
    }

    /// Whether a place may hold a pointer, by its type.
    ///
    /// **`None` answers yes.** [`TranslationUnit::place_ty`] has no type for a
    /// `Deref` of something that is not a pointer, which the lowering does not
    /// build and a hand-built unit can, and a place whose type this cannot
    /// name is one it cannot narrow: [`replaced_by`] reads it the same way.
    ///
    /// **A `char` answers no**, though C17 6.5 p7 lets one copy a pointer a
    /// byte at a time and [`replaced_by`] reads it as reaching everything for
    /// that reason. The copy is followed where the byte is read, by
    /// [`Self::read_through`]; answering yes here would also read every
    /// character handed to a call as everything stored anywhere, which refused
    /// correct string code. See ADR-0046.
    pub(super) fn may_be_pointer(&self, function: &Function, place: &Place) -> bool {
        match self
            .unit
            .place_ty(function, place)
            .map(|ty| self.unit.ty(ty))
        {
            Some(Ty::Pointer(_)) | None => true,
            Some(Ty::Int | Ty::Char | Ty::Void) => false,
            // As its elements do: no whole array is read or written, and where
            // one were, what it holds is its elements. **Held by nothing,
            // measured**: answering `false` changes no case, since no C reads a
            // place of array type.
            Some(Ty::Array { element, .. }) => {
                match self.unit.ty(innermost_element(self.unit, element)) {
                    Ty::Pointer(_) => true,
                    Ty::Int | Ty::Char | Ty::Void => false,
                    Ty::Array { .. } => unreachable!("`innermost_element` strips every array"),
                }
            }
        }
    }

    /// How many bytes one object of the type a place's pointer points to
    /// takes, where the place holds a pointer: `Some(0)` for a `void *`, which
    /// says nothing about what it points at, and `None` for a place that is
    /// not a pointer. See ADR-0060.
    pub(super) fn pointee_size(&self, function: &Function, place: &Place) -> Option<u64> {
        let ty = self.unit.place_ty(function, place)?;
        match self.unit.ty(ty) {
            Ty::Pointer(pointee) => Some(self.unit.size_of(pointee).unwrap_or(0)),
            // An array is not a pointer, so a place of one points at nothing.
            Ty::Int | Ty::Char | Ty::Void | Ty::Array { .. } => None,
        }
    }

    /// How many bytes the pointers each level below what a value of type
    /// `ty` points at are believed to point at, at most: at each level, what
    /// a pointer of the type stored there believes, and `0` from the first
    /// level that is no pointer down, since nothing stored there was vouched
    /// for as one. `void ***` believes `8` and then `0`, `int ***` `8` and
    /// then `4`. `None` for a `ty` that is no pointer. See
    /// [`Held::contents_at_most`] and ADR-0060.
    pub(super) fn type_levels(&self, ty: TyId) -> Option<Levels> {
        let Ty::Pointer(mut stored) = self.unit.ty(ty) else {
            return None;
        };
        let mut levels = Vec::with_capacity(self.deepest_pointer);
        // Once `stored` is no pointer it stays what it is, so every level
        // below answers `0` as well.
        //
        // **Through an array to its element**, since memory holding an array
        // holds its elements one after another: `int *(*)[2]` believes
        // `int`s at level 0. Stopping at an array instead believed nothing of
        // an array of pointers reached through a pointer (#621).
        for _ in 0..self.deepest_pointer {
            stored = innermost_element(self.unit, stored);
            let width = match self.unit.ty(stored) {
                Ty::Pointer(inner) => {
                    stored = inner;
                    self.unit.size_of(inner).unwrap_or(0)
                }
                Ty::Int | Ty::Char | Ty::Void => 0,
                Ty::Array { .. } => unreachable!("`innermost_element` strips every array"),
            };
            levels.push(Some(width));
        }
        Some(Levels::from_bounds(levels))
    }

    /// What a pointer read as `place` believes the memory it points at
    /// holds, by level: the type it is read as, no more than what the
    /// memory it is read out of was believed one level further down
    /// ([`Known::memory_levels`]). A local's own value believes what it
    /// carries. See [`Held::contents_at_most`] and ADR-0060.
    ///
    /// **One answer for a load assigned and a load handed straight on**,
    /// so that `use(*pi)` and `a = *pi; use(a);` cannot be told apart.
    pub(super) fn contents_read(
        &self,
        function: &Function,
        place: &Place,
        value: &Known,
    ) -> Levels {
        if place.projection.is_empty() {
            return value.points_to[place.local].contents_at_most.clone();
        }
        let mut levels = self
            .unit
            .place_ty(function, place)
            .and_then(|ty| self.type_levels(ty))
            .unwrap_or_default();
        let depth = derefs(place);
        if depth > 0 {
            levels.bound_by(
                &value
                    .memory_levels(place.local, depth, self.deepest_pointer)
                    .below(1),
            );
        }
        levels
    }

    /// How many bytes a pointer read through `place` is believed to point
    /// at: the type it is read as, no more than the pointer gone through
    /// says its memory holds that many levels down, nor than the sites it
    /// may be read out of, or any on the way to them, say was stored there
    /// ([`Known::memory_levels`]). `0` where nothing is believed, **and `0`
    /// where nothing on the way of the read recorded a width for the pointer
    /// read**, so that `Known::memory_levels` answers no bound: a route that
    /// reaches memory without recording a width has read none, and believing
    /// the type there is how seventeen routes were silent from #622 to #724.
    /// See ADR-0061.
    ///
    /// **One answer for every reader of a load**: a load assigned, a load
    /// handed straight to a call or to a library function, and a deeper
    /// level of an access. **At every depth**: `**pi` with `pi` a `void ***`
    /// converted to `int ***` reads what `pi` arrived believing two levels
    /// down, where asking it only at one dereference believed the type
    /// (#634). See ADR-0060.
    pub(super) fn believed_read(&self, function: &Function, place: &Place, value: &Known) -> u64 {
        let as_read = self.pointee_size(function, place).unwrap_or(0);
        let depth = derefs(place);
        // **As its type where the read is reported as indeterminate**:
        // nothing was stored in fresh memory, so nothing records a width,
        // and the defect is the read, reported once where it happens, as the
        // free and the `realloc` doubts are. Asked here rather than where a
        // load is assigned, so that a load handed straight on is answered
        // the same (ADR-0057, ADR-0061).
        if depth == 0 || self.reads_fresh(function, place, value) {
            return as_read;
        }
        let bound = value
            .memory_levels(place.local, depth, self.deepest_pointer)
            .at(0);
        as_read.min(bound.unwrap_or(0))
    }

    /// Whether an object of type `ty` holds a pointer: a pointer, or an array
    /// of them through every dimension. What [`Known::init`] watches,
    /// since a pointer is what ADR-0057 reports read out of memory nothing
    /// wrote, and an integer or a byte is not followed there either.
    ///
    /// Written out rather than as a `matches!`, so a kind of type added later
    /// is asked whether it holds one.
    pub(super) fn holds_a_pointer(&self, ty: TyId) -> bool {
        match self.unit.ty(innermost_element(self.unit, ty)) {
            Ty::Pointer(_) => true,
            Ty::Int | Ty::Char | Ty::Void => false,
            Ty::Array { .. } => unreachable!("`innermost_element` strips every array"),
        }
    }

    /// **What a write pours into memory**, at the store and at the library
    /// copy: what it carries, into the allocations it lands in, and, where it
    /// may land where this check cannot place it, into every allocation code
    /// this check cannot read can name. One function with one arm per kind
    /// of write where the doors differ, so that a rule changed for one is
    /// read beside the other (#737).
    ///
    /// A store that may land unplaced lands so only where it stores a
    /// pointer; a copy lands so whatever its type, since it copies bytes. A
    /// store exposes what it carries where it is unplaced or its pointer is
    /// unnamed, and tells a pending read where what it lands in is already
    /// reachable; a copy exposes what it was handed afterwards, in
    /// [`Self::initialised_by`]. Each as that door did. See ADR-0044.
    fn land_contents(
        &self,
        value: &mut Known,
        landing: &Landing,
        poured: &Cell,
        in_part: &[LocalIdx],
        pours: Pours,
    ) {
        // Every field named, so that one added to `Landing` is answered here
        // (E0027): what the landing says it lands in is poured below, and the
        // rest is asked through its methods.
        let Landing {
            containers,
            targets: _,
            one_step: _,
            pointer: _,
            deep: _,
            kind: _,
            // Whether a write is certain is the unwritten belief's question,
            // `Landing::certain`; contents are poured where it may land.
            through_unrecorded: _,
        } = landing;
        let unplaced = landing.unplaced();
        let unnamed = landing.unnamed(value);
        let elsewhere = unplaced || unnamed || landing.handed_over(value);
        match pours {
            Pours::Store {
                pointer_stored,
                carried,
                span,
            } => {
                if elsewhere && pointer_stored {
                    value.landed_where_unknown_code_points(poured);
                }
                // **A doubt the store kept on a block it may have missed
                // survives its exposure**, which writes every block code this
                // check cannot read now reaches: `*o = m1;` before `v = *t;
                // *v = x;` with `*t` either `m1` or `m2` lost the doubt about
                // `m1` that `initialised_by` had just kept (#754), where the
                // copy's door puts its marks back after its own exposure.
                // Found by review.
                let before = value.init.clone();
                let keep = |value: &mut Known| {
                    for &site in containers.iter() {
                        if before[site] == Init::Maybe {
                            value.init[site] = Init::Maybe;
                        }
                    }
                };
                if unplaced {
                    // No container, so what is poured is the arrays it marks
                    // alone.
                    value.pour(&[], poured, in_part);
                    value.expose(self.sources, carried, Some(span));
                    keep(value);
                } else {
                    if unnamed {
                        value.expose(self.sources, carried.clone(), Some(span));
                        keep(value);
                    }
                    value.pour(containers, poured, in_part);
                    // Stored where code this check cannot read already
                    // reaches is reachable to it from now on, which no call
                    // marks until the next one: a read before this store in
                    // the same expression is told here.
                    let reachable = value.reachable_now();
                    if containers
                        .iter()
                        .any(|container| reachable.contains(container))
                    {
                        let routes = value.closure(carried);
                        value.noticed(self.sources, Some(span), &routes);
                    }
                }
            }
            Pours::Copy => {
                if elsewhere {
                    value.landed_where_unknown_code_points(poured);
                }
                value.pour(containers, poured, in_part);
            }
        }
    }

    /// Bound what a call this check cannot read may write into `local`,
    /// reached on the walk a departure takes as `read_as`, at its own level
    /// and below, and record it where that type is untyped somewhere below,
    /// so a later call walks there again ([`Known::handed_untyped`]). One
    /// place for the call's own arguments and for what an earlier call kept
    /// (#679), so the two cannot read a local differently.
    fn handed(&self, value: &mut Known, local: LocalId, read_as: Option<TyId>) {
        // Reached as a pointer, the body stores one of that type; reached as
        // `void`, through a `void *`, it may store any pointer, so nothing is
        // believed.
        //
        // **Reached as bytes, a local is `void`'s case** (#675): `memcpy` puts
        // any pointer in bytes, and is asked nothing of their type, so
        // `put(pv)` with `put` taking `char (*)[8]` left `v` believing its
        // declared type at every level, and a one-byte allocation was handed on
        // in silence. Whatever the local is declared as: a `char` buffer filled
        // so and read back as a pointer through a `void *` was silent too,
        // where a `void *` callee was doubted. Found by review. See ADR-0060.
        //
        // **And reached below a type that stops being a pointer, the same**
        // (#677): a callee handed `&v` as bytes or through a `void *` may read
        // `&w` back out of `v` and store any pointer into `w`, so `use1(w)`
        // handed a one-byte allocation on in silence while `use1(*v)` was
        // doubted. The walk does not say whether the type was reached through a
        // typed pointer, so one reached below memory a typed callee holds an
        // `int` in, which a may-set says may hold a local's address, is doubted
        // too, and so is one a later departure reaches only through such a
        // may-set, named in a report of its own, by the user's decision. See
        // ADR-0060.
        let Some(read_as) = read_as else {
            remember(&mut value.handed_untyped[local], None);
            value.handed_at_most[local] = bounded(value.handed_at_most[local], Some(0));
            value.handed_contents_at_most[local]
                .bound_by(&Levels::from_bounds(vec![Some(0); self.deepest_pointer]));
            return;
        };
        // Recorded, so a later call walks below it again. See
        // `Known::handed_untyped`.
        if self.untyped_below(read_as) {
            remember(&mut value.handed_untyped[local], Some(read_as));
        }
        let (width, levels) = self.handed_bounds(Some(read_as));
        value.handed_at_most[local] = bounded(value.handed_at_most[local], Some(width));
        value.handed_contents_at_most[local].bound_by(&levels);
    }

    /// What a call this check cannot read may store in memory it reached as
    /// `read_as`: how wide the pointer it stores there points, and what that
    /// pointer's own memory holds, level by level. `0` at every level where
    /// it was reached as `void` or as bytes, `None`, since such a callee may
    /// store any pointer. One answer for a local ([`Self::handed`]) and for a
    /// site ([`Self::handed_site`]), so the two cannot read a type
    /// differently.
    ///
    /// No case reaches the `None` arm through a site, nor tells the levels
    /// below a site's own from nothing: only the width a site is handed at is
    /// held, by `a_pointer_read_out_of_memory_a_call_was_handed_is_not_reported_as_indeterminate`.
    fn handed_bounds(&self, read_as: Option<TyId>) -> (u64, Levels) {
        let nothing = || Levels::from_bounds(vec![Some(0); self.deepest_pointer]);
        let Some(read_as) = read_as else {
            return (0, nothing());
        };
        // An array of pointers holds pointers, as `type_levels` reads it.
        let width = match self.unit.ty(innermost_element(self.unit, read_as)) {
            Ty::Pointer(pointee) => self.unit.size_of(pointee).unwrap_or(0),
            Ty::Void | Ty::Int | Ty::Char => 0,
            Ty::Array { .. } => {
                unreachable!("`innermost_element` strips every array")
            }
        };
        // **And below it, at every level**, as the type it was reached as says:
        // through a `void *`, nothing at any level. See
        // `Known::handed_contents_at_most`. Through an array to its element, as
        // the width above reads it: `fill(&a)` with `a` an `int **[2]` took `a`
        // as reached through a `void *`. Found by review.
        let levels = self
            .type_levels(innermost_element(self.unit, read_as))
            .unwrap_or_else(nothing);
        (width, levels)
    }

    /// Record memory a call this check cannot read reached as `read_as`,
    /// where that type is untyped somewhere below. See
    /// [`Known::handed_untyped`].
    ///
    /// **And bound what it may store there**, as [`Self::handed`] bounds a
    /// local: what the callee stores is believed as it was handed, and a
    /// site records nothing else, so `init(box); int *t = *box;` with `init`
    /// taking `int **` believed nothing once a load believes nothing that no
    /// width was recorded for (ADR-0061).
    fn handed_site(&self, value: &mut Known, site: Site, read_as: Option<TyId>) {
        if read_as.is_none_or(|ty| self.untyped_below(ty)) {
            remember(&mut value.handed_untyped[site], read_as);
        }
        let (width, levels) = self.handed_bounds(read_as);
        value.contents[site].bound_stored(&levels.stored_under(Some(width), self.deepest_pointer));
    }

    /// Whether a callee handed a pointer to `ty` may store any pointer
    /// somewhere below it: `ty` holds no pointer, or is a pointer whose chain
    /// ends in `void` or `char`. Not one that ends in `int`, which a
    /// well-typed `int **` out-parameter does. See [`Known::handed_untyped`].
    ///
    /// Written out rather than as a `matches!`, so a kind of type added later
    /// is asked what it is.
    fn untyped_below(&self, ty: TyId) -> bool {
        match self.unit.ty(innermost_element(self.unit, ty)) {
            Ty::Pointer(pointee) => {
                let mut below = innermost_element(self.unit, pointee);
                while let Ty::Pointer(next) = self.unit.ty(below) {
                    below = innermost_element(self.unit, next);
                }
                match self.unit.ty(below) {
                    Ty::Void | Ty::Char => true,
                    Ty::Int => false,
                    Ty::Pointer(_) | Ty::Array { .. } => {
                        unreachable!("the loop strips every pointer and array")
                    }
                }
            }
            Ty::Void | Ty::Int | Ty::Char => true,
            Ty::Array { .. } => unreachable!("`innermost_element` strips every array"),
        }
    }

    /// Whether a place is a byte: of character type, which C17 6.5 p7 lets read
    /// any object, a pointer included.
    ///
    /// Written out rather than as a `matches!`, so a kind of type added later
    /// is asked whether it can copy a pointer too. See ADR-0046.
    pub(super) fn reads_a_byte(&self, function: &Function, place: &Place) -> bool {
        match self
            .unit
            .place_ty(function, place)
            .map(|ty| self.unit.ty(ty))
        {
            Some(Ty::Char) => true,
            Some(Ty::Pointer(_) | Ty::Int | Ty::Void | Ty::Array { .. }) | None => false,
        }
    }

    /// What a value read through a projection holds: what memory records
    /// there, `Known::load_into`, marked as a load where it may be a pointer
    /// or a byte of one, so that the report proves nothing with it.
    ///
    /// One answer for the three places a load is given to something: an
    /// assignment, a write through a pointer, and what a library copy returns
    /// when handed one. One rule in several places drifts apart.
    /// See ADR-0067.
    fn read_through(&self, function: &Function, source: &Place, value: &Known) -> Held {
        // Every mark a load gives before it reads anything, each said, so a
        // mark added is answered here (#732); what it reads is
        // `Known::load_into`'s, and whether it is fresh is below.
        let mut held = Held::from_marks(
            value.points_to.len(),
            Marks {
                lost: false,
                freed: None,
                writes_elsewhere: true,
                offset: Offset::Zero,
                // **A byte is a load too.** C17 6.5 p7 lets a character type
                // read any object, so a pointer copied a byte at a time is
                // copied, and the byte carries what was stored where it was
                // read from, marked as a load is so that it proves nothing
                // about what it names. See ADR-0046.
                loaded: self.may_be_pointer(function, source)
                    || self.reads_a_byte(function, source),
                foreign: false,
                stale_read: false,
                unreplaced_read: false,
                lost_read: false,
                from_caller: false,
                returned_by: None,
                fresh_read: false,
                // **Believed to point at one whole object of the type it is
                // read as**, which the store that put it there is asked about.
                // A byte is no pointer and believes nothing; a `void *`
                // believes it starts an object and no width, which is all its
                // store is asked. No more than the memory it is read out of is
                // believed to hold ([`Self::believed_read`]), and only where
                // every local it may be read out of fits, below. See ADR-0060.
                believed: self
                    .pointee_size(function, source)
                    .map(|_| self.believed_read(function, source, value)),
                // **And it arrives as the type it is read as**, which bounds
                // what its own memory holds, no more than the memory it was
                // read out of believed one level down: `a = *pi;` with `pi` a
                // `void ***` converted to `int ***` believes nothing of what
                // `*a` points at, where the type alone said whole `int`s
                // (#634). See `Held::contents_at_most`.
                //
                // Mutation: take the type alone, and
                // `a_void_pointer_three_levels_deep_converted_whole_is_believed_below_its_first_level`
                // goes silent.
                contents_at_most: self.contents_read(function, source, value),
                inner_address: false,
                element: None,
                array_edge: false,
                stored_through: false,
            },
        );
        // What could be named as this was read. See `Held::named_at_read`.
        if held.loaded {
            for named in value.locals_named_by_unknown_code() {
                held.named_at_read.set(named);
            }
        }
        // **And what was stored where it was read from**, at an offset nobody
        // said, so that a dereference of it can be asked about what it may
        // point at. `loaded` stays set, which is what marks the set as
        // possibly incomplete for every reader. See ADR-0045.
        let depth = derefs(source);
        if held.loaded && depth > 0 {
            // What memory holds there, and the marks a load out of it
            // carries, read as `built::built_from` reads them. See
            // `Known::load_into`.
            let named_a_local = value.load_into(
                &mut held,
                source.local,
                depth,
                &|local| self.is_array(function, local),
                self.reads_caller_memory(source.local, depth, value),
            );
            if named_a_local {
                held.writes_elsewhere = true;
            }
            // **Believed only where every local it may be read out of fits
            // what it is read as.** A local is no memory with a door: an
            // assignment to it is asked nowhere, and a `void *` a callee
            // wrote there was asked its start and no width. So each such
            // local is asked what it holds, as an access through its address
            // asks it (ADR-0060), without doubting the escape its address
            // is. `int *q = *pi; *q = 1;` with `v = p + 4` after `pi = &v`
            // is unknown at `*q` for it (#627).
            if held.believed.is_some_and(|width| {
                !self.read_from_fits(function, source.local, depth, width, value)
            }) {
                held.believed = None;
            }
            // The question the report asks at this read, so that a value
            // carrying the mark is always one reported where it was read.
            // See `Self::reads_fresh` and ADR-0057.
            held.fresh_read = self.reads_fresh(function, source, value);
        }
        held
    }

    /// Whether every local a pointer read `depth` dereferences below `local`
    /// may be read out of holds a value that fits `width` bytes, as
    /// [`fits`] answers its own value with no doubt of unseen writes: what a
    /// belief in the type a pointer is read as needs, since a local is not
    /// memory with a door and `v = p + 4;` after `pi = &v;` was never asked
    /// where it was assigned. One answer for a load and for a library copy,
    /// which is a load of its source one level further (#777); the copy
    /// asking none believed a pointer a call wrote and this function moved
    /// as a whole `int` at its start. See ADR-0060.
    fn read_from_fits(
        &self,
        function: &Function,
        local: LocalId,
        depth: usize,
        width: u64,
        value: &Known,
    ) -> bool {
        // One level short of `depth`: the locals the value is read out of,
        // not those whose address the value may be. `fit::read_from_locals`
        // takes the same level for an access.
        let read_from = value.levels_below(local, depth - 1).1;
        function
            .locals()
            .filter(|each| read_from.contains(&LocalIdx::of(each)))
            .all(|each| {
                matches!(
                    fits(self, function, each, width, value, false),
                    None | Some(Fit::Inside)
                )
            })
    }

    /// Whether a load `depth` dereferences through `local` reads memory a
    /// caller or a callee handed over: memory a pointer parameter points at,
    /// or what a call this check cannot read returned, which is the same
    /// fact for the other side of a call (#394). For a parameter: whether
    /// `local` holds the site of one, or
    /// was itself read out of such memory, or the load reads through an
    /// allocation one was stored in, at any level. The second is the same read
    /// one level further in, `q = **ppp` spelled `pp = *ppp; q = *pp;`, and
    /// without it `q` held nothing a call could make lost; the third is the
    /// same read after a store, `*box = q; r = *box;` or `r = **bb;`, which
    /// a cell's `from_caller` says. Below the first level a load also reads
    /// what is reachable to code this check cannot read, in
    /// [`Known::levels_below`] (#486), which doubts `r = **bb;` without the
    /// third; the third still decides whether `free(**bb)` frees what a
    /// caller handed over, and so whether a pointer read out of another
    /// parameter is doubted after it. Found by review. Only the parameters
    /// `exposed_parameters` names,
    /// so what the host hands `main` is not this, as it is not exposed. One
    /// answer for a load assigned and a load as an operand, so that the two
    /// cannot disagree. See ADR-0040.
    fn reads_caller_memory(&self, local: LocalId, depth: usize, value: &Known) -> bool {
        let held = &value.points_to[local];
        // At least the local's own allocations, which a load with no
        // dereference in its place, an element, still reads through.
        held.from_caller
            || value.marked_below(local, depth.max(1), |contents| contents.holds_from_caller())
            || value.caller_through(local, depth)
            // **Only for a local this check lost**, measured: an exposed
            // parameter's site is marked `from_caller` where the function
            // starts and nothing clears it, so `marked_below` answers for it
            // first, except where `level_zero` refuses a lost local. A load
            // through a lost local is doubted on its own, so no program shows
            // this either: three that load through a parameter lost on one
            // arm printed the same without it. The unit test
            // `a_lost_local_holding_a_parameters_site_reads_caller_memory` is
            // the guard.
            || self
                .exposed_parameters
                .iter()
                .any(|&parameter| held.has_site(Site::of(LocalIdx::of(parameter))))
            // Or what a call this check cannot read handed back: memory its
            // callee may still hold, and free at the next such call (#394).
            // **The `Callee::Opaque` arm answers for it first**, by marking
            // the cell of what the call returned `from_caller`, so no program
            // shows this clause alone, measured; the unit test
            // `a_local_holding_what_an_opaque_call_returned_reads_callee_memory`
            // is the guard.
            || held.sites().any(|site| self.opaque_results[site.local()])
    }

    /// Whether a free or `realloc` handed these arguments may free what the
    /// caller owns: an argument holding an exposed parameter's site, or read
    /// out of what one points at, or a place of dereferences whose load reads
    /// caller memory, `free(*pp)`. Not an allocation this function made,
    /// which the caller cannot have handed it. See ADR-0040.
    fn frees_callers(&self, handed: &[Operand], value: &Known) -> bool {
        handed.iter().any(|argument| {
            let Operand::Copy(place) = argument else {
                return false;
            };
            if place.projection.is_empty() {
                let held = &value.points_to[place.local];
                held.from_caller
                    || self
                        .exposed_parameters
                        .iter()
                        .any(|&parameter| held.has_site(Site::of(LocalIdx::of(parameter))))
            } else {
                let depth = derefs(place);
                depth > 0 && self.reads_caller_memory(place.local, depth, value)
            }
        })
    }

    /// What an operand may reach beyond the sites it holds, because it may be
    /// a pointer read out of memory.
    ///
    /// Read through one `Deref` of a local holding sites, with nothing
    /// `Beyond::unrecorded_address` names besides (a load, a pointer it lost,
    /// one a call wrote), what those allocations may contain. Read any other
    /// way, or held by a local that may hold such a pointer, what is stored
    /// anywhere, [`Known::stored`]. **A local merely holding no site answers
    /// nothing**: `int *z = 0;` is one, and is not a load. What the report
    /// reads is not this, and is `Known::reached_by`'s. See ADR-0067.
    fn read_out(&self, function: &Function, operand: &Operand, known: &Known) -> Vec<Site> {
        let Operand::Copy(place) = operand else {
            return Vec::new();
        };
        // **A byte read out of memory handed to a call reaches what was stored
        // where it was read**, as a byte assigned first does: a callee can keep
        // the bytes of a pointer and put it back together. Never everything
        // stored anywhere, which is what refused a character handed to a call
        // in correct string code. See ADR-0046.
        let depth = derefs(place);
        if depth > 0 && self.reads_a_byte(function, place) {
            return known
                .levels_below(place.local, depth)
                .0
                .into_iter()
                .collect();
        }
        if !self.may_be_pointer(function, place) {
            return Vec::new();
        }
        let held = &known.points_to[place.local];
        if place.projection.is_empty() {
            return if held.beyond().unrecorded_address() {
                known.stored()
            } else {
                Vec::new()
            };
        }
        if place.projection.as_slice() == [Projection::Deref]
            && !held.beyond().unrecorded_address()
            && held.sites().next().is_some()
        {
            return held
                .sites()
                .flat_map(|container| known.contents[container].sites())
                .collect();
        }
        known.stored()
    }

    /// Everything a call handed these arguments can reach by itself: the sites
    /// they name, what every escaped local holds, and what an argument read
    /// out of memory may be. Not closed over what those allocations hold.
    ///
    /// One function for the two arms of the transfer that expose it and for
    /// [`used_before`](super::report::used_before), which asks a read carried to the call about it, so
    /// that what a call is asked about and what it is taken to have reached
    /// cannot disagree. See ADR-0039 and ADR-0040.
    pub(super) fn reach(
        &self,
        function: &Function,
        handed: &[Operand],
        named: impl Iterator<Item = Site>,
        known: &Known,
    ) -> Vec<Site> {
        let mut reach = known.reach_of(named);
        // **And what an argument read out of memory may be.** It names no
        // site, and "no site" is what the report is told; what the callee can
        // reach through it is whatever was stored where it was read from.
        // `release(*tab);` frees what `tab` held. See ADR-0040.
        for argument in handed {
            reach.extend(self.read_out(function, argument, known));
        }
        reach
    }

    /// Whether a call to this function may have freed any allocation still
    /// live, whatever it was handed: a hatch, whose unproven conclusions are
    /// listed rather than reported. See ADR-0038.
    pub(super) fn frees_anything(&self, callee: FuncId) -> bool {
        self.unit.function(callee).hatch()
    }

    /// Which locals a call this check cannot read writes its result into,
    /// for [`Self::opaque_results`].
    ///
    /// Asked of [`Self::callee`], so that which calls are read by name is
    /// said in one place. A destination with a projection is not a site, as
    /// the call transfer says, so it marks nothing.
    pub(super) fn opaque_results_of(&self, function: &Function) -> Table<LocalIdx, bool> {
        let mut marks: Table<LocalIdx, bool> = Table::filled(false, function.locals().len());
        for block in function.blocks() {
            if let Terminator::Call {
                callee,
                arguments: _,
                destination: Some(place),
                then: _,
                origin: _,
            } = &block.terminator
            {
                // Every kind written out, so that one added later is asked
                // whether what it returns was handed over.
                let handed_over = match self.callee(*callee) {
                    Callee::Opaque => true,
                    Callee::Frees
                    | Callee::Allocates
                    | Callee::Reallocates
                    | Callee::ReturnsFirst
                    | Callee::Copies
                    // Returns nothing as the library declares it. One redeclared
                    // to return a pointer is undefined and believed all the
                    // same, as ADR-0051 believes it.
                    | Callee::Ends => false,
                };
                if handed_over && place.projection.is_empty() {
                    marks[place.local] = true;
                }
            }
        }
        marks
    }

    /// The locals a call this check cannot read is handed the address of: what
    /// an argument by name may point at, and the locals a place of
    /// dereferences may be. One answer for what the call exposes and for
    /// `report::handed_stored_out`, which asks what those locals hold, since
    /// `release_pp(&x)` hands the call `x`'s allocation where no argument
    /// names it (#672, found by review). See `Known::expose_locals`.
    pub(super) fn locals_handed_to_opaque_call(handed: &[Operand], value: &Known) -> Vec<LocalIdx> {
        let mut handed_locals: Vec<LocalIdx> = Vec::new();
        for argument in handed {
            if let Operand::Copy(place) = argument {
                if place.projection.is_empty() {
                    handed_locals.extend(value.written_through(place.local));
                } else {
                    handed_locals.extend(value.levels_below(place.local, derefs(place)).1);
                }
            }
        }
        handed_locals
    }

    /// What a call this check cannot read is handed: what it is handed by
    /// name or as a load, and what an argument read out of memory may be.
    /// Every allocation those hold, however deep, `Known::closure` of it, is
    /// what the call's transfer marks `may_be_freed`, and
    /// `report::handed_stored_out` asks the same set whether any was stored
    /// where code outside may read it (#672). See ADR-0047 and
    /// `Known::stored_out`.
    pub(super) fn handed_to_opaque_call(
        &self,
        function: &Function,
        handed: &[Operand],
        value: &Known,
    ) -> Vec<Site> {
        let touched = Self::touching(handed.iter(), value);
        let mut handed_memory: Vec<Site> = named(&touched).collect();
        for argument in handed {
            handed_memory.extend(self.read_out(function, argument, value));
        }
        handed_memory
    }

    /// What the arguments of a call reach, in the order they were written.
    ///
    /// A local that reaches no site, is a load, or may be a local's address
    /// reaches [`Reached::Lost`] as well as whatever sites it has: what it
    /// hands the call is not, or not only, an allocation this check names.
    ///
    /// Shared with [`findings`](super::findings), so that the walk which reports and the walk which
    /// computes cannot disagree about what a call touches.
    ///
    /// **What they may disagree about is which arguments are handed here**, and
    /// exactly one thing does it: `asked` leaves out a pointer the nullability
    /// check established is null, because C says such a call does nothing, and
    /// only the walk that reports asks that. See ADR-0027, and `asked` for why
    /// the transfer is deliberately not told.
    pub(super) fn touching<'o>(
        arguments: impl IntoIterator<Item = &'o Operand>,
        known: &Known,
    ) -> Vec<Reached> {
        let mut reached = Vec::new();

        for argument in arguments {
            let place = match argument {
                // Not a pointer that went missing. `free(0)` is the case, and
                // C17 7.22.3.3 p2 makes it do nothing, so it is written on
                // purpose and is not something this check lost track of.
                //
                // **The arm is wider than the clause**, which covers the null
                // pointer and makes every other address undefined. `free(17)`
                // is skipped here too and this check says nothing about it.
                // What keeps that from mattering is not this line: C17
                // 6.5.2.2 p2 makes the call a constraint violation, so a
                // conforming implementation has to diagnose it before any
                // analysis runs. This frontend does, `SC0302` since #356, and
                // its driver runs no analysis on a tree the type check
                // reported, since #354. `a_free_of_a_null_constant` pins the
                // half the clause supports; the other half is held by nobody
                // here and is not this check's to hold.
                //
                // **The same clause reaches a local**, where the constant was
                // given a name first, and that is `asked`'s rather than this
                // arm's: what it takes to recognise is a nullness the other
                // check established, which nothing here can see. See ADR-0027.
                Operand::Constant(_) => continue,
                Operand::Copy(place) => place,
            };

            if !place.projection.is_empty() {
                // `free(*pp)` frees whatever `pp` points at, and this check
                // follows locals rather than what they point at.
                reached.push(Reached::Lost(LostReason::ReadOutOfMemory));
                continue;
            }

            let before = reached.len();
            reached.extend(known.reached_by(place.local));
            // **A load is something this check cannot fully name**, whatever
            // sites it holds, so a free of it stays the doubt it was rather
            // than a free of those sites alone: answering them without this
            // made `free(q)` of a load say less. See ADR-0045.
            //
            // **Nor is a pointer that may be a local's address.** On the path
            // that took `&x` a free of it, or a `realloc`, releases no
            // allocation at all. The join keeps the sites the other path
            // brought and drops that one, so `if (c) r = &x; free(r);` was
            // read as a free of the allocation and said nothing. See
            // `Held::may_be_a_locals_address`.
            //
            // **Nor is a pointer a call this check cannot read wrote**, which
            // may be any pointer. Alone it names no site and the first row
            // doubts it, in that row's words; joined with an arm that named
            // `q`, it names `q`'s site and nothing else said so, and
            // `get(&u); if (c) r = u; else r = q; free(r);` was read as a free
            // of `q` while `u` may be one already freed (#497). Asked only
            // where it names a site, so that the first row's words stand for
            // an output parameter alone. See ADR-0044.
            let held = &known.points_to[place.local];
            // Which of the four it is, as far as this can tell: one alone
            // names its reason, and more than one is `Other` unless one is
            // `MayBeALocal`, which `LostReason::joined` keeps (#213).
            let reasons = [
                (reached.len() == before, LostReason::NoSiteKnown),
                (held.loaded, LostReason::ReadOutOfMemory),
                (held.may_be_a_locals_address(), LostReason::MayBeALocal),
                (held.foreign && reached.len() > before, LostReason::Other),
            ];
            if let Some(reason) = reasons
                .iter()
                .filter(|(applies, _)| *applies)
                .map(|&(_, reason)| reason)
                .reduce(LostReason::joined)
            {
                reached.push(Reached::Lost(reason));
            }
        }

        reached
    }

    /// Whether this call is handed a local that may hold an allocation this
    /// check cannot name.
    ///
    /// **A second question about the same arguments**, and it cannot be folded
    /// into [`Allocations::touching`]: that answers [`Reached::Lost`] for an
    /// escaped local as well, which is ADR-0017, and the two facts are not
    /// interchangeable here. Freeing an escaped local that no call has run past
    /// still frees what this check thinks it holds, and reading the folded
    /// answer instead would give up the proof
    /// `a_free_through_an_escaped_local_is_seen_by_a_sharer` holds. One rule
    /// written in two places drifts apart, so the argument walk is
    /// spelled the way its twin above spells it and the reason there are two is
    /// written here. See ADR-0029.
    ///
    /// **Neither arm is observable, and both are here anyway.** `free` takes
    /// one argument, so the argument this walk skips is the only argument
    /// there is, and the branch that reads this then writes on nothing:
    /// answering `true` for a constant, and dropping the projection test, each
    /// leave the whole workspace green, measured. The `reached.len() > 1`
    /// branch says the same thing about the same premise. What they would cost the day something
    /// reaches them is a free refusing to prove because of a row belonging to
    /// a pointer rather than to what it points at. The arm above them is the
    /// one that decides anything.
    fn holds_something_unnameable(arguments: &[Operand], known: &Known) -> bool {
        arguments.iter().any(|argument| match argument {
            // A constant holds no allocation, for the reason `touching` gives.
            Operand::Constant(_) => false,
            // A projection names a place rather than a local, and this check
            // follows locals: `free(*pp)` is already a `Reached::Lost` above,
            // and there is no row here to ask.
            // A load holds what was stored where it was read from, which may
            // not be all it holds, so a free of it does not say which went
            // either. See ADR-0045.
            //
            // A pointer that may be a local's address may have freed none of
            // its sites, for the reason `touching` gives. Asked here too, or
            // a later free of one of those sites is proved a double free on
            // the path that took `&x`, where nothing freed it.
            Operand::Copy(place) => {
                let held = &known.points_to[place.local];
                place.projection.is_empty()
                    && (held.beyond().unrecorded_address() || held.may_be_a_locals_address())
            }
        })
    }

    /// **A branch on what a `realloc` returned says what became of what it was
    /// handed.** On the arm where the pointer is null the call failed and the
    /// old allocations are live again; on the other it succeeded and they were
    /// freed at the call. Only where the tested pointer holds exactly the one
    /// allocation the fact is about, and is not one this check stopped
    /// following. Which local the branch tested is the nullability check's
    /// answer, shared, so the two checks read one branch alike. See ADR-0039.
    fn branch_on_reallocs_result(
        &self,
        function: &Function,
        block: BlockId,
        terminator: &Terminator,
        index: usize,
        value: &mut Known,
    ) {
        let Terminator::Branch {
            condition: Operand::Copy(condition),
            ..
        } = terminator
        else {
            return;
        };
        let Some((local, on_then)) = crate::nullability::tested_against_null(
            self.unit, function, &self.cfg, block, condition,
        ) else {
            return;
        };
        let held = &value.points_to[local];
        let mut sites = held.sites();
        let (Some(returned), None) = (sites.next(), sites.next()) else {
            return;
        };
        if held.lost || held.returned_by != Some(returned) {
            return;
        }
        let Some(fact) = value.realloced[returned].clone() else {
            return;
        };
        // `Terminator::successors` pushes `then` and then `otherwise`, so
        // index 1 is the other arm, as the nullability check reads it.
        let null = (index == 0) == (on_then == crate::nullability::Nullness::Null);
        for (site, made) in fact.old {
            if null {
                if matches!(value.state[site], SiteState::Unknown) {
                    value.state[site] = SiteState::Live(made);
                }
            } else {
                value.state[site] = SiteState::Freed {
                    made,
                    freed: Freeing {
                        at: fact.at,
                        sequenced: true,
                    },
                };
            }
        }
    }

    /// [`Analysis::join`], and along a loop's back edge [`Analysis::join_back`]
    /// where `back` is set, which it is for a loop shown to run at least
    /// once, `Allocations::runs_once`: one body, so the two cannot drift
    /// apart. They differ only in what a local unwritten on one side and
    /// written on the other becomes. See `Known::init`.
    fn joined(&self, into: &mut Known, from: &Known, back: bool) {
        // **Every field named, never `..`.** A lattice value whose join
        // forgets a field reaches a fixpoint over a value nobody is joining,
        // and nothing else in the build says so: the field is read, the walk
        // ends, and the answer is wrong on exactly the programs a join is for.
        // This is the same as `..` letting a field walk past a match that was
        // otherwise exhaustive, one type over.
        let Known {
            points_to,
            state,
            escaped,
            handed_away,
            exposed,
            contents,
            init,
            realloced,
            pending,
            calls,
            exposed_after_call,
            lost_exposed,
            extent,
            handed_at_most,
            handed_contents_at_most,
            element_init,
            exposed_in_part,
            stored_in_part,
            stored_out,
            handed_untyped,
        } = into;
        // Which locals this side left unwritten, before `unwritten` is
        // intersected below: such a local is no null, and the values' join
        // asks it (#719). See `Held::joined`.
        let unwritten_here: Table<LocalIdx, bool> = Table::from_vec(
            init.locals()
                .map(|local| init[local].is_unwritten())
                .collect(),
        );
        // May-facts, so a union, as `exposed` of a local is. See
        // `Known::handed_untyped`.
        for (here, there) in handed_untyped.iter_mut().zip(&from.handed_untyped) {
            for ty in there {
                if !here.contains(ty) {
                    here.push(*ty);
                }
            }
        }
        // A may-fact, so a union, at both kinds of join. See
        // `Known::stored_out`.
        for (here, there) in stored_out.iter_mut().zip(&from.stored_out) {
            *here = *here || *there;
        }
        // Per element, joined as `Init::joined` says. See
        // `Known::element_init`.
        for (here, there) in element_init.iter_mut().zip(&from.element_init) {
            here.join(there, back);
        }
        for (here, there) in exposed_in_part.iter_mut().zip(&from.exposed_in_part) {
            *here = *here || *there;
        }
        for (here, there) in stored_in_part.iter_mut().zip(&from.stored_in_part) {
            *here = *here || *there;
        }
        // **Joined as `Init::joined` says**: unwritten only where no path
        // wrote it, and maybe unwritten where one path wrote it and the other
        // left it unwritten, which is the doubt itself, but not at the back
        // edge of a loop shown to run. An array kept per element has its own
        // doubt above, and its record here gains none, at either kind of
        // join. See `Known::init`.
        //
        // **One loop over every place, each kind by its own rule** (#779).
        // Both rules read `state` as it was before this join: `state` is
        // joined below, and must not be before this loop.
        for place in init.indices() {
            let (here, there) = (init[place], from.init[place]);
            init[place] = match place {
                // Kept as the two flags joined, though it decides no case,
                // measured: nothing reads a kept array's whole record for a
                // doubt, `Known::may_be_unwritten` asking its elements.
                Loc::Local(local) => {
                    let kept = element_init[local].len() > 0;
                    here.joined(there, back || kept)
                }
                // **Joined as `Init::joined` says** where both sides name the
                // site, fresh where every path left it fresh, since a use of
                // what was read out of it is reported as a proof; a union here
                // reported `if (c) *box = q; t = *box; *t = 1;`, which C
                // defines when `c` holds. A site one side left fresh and the
                // other did not may still be fresh, at a forward join only
                // (#721).
                //
                // **Where one side does not name the site, `Unheld`**, the
                // side that names it says, as `state`'s join takes the state
                // of the side that holds it: a block made on one arm only, or
                // in last turn's body, kept its mark (#724), and `if (k) *m =
                // x; else { free(m); m = 0; }` does not doubt a read of `*m`
                // on the arm that wrote it. One rule for both sides, so that
                // what guards one guards the other: measured, only one
                // orientation had a case of its own. See ADR-0048.
                Loc::Heap(site) => match (&state[site], &from.state[site]) {
                    (_, SiteState::Unheld) => named_beside_unheld(here, there),
                    (SiteState::Unheld, _) => named_beside_unheld(there, here),
                    _ => here.joined(there, back),
                },
            };
        }
        // Bounds, so the lower of two at every level. See
        // `Known::handed_contents_at_most`.
        for (here, there) in handed_contents_at_most
            .iter_mut()
            .zip(&from.handed_contents_at_most)
        {
            here.bound_by(there);
        }
        for (here, there) in handed_at_most.iter_mut().zip(&from.handed_at_most) {
            *here = bounded(*here, *there);
        }
        // A may-fact, so a union, as `exposed` is.
        *lost_exposed |= from.lost_exposed;
        // A side that never made the site says nothing about it. A local's
        // size is the same on both sides, and joins as itself. Two sides
        // that both did are joined as `Extent::joined` says, which only
        // narrows, so being wrong here doubts an access rather than proves
        // one. **`SiteState::Unheld` is not asked**, although it is the
        // state's identity (ADR-0048): a site is one call's destination or one
        // parameter, which makes it with the same size every time, so two
        // sides never bring different sizes for it and there is nothing for
        // the identity to choose between. Measured: a version that took the
        // other side's size where one side was `Unheld` answered every test
        // as this one does. See ADR-0060.
        for (here, there) in extent.iter_mut().zip(&from.extent) {
            *here = match (*here, *there) {
                (Some(ours), Some(theirs)) => Some(ours.joined(theirs)),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            };
        }
        // Kept only where both arms remember the same `realloc`: a fact one
        // arm lacks is one a branch after the join cannot act on. See ADR-0039.
        for (here, there) in realloced.iter_mut().zip(&from.realloced) {
            // And where one arm left an old allocation in another state: a
            // free on one arm leaves it `Unknown` after the join, which the
            // null arm would read as the `realloc`'s. Found by review. Not
            // where one arm left it `Unheld`: nothing names it there, so the
            // join gives it the other arm's state, which is what the fact
            // then acts on. `n = realloc(p, 8); if (c) p = 0;` reaches the
            // branch that way, since the fact holds nothing (#494). Mutation:
            // count `Unheld` as disagreeing;
            // `a_realloc_fact_survives_a_join_where_one_arm_dropped_the_old_pointer`
            // reports at the free on the null arm.
            let disagree = here.as_ref().is_some_and(|fact| {
                fact.old.iter().any(|&(old, _)| {
                    state[old] != from.state[old]
                        && !matches!(state[old], SiteState::Unheld)
                        && !matches!(from.state[old], SiteState::Unheld)
                })
            });
            if here != there || disagree {
                *here = None;
            }
        }

        for (local, (here, there)) in points_to
            .indices()
            .zip(points_to.iter_mut().zip(&from.points_to))
        {
            here.joined(
                there,
                self.sources,
                (unwritten_here[local], from.init[local].is_unwritten()),
            );
        }

        // **A site nothing on one side can still name takes the other side's
        // state**, because `edge` made it `SiteState::Unheld` there, which is
        // the join's identity. On the arm of `if (c) { free(p); p = 0; }` that
        // freed it, nothing read again holds the allocation, so whether it was
        // freed there is about no execution's next use of it, and joining it
        // in made the other arm's live `p` unproven. See ADR-0048.
        for (here, there) in state.iter_mut().zip(&from.state) {
            *here = here.joined(*there, self.sources);
        }

        // A local whose address escaped on one arm has escaped where the arms
        // meet: the other arm did not un-take it.
        for (here, there) in escaped.iter_mut().zip(&from.escaped) {
            *here = *here || *there;
        }
        // **And one handed away on both arms is handed away where they meet,
        // which is an intersection, not the union above.** The fact exempts a
        // holder, so it has to hold on every path: a union exempted `a` after
        // `if (c) stash(pa);` on the path that never handed it, and was
        // silent there. See ADR-0047.
        for (here, there) in handed_away.iter_mut().zip(&from.handed_away) {
            *here = *here && *there;
        }

        // Exposed on one arm is exposed where the arms meet, and what an
        // allocation may hold on either arm it may hold after them. A union,
        // for the reason `escaped` is one, for a block and for a local whose
        // address a call on one arm may have kept alike. See ADR-0039.
        for (here, there) in exposed.iter_mut().zip(&from.exposed) {
            *here = *here || *there;
        }
        // What each allocation may contain, every field at once. See
        // `Cell::absorb`.
        for (here, there) in contents.iter_mut().zip(&from.contents) {
            here.absorb(there);
        }

        // **A union, because a read on either arm is a read some execution
        // performed.** What this costs when it is wrong is a report about a
        // read that did not happen, which the reader sees; the other direction
        // loses the read that did. The arms themselves do not meet before their
        // join, which is what keeps a read on one arm from being reported
        // against a free on the other: each arm is walked from the value that
        // reached it and not from this one.
        for (key, entry) in &from.pending {
            let here = pending.entry(key.clone()).or_insert(PendingRead {
                at: entry.at,
                sites: BTreeSet::new(),
                // Whichever arm got here first, since the key's place
                // decides the kind and two arms cannot disagree about it.
                // Answering `Read::Dereference` here fails
                // `a_pointer_handed_to_a_call_inside_one_arm_before_a_free_survives_the_join`.
                read: entry.read,
                reachable: BTreeSet::new(),
                after_call: false,
            });
            here.sites.extend(&entry.sites);
            here.after_call |= entry.after_call;
            // Reachable on one arm is reachable where they meet, for the
            // reason `exposed` is a union.
            here.reachable.extend(&entry.reachable);
        }
        for (key, span) in &from.calls {
            calls.entry(*key).or_insert(*span);
        }
        exposed_after_call.extend(&from.exposed_after_call);

        // And applying it, which the union alone does not do. [`Known::settle`]
        // says why a join needs this and the assignments do not cover it.
        into.settle();
    }
}

/// What a site is where the paths meet, when one side names it and the
/// other, `SiteState::Unheld`, does not: what the side that names it says,
/// with a doubt the other side still held kept, as the two flags this
/// replaced joined. **That doubt decides no case**, measured; it is kept so
/// that #733 changes no answer. See `Allocations::joined` and ADR-0048.
fn named_beside_unheld(named: Init, unheld: Init) -> Init {
    match unheld {
        Init::Maybe if named == Init::Written => Init::Maybe,
        Init::Unwritten | Init::Maybe | Init::Written => named,
    }
}

impl Analysis for Allocations<'_> {
    /// **Two things happen on an edge, in this order.** A branch on what a
    /// `realloc` returned says what became of what it was handed,
    /// [`Self::branch_on_reallocs_result`]; and a local nothing reads past the
    /// edge holds nothing, so that where the arms meet a site nothing live
    /// still names on one of them takes its state from the other. See
    /// ADR-0048. The `realloc` branch first, because it reads the local the
    /// branch tested, which is often read nowhere after it: cleared first,
    /// three `realloc` cases lost their answer, measured. A local whose
    /// address was taken is never cleared, since [`live_in`](crate::liveness::live_in) counts it live
    /// everywhere: `Rvalue::Address` is the only way an address is taken.
    fn edge(
        &self,
        function: &Function,
        block: BlockId,
        terminator: &Terminator,
        index: usize,
        value: &mut Self::Value,
    ) {
        self.branch_on_reallocs_result(function, block, terminator, index, value);
        let mut successors = Vec::new();
        terminator.successors(&mut successors);
        let Some(successor) = successors.get(index) else {
            return;
        };
        let live = &self.live_in[successor.index()];
        for local in function.locals() {
            if !live[local.index()] {
                value.clear(local);
            }
        }
        // After the clearing, so that a site only a dead local held is one
        // nothing holds. See ADR-0048.
        let held = value.held_sites();
        for (site, state) in value.state.enumerated_mut() {
            if !held[site] {
                *state = SiteState::Unheld;
            }
        }
    }

    type Value = Known;

    fn height(&self, function: &Function) -> usize {
        let locals = function.locals().len();
        // Each local's set of sites only grows, so it takes at most one step
        // per site, and a site is a local. Each site's state walks `Live` to
        // `Freed` or to `Reachable`, and on to `Unknown`, which is still two
        // steps since the join of those two is `Unknown` (ADR-0047); its
        // `freed` span can only move to an earlier
        // one, which it can do at most once per site that frees; and its `made`
        // span can only fall from `Some` to `None`, once. A local's `escaped`
        // bit goes from `false` to `true` and never back, once each, and its
        // `lost` bit costs one more of the same. Each local's set of locals a
        // write through it may reach is a second square table that only grows,
        // so it takes at most one step per pair, and whether that table is all
        // of what a write through it may reach goes from `false` to `true`
        // once per local, for one more step each. Where the set it named was
        // freed goes from `None` to `Some` once per local and a join only takes
        // it away, so it costs one more step each. Whether a free has been
        // sequenced is one bit per site: the transfer sets it and only a join
        // takes it back, which a join can do once, so it is one more step per
        // site and the number below is not changed for it. That is slack being
        // spent rather than a bound being re-derived, and the paragraph below
        // is why that is acceptable here. Where a local points in its sites
        // moves from `Zero` or `NonZero` to `Unknown` at a join and never back,
        // once per local, and the per-local term below counts that step.
        // Whether a site is exposed is one more bit per site that a join only
        // sets, so one more step each; what each site may contain is a third
        // square table that only grows at a join, one step per pair, and the
        // first term gains a third square for it. See ADR-0039. Whether a
        // local may hold a pointer read out of memory is one more bit per
        // local that a join only sets, one more step each. See ADR-0040.
        // Whether a call this check cannot read may have written into a local
        // is one more bit per local that a join only sets, one more step each.
        // See ADR-0044. Whether a site may contain an allocation that is gone
        // is one more bit per site that a join only sets, one more step each,
        // and whether a local was read out of such a site one more per local.
        // See ADR-0045. Whether a local was read out of what a parameter
        // points at is one more per local, and whether a site may hold such a
        // pointer one more per site, which a join only sets. See ADR-0040.
        // What a `realloc` remembered on a site is one more per site: set by
        // the call, and a join only takes it away; and which site a local is
        // exactly the result of is one more per local, the same. See ADR-0039.
        // Which locals' addresses each site may hold is a fourth square table
        // that only grows, one step per pair, and the first term gains a
        // fourth square for it. See ADR-0045. Whether code this check cannot
        // read may hold a local's address is one more bit per local that the
        // transfer only sets and a join only clears, so an entry value moves
        // it once, and the per-local term gains it. Whether a site may hold a
        // slot such a call could not replace is one more bit per site, and
        // whether a local was read out of one one more per local, which a
        // join only sets. See ADR-0047. Whether a lost value may have been
        // stored in a site is one more bit per site that a join only sets
        // (#503).
        //
        // **The bit is not monotone in the transfer, and does not have to be.**
        // `Held::clear` puts it back at every fresh assignment. What this
        // number bounds is how often a *block's entry value* can move, and a
        // transfer that takes a fact away inside a block takes it away from
        // every input alike, so it does not make the entry value descend; the
        // paragraph after next is the field that does. Generous
        // rather than tight, which is the direction `Analysis::height` says to
        // err in: answering too low stops a correct analysis.
        //
        // **An entry value inside a loop can move down as well, since
        // ADR-0016's solver works a block that is not a loop's head out again
        // from what its predecessors send now.** The transfer is not monotone
        // in every field: `Held::freed` is a proof a set of two sites carries
        // and a set of one does not, and a `realloc` fact is recorded only
        // while every old site is `Live`. A head keeps what it had and joins
        // the rest in, so its value only climbs, and every cycle passes
        // through one, which is what bounds the walk; the blocks between two
        // heads are answered from them. The five steps per site counted below
        // were for the one join that did not union, ADR-0048's replacement of
        // a state where one side did not hold the site; that is
        // `SiteState::Unheld` joining as the identity now, one step up from it
        // per site, and the rest is kept as slack.
        //
        // **What has been read since the last sequence point is a set of
        // positions, so it is counted in positions rather than in locals.** A
        // block's entry value can gain one entry per place read at one element
        // or terminator of the function, and each entry's set of sites can gain
        // one site per local, and its set of sites reachable to code this check
        // cannot read one more, which is ADR-0042's and was left uncounted
        // until ADR-0044 recounted this, and the calls pending with them one
        // step per position more. The same paragraph above applies to
        // it: the transfer empties it at every marker and only a join makes an
        // entry value grow.
        //
        // **What a site takes, `Known::extent`, only descends**, and every
        // value it can take is the size an allocation was asked for or a
        // type's size, one per site made or parameter, so at most one step
        // per such value below where it started, plus one from nothing.
        // Counted per site as one per position and one per local, which is
        // more than there are values, and one more for the step from
        // nothing: `locals * (positions + locals + 1)`. Whether a local may
        // hold the address of something inside another is one more bit per
        // local that a join only sets, the last `locals`. What a local's
        // value is believed to point at, `Held::believed`, is a proof a join
        // only lowers or drops, and the slack above covers it. See ADR-0060.
        //
        // **What a local's value believes of its memory, what was stored in
        // a site, and what a call it was handed to may have written there,
        // one [`Levels`] each, are a list of [`Self::deepest_pointer`] entries
        // apiece**, and a join only lowers an entry: from no bound to a
        // width, and from a width to a smaller one. Every width is a type's
        // size or `0`, so at most one per local and one more, and the list
        // may also grow to its full length once, which changes no bound but
        // is a step to `Eq`: `locals + 2` steps an entry, three lists a local
        // (#636).
        // Found by review, which measured a loop head visited 255 times at
        // 256 `*`s where the bound had not counted these (#634).
        let positions: usize = function
            .blocks()
            .map(|block| block.elements.len() + 1)
            .sum();

        // **Nothing holds this number.** Measured: answering `locals` instead
        // leaves the whole suite passing, because no function here takes more
        // visits to a block than it has locals, and building one that did
        // would be building a program for the bound rather than for the check.
        // What a wrong answer costs is what that method promises: too low is a
        // panic naming `Analysis::height`, which is a build that stops with
        // something to read rather than a wrong answer about a program.
        locals * locals * 4
            + locals * (locals + 26)
            + positions * (2 * locals + 3)
            + locals
            + locals * (positions + locals + 2)
            + 3 * locals * self.deepest_pointer * (locals + 2)
    }

    fn on_entry(&self) -> Self::Value {
        let mut known = Known {
            points_to: Table::filled(Held::none(self.locals), self.locals),
            // Nothing has allocated into any of these yet, so none of them can
            // say where it came from. A parameter stays this way: its
            // allocation happened somewhere this check cannot see.
            state: Table::filled(SiteState::Live(None), self.locals),
            // Nothing holds a local's address where a function starts, a
            // parameter included: what a caller holds is its own local.
            escaped: Table::filled(false, self.locals),
            handed_away: Table::filled(false, self.locals),
            exposed: Table::filled(false, Loc::count(self.locals)),
            contents: Table::filled(Cell::none(self.locals), self.locals),
            realloced: Table::filled(None, self.locals),
            // Nothing has been read yet, so there is nothing a free could be
            // unordered against.
            pending: BTreeMap::new(),
            calls: BTreeMap::new(),
            exposed_after_call: BTreeSet::new(),
            lost_exposed: false,
            // Nothing is made yet but the parameters, below, and every local
            // is the size its type is. See `Known::extent`.
            extent: {
                let function = self.unit.function(self.function);
                let mut extent = Table::filled(None, Loc::count(self.locals));
                for local in function.locals() {
                    extent[local] = self
                        .unit
                        .size_of(function.local(local))
                        .map(Extent::Exactly);
                }
                extent
            },
            // Nothing stored and nothing handed to a call yet.
            handed_at_most: Table::filled(None, self.locals),
            handed_contents_at_most: Table::filled(Levels::default(), self.locals),
            // Every local that may hold a pointer, but the parameters, which
            // the caller wrote. See `Known::init`.
            // Every place written but the locals `unwritten_at_entry` names,
            // every local but a parameter: a parameter's memory is the
            // caller's, which may have written it, and an allocation is made
            // fresh where it is made.
            init: {
                let unwritten = self.unwritten_at_entry();
                let mut init = Table::filled(Init::Written, Loc::count(self.locals));
                for local in init.locals() {
                    if unwritten[local.index()] {
                        init[local] = Init::Unwritten;
                    }
                }
                init
            },
            element_init: Table::from_vec(self.elements_at_entry()),
            exposed_in_part: Table::filled(false, self.locals),
            stored_in_part: Table::filled(false, self.locals),
            // No call has been handed anything yet.
            // Nothing has been stored anywhere yet.
            stored_out: Table::filled(false, self.locals),
            // No call has been handed anything yet.
            handed_untyped: Table::filled(Vec::new(), Loc::count(self.locals)),
        };

        // A parameter holds whatever the caller passed, which is a thing this
        // function can free and did not make. The module comment says why one
        // has to be a site of its own.
        //
        // At its start, because the site stands for whatever the caller passed
        // and not for an allocation behind it. ADR-0036 says why a free of
        // `p + 1` is still proved on that reading.
        //
        // **And believed to be one whole object of the type it points to**,
        // and no more: the caller is where it is asked. A `void *` is believed
        // nothing. See ADR-0060.
        let function = self.unit.function(self.function);
        for &parameter in &self.parameters {
            known.points_to[parameter].hold(Site::of(LocalIdx::of(parameter)), Offset::Zero);
            // And what its memory holds is believed as its type says, which
            // the caller asked where it stored it. See `Held::contents_at_most`.
            known.points_to[parameter].contents_at_most = self
                .type_levels(function.local(parameter))
                .unwrap_or_default();
            known.extent[Site::of(LocalIdx::of(parameter))] = Some(Extent::AtLeast(
                self.pointee_size(function, &Place::local(parameter))
                    .unwrap_or(0),
            ));
        }

        // **And exposed**, so that `release_all(); return *p;` is unproven:
        // the caller may have stashed `p` where `release_all` frees it. What
        // this costs is that a pointer parameter read after any call this
        // check cannot read is unproven too. See ADR-0040.
        known.expose(
            self.sources,
            self.exposed_parameters
                .iter()
                .map(|&local| Site::of(LocalIdx::of(local))),
            None,
        );

        // **And what it points at may hold a pointer the caller kept**, so a
        // free of the caller's pointer or a call this check cannot read makes
        // it stale, and a load out of it afterwards is one this check stopped
        // following. Unmarked, `q = *pp; free(q); q = *pp; return *q;` read
        // `*pp` again as a fresh value and was believed (#482). See ADR-0040.
        for parameter in &self.exposed_parameters {
            known.contents[Site::of(LocalIdx::of(parameter))].mark_from_caller();
        }

        known
    }

    fn join(&self, into: &mut Self::Value, from: &Self::Value) {
        self.joined(into, from, false);
    }

    fn join_back(&self, head: BlockId, into: &mut Self::Value, from: &Self::Value) {
        self.joined(into, from, self.runs_once[head.index()]);
    }

    /// A back edge folded in, then every place written on the paths into the
    /// loop this turn and unwritten along the back edge made `Maybe`, as
    /// [`Init::unwritten_again`] says. **Not a site the paths into the loop
    /// did not name**, `SiteState::Unheld`, which is the join's identity
    /// (ADR-0048): a block made in last turn's body is unwritten, and was
    /// nothing before the loop. **A scalar local and an element decide no
    /// case yet**: nothing makes one unwritten again but `StorageLive`, which
    /// makes a new object, while a copy between a block and a local carries
    /// no mark (#750). They are asked so that the rule does not split by
    /// kind (#760).
    fn join_back_beside(
        &self,
        head: BlockId,
        into: &mut Self::Value,
        from: &Self::Value,
        before: &Self::Value,
    ) {
        self.joined(into, from, self.runs_once[head.index()]);
        for place in into.init.indices() {
            let again = into.init[place].unwritten_again(before.init[place], from.init[place]);
            match place {
                Loc::Heap(site) => {
                    if !matches!(before.state[site], SiteState::Unheld) {
                        into.init[place] = again;
                    }
                }
                Loc::Local(_) => into.init[place] = again,
            }
        }
        for (local, elements) in into.element_init.enumerated_mut() {
            elements.unwritten_again(&before.element_init[local], &from.element_init[local]);
        }
    }

    fn element(&self, function: &Function, element: &Element, value: &mut Self::Value) {
        // What is read here is read where this element runs, against what held
        // before it, which is the same question `findings` asks one line
        // earlier and has to get the same answer to.
        //
        // **Before the arms, so that no arm's early return can skip it, and
        // nothing observes that today.** Measured: moving it below the match
        // changes no answer, because the one arm that returns early is the
        // write through a pointer, and this frontend reads an assignment's
        // value back into a temporary, so the read is recorded by that element
        // instead. That is a property of one lowering rather than of the IR,
        // and such a return has already skipped the rules below it here once,
        // turning a suspicion into a proof. Written first because the order is
        // free and the alternative is guarded by nothing.
        value.met(dereferenced_in_element(element));

        // Every field written out, never `..`, which would let a field added
        // to a variant that already exists walk past an exhaustive match.
        match element {
            // Evaluating a place writes nowhere, so no local changes what it
            // holds and no site changes what is known about it. What it reads
            // is not nothing, and `Known::met` above has already taken it: this
            // element exists so that `*p;` on its own can be seen at all.
            Element::Evaluate {
                place: _,
                origin: _,
            } => {}
            // **Every free reaching here is now ordered before everything
            // that follows.** No value moves, so no local's set changes; what
            // changes is that a free this check was holding open can be acted
            // on. See ADR-0022.
            Element::Sequenced { origin: _ } => {
                for state in &mut value.state {
                    if let SiteState::Freed { freed, .. } = state {
                        freed.sequenced = true;
                    }
                }
                // The same fact about a free of a may-set, which ADR-0020
                // records on the local that named the set rather than on its
                // members. One rule, applied to both places a free is written.
                for held in &mut value.points_to {
                    if let Some(freed) = &mut held.freed {
                        freed.sequenced = true;
                    }
                }
                // **And every read behind this is now ordered before whatever
                // frees ahead of it**, so there is nothing left for a later
                // free to be unordered against. The same element answering both
                // directions is the point: one marker, one meaning, read from
                // each side. See ADR-0023.
                value.pending.clear();
                // And every call, for the same reason. Not at
                // `ArgumentsEvaluated`, which orders reads before one call and
                // a call in its arguments before nothing else. No program
                // measured tells the two apart, because that marker is not
                // emitted under an operator that does not sequence.
                value.calls.clear();
                // **And what was made reachable while a call was pending is
                // unproven from here**, as an opaque call leaves what it may
                // free: C may have run the call after the event, so the call
                // may have freed it, and the forward walk met the call first.
                // `r = release_all() + (memset(a, 0, 4) != 0); r = r + a[0];`
                // read `a` in silence. A proved free stays proved. See
                // ADR-0042.
                // Reachable rather than freed: the call reached it through
                // the exposure. See ADR-0047.
                for &site in &value.exposed_after_call {
                    value.state[site].doubted();
                }
                value.exposed_after_call.clear();
            }
            // **Every read behind this is ordered before the call that
            // follows**, which is the half of the marker above that this one
            // says. C17 6.5.2.2 p10's first sentence orders a call's arguments
            // before the call unconditionally, so `free(p + *p)` is a program C
            // defines and was reported until this element existed.
            //
            // The other half is left alone, and the element's own doc comment
            // is where the reason is: concluding it here proved a use after
            // free about `g((free(p), 0), *p)`, whose two arguments C leaves
            // unsequenced. See ADR-0026.
            Element::ArgumentsEvaluated { origin: _ } => value.pending.clear(),
            // Reported where the walk reaches it, and nothing a pointer holds
            // changes: the element says what the arguments do, and their own
            // elements and the call are what do it. See ADR-0052.
            Element::UnsequencedAccess {
                local: _,
                written: _,
                accessed: _,
                both_write: _,
                between: _,
                origin: _,
            } => {}
            Element::Assign(operation) => {
                // **What a store that leaves carries is stored out**, asked
                // before it lands, as the store door asks it: the sites the
                // stored value holds, and what a pointer read out of memory
                // may be. See `Known::stored_out`. **Three halves decide no
                // case, measured**, and are kept so this asks what the store
                // door does and records what the store records: the
                // `StoreKind::Direct` arm, since a write into a local such code
                // names hands what it
                // writes away and the free is then a double free's doubt
                // first; the pointer test, since an integer that holds sites
                // is a conversion the frontend refuses (`SC0302`); and
                // `read_out`, since a pointer read out of memory reaches a
                // store as an operand whose sites `carried` already holds.
                let store = self.store_in(element, value);
                let marks = store.as_ref().is_some_and(|store| {
                    let kind = match store.kind {
                        StoreKind::Direct => true,
                        StoreKind::Through { derefs } => derefs > 0,
                        // `store_in` makes no copy; listed so that a kind
                        // added is answered here.
                        StoreKind::Copy => false,
                    };
                    kind && store.leaves && self.may_be_pointer(function, &store.into)
                });
                if marks {
                    let mut out: Vec<Site> = self
                        .carried(function, &operation.value, value)
                        .sites()
                        .collect();
                    if let Rvalue::Use(written) = &operation.value {
                        out.extend(self.read_out(function, written, value));
                    }
                    // **And everything those allocations hold, at every
                    // level**, as `freed_behind` asks what leaves: a box stored
                    // out holds what was stored in it before, and `*b = x;
                    // *pp = b; free(x);` left the caller a freed pointer one
                    // level down in silence. Found by review of #650.
                    for site in value.closure(out) {
                        value.stored_out[site] = true;
                    }
                }

                // An assignment to a local writes it, whatever it writes. See
                // `Known::init`.
                if operation.place.projection.is_empty() {
                    value.written_local(LocalIdx::of(operation.place.local));
                }

                // **This check follows a write through exactly one `Deref` and
                // nothing deeper.** The edge recorded at `Rvalue::Address` is
                // one step, and reading it as two would be inventing the
                // second. `**ppp = q` is therefore a write this check cannot
                // follow at all, and so is a write through any other
                // projection.
                let one_step = operation.place.projection.as_slice() == [Projection::Deref];
                // What was unwritten before this store, for a local it may
                // have written past. See `Allocations::written_past_its_end`.
                let unwritten_before = value.unwritten_locals();
                // Where it lands, before anything here writes: the one
                // description of this write the unwritten belief applies and
                // the replacement below reads (#778).
                let deep = derefs(&operation.place);
                let before = self.landing(
                    value,
                    operation.place.local,
                    deep,
                    one_step,
                    if deep == 0 {
                        StoreKind::Direct
                    } else {
                        StoreKind::Through { derefs: deep }
                    },
                );
                let targets = if one_step {
                    before.targets.clone()
                } else {
                    Vec::new()
                };

                // **Whether this write lands in one local and nowhere else.**
                // ADR-0028's condition, read here and at the replacement
                // further down: one is about what the write may have reached
                // *besides* its target and the other about what it does to
                // that target, and the two have to be the same question, so
                // both ask `Landing::strong`.
                //
                // A deeper projection is never certain, and that is the
                // condition there rather than an extra clause: `written_through`
                // answers about the local the place starts at, so for `**ppp`
                // it answers about `*ppp` and names the wrong thing.
                let certain = before.strong(value);

                // **Any write through a projection that is not the certain one
                // may have landed in an escaped local.** Not only the one this
                // arm can follow: a write it cannot follow at all is the case
                // that needs this most, and keying the rule on the shape the
                // arm below reads left `**ppp = q` saying nothing whatever
                // while its own corpus case, spelled with a temporary, was
                // answered. Two review lenses found that independently. See
                // ADR-0031.
                // The locals it may land in, asked before it lands, so that
                // what they record below is about this store. See
                // `Known::named_after_a_store`.
                let landed =
                    (!operation.place.projection.is_empty() && certain.is_none()).then(|| {
                        value.locals_a_write_may_land_in(
                            operation.place.local,
                            derefs(&operation.place),
                        )
                    });
                // What it writes of what is unwritten, after the locals it may
                // land in are asked. See `Allocations::initialised_by`.
                self.initialised_by(
                    function,
                    value,
                    Wrote::Store {
                        place: &operation.place,
                        landing: &before,
                        unwritten_before: &unwritten_before,
                    },
                );

                // **Into the allocations the pointer holds, what the write
                // carries is inside them**, exposed whenever they are, by
                // [`Known::expose`]'s closure. A write this check cannot place
                // exposes what it carries at once: one deeper than one `Deref`
                // whose level above names nothing, or one through a pointer that holds
                // neither an allocation nor a local's address, or, for that
                // part, one through a pointer that may also point into memory
                // this check does not model or hold something it lost. A write that
                // may land in followed locals records nothing here: their
                // addresses escaped, and what they hold is in every call's
                // reach. A store into a local aggregate, once fields and
                // indices are lowered, has no targets and lands in the
                // exposing branch, which is a false report the reader can see
                // until it is recorded inside the local instead. See ADR-0039.
                if !operation.place.projection.is_empty() {
                    let written = self.carried(function, &operation.value, value);
                    // **A pointer that reaches an array only by #639's rule,
                    // stored**, into memory or into a local through its
                    // address, either of which a load reads back without its
                    // element. Poured below. See `Known::stored_in_part`.
                    let in_part: Vec<LocalIdx> = if written.array_edge {
                        written.locals().collect()
                    } else {
                        Vec::new()
                    };
                    let mut carried: Vec<Site> = written.sites().collect();
                    // And what a pointer read out of memory may be, which the
                    // sites above do not name: `*b = *a;` stores in `b` what
                    // `a` held. See ADR-0040.
                    if let Rvalue::Use(written) = &operation.value {
                        carried.extend(self.read_out(function, written, value));
                    }
                    // **A store of more than one dereference lands in what the
                    // level above may be**, `**t3 = r` in what `*t3` may point
                    // at, and is recorded there as a store through one is;
                    // `unnamed` below exposes it too, since that set is a lower
                    // bound. Unplaced, `**t3 = r; free(r); ***t3` read nothing
                    // of it. See ADR-0045.
                    let deep = derefs(&operation.place);
                    let landing = self.landing(
                        value,
                        operation.place.local,
                        deep,
                        one_step,
                        StoreKind::Through { derefs: deep },
                    );
                    // **And in the locals the level above may be**, `**t3 = r`
                    // with `*t3 = &slot` in `slot`, as a write through an
                    // alias that may land in several locals: by union, the
                    // proof dropped. See ADR-0045 and ADR-0019.
                    let deep_targets: Vec<LocalIdx> = if deep > 1 {
                        landing.targets.clone()
                    } else {
                        Vec::new()
                    };
                    // And as through one dereference, where a level above
                    // reached them only by the rule that keeps an edge through
                    // a known move: `**ppp = q;` with `*ppp = &z + 7` proved a
                    // use after free once the one-step store was marked and
                    // this was not. Found by review.
                    let moved_above = deep > 1
                        && value
                            .levels_below(operation.place.local, deep - 2)
                            .1
                            .into_iter()
                            .any(|local| value.points_to[local].array_edge);
                    for &target in &deep_targets {
                        if moved_above {
                            self.written_past_its_end(function, value, target, &unwritten_before);
                        }
                        value.target_union(target, &written);
                    }
                    // Written into locals whose addresses escaped, which every
                    // call reaches, so a pending read is told, as at an
                    // exposure. See ADR-0042.
                    if !targets.is_empty() {
                        let routes = value.closure(carried.clone());
                        value.noticed(self.sources, Some(operation.origin.span()), &routes);
                    }
                    // **A pointer that holds a site may also hold what this
                    // check cannot name**: a pointer read out of memory, which
                    // may be memory the caller owns; a site it lost the name
                    // for; or what a call this check cannot read may have
                    // written into it. For that part the write is unplaced, so
                    // what it carries is exposed as well as recorded in the
                    // sites: `t = c ? s : *tab; *t = a;` stored `a` only in
                    // `s` and was silent after a later call. See ADR-0044.
                    // **Not a local whose address it carries**, though code
                    // this check cannot read may find it there: the slot may
                    // be written again before any such code runs, and an
                    // `unnamed` pointer may land in this function's own
                    // memory on another path, so the address being out there
                    // is a may-fact and would exempt a holder from being
                    // asked. See ADR-0047.
                    //
                    // **What it carries may land in any memory code this check
                    // cannot read can name**, which is every exposed
                    // allocation and every allocation one holds, since a
                    // pointer this check cannot place may be any of them: `*a
                    // = t; w = *b; *w = m; free(m); q = *t; return *q;` with
                    // `a == b` read back nothing at `*t` and was believed
                    // (#487). Reachable rather than exposed, because `*a = t;`
                    // records `t` in `a`'s memory and does not mark it. Only a
                    // pointer stored: `**pp = x;` with `x` an `int` copied out
                    // of a parameter carried that parameter's site, and a
                    // function with no free reported a use of freed memory.
                    // See ADR-0044.
                    //
                    // **And a store into handed-over memory lands the same
                    // way**, since another parameter may name that memory:
                    // `*w = m; free(m); q = *pp; return *q;` with `f(&x, &x)`
                    // read back nothing at `*pp` and was believed (#489).
                    // Reachable to code this check cannot read, which is what
                    // another parameter can name, and not a cell's `from_caller`,
                    // which a buffer of this function's own gains by holding
                    // a caller's pointer. Found by review.
                    // **A pointer read out of memory is bounded by what it
                    // is believed**, whatever the record says it names:
                    // the sites a load holds are what the walk every load
                    // takes found there, at an offset nobody said, and may
                    // be the caller's memory or another allocation rather
                    // than what it points at. `char *c = *cc; *m = c;`
                    // handed `m` on as an `int **` with nothing asked, and
                    // so did the same with `*m = malloc(4);` first, whose
                    // site the load also named (#632). **Only a load** was
                    // bounded here then, which is what `written.loaded` asks:
                    // bounding a pointer this check lost at zero doubted a
                    // lost pointer to whole allocations read back, which that
                    // issue was not about. Since ADR-0061 every value is
                    // bounded, a lost one at zero (`Held::recorded_width`),
                    // and the arms below say how. A load that believes nothing is bounded at
                    // zero, which is what it was asked. `Known::memory_levels`
                    // reads a local's own level from the same two fields, so
                    // that a copy of the local carries what this store
                    // would; it does not ask `may_be_pointer`, so a byte
                    // copied is bounded where its store is not (#711). See
                    // ADR-0060.
                    //
                    // **And a pointer a call this check cannot read wrote, at
                    // what it is believed**, which the opaque call's write
                    // puts in `believed`, no wider than it was handed as: `get(&c); *cm = c;` with `get`
                    // taking `char **`, read back through an `int **`, was
                    // believed four bytes wide where `*pv` of `c`'s address
                    // was doubted (#715). Where it was handed nothing, as its
                    // type, as ADR-0017 believes an output: bounding such a
                    // pointer at zero doubted 34 cases, measured. Said as
                    // `AS_TYPED`, since no bound is now no belief (ADR-0061).
                    //
                    // **And any other value at the width of what it names**,
                    // so that `*box = &q; int *t = *box; *t = 1;` proves what
                    // it proved when a load believed its type (ADR-0061).
                    let bound = if !self.may_be_pointer(function, &operation.place) {
                        None
                    } else if written.loaded {
                        Some(written.believed.unwrap_or(0))
                    } else if written.foreign {
                        // And what the local was handed as, which a join
                        // keeps where `believed` does not. A join with a null
                        // keeps `believed` too (#719).
                        let handed = match &operation.value {
                            Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => {
                                value.handed_at_most[place.local]
                            }
                            Rvalue::Use(_)
                            | Rvalue::Address(_)
                            | Rvalue::Unary { .. }
                            | Rvalue::Binary { .. } => None,
                        };
                        Some(written.believed.or(handed).unwrap_or(AS_TYPED))
                    } else {
                        written.recorded_width(&value.extent)
                    };
                    // **And below it, what the stored pointer believes of
                    // its own memory, whatever it is**: a parameter or a
                    // call's result too, which name a site the record
                    // holds and leave what that site holds to what they
                    // arrived as. `*box = m;` with `m` a `void **`
                    // parameter, read back through an `int ***`, believed
                    // whole `int`s two levels down (#634). A local's
                    // address and an allocation believe nothing here, so
                    // record nothing. See `Cell::stored_levels`.
                    //
                    // Mutation: record level 0 alone, and
                    // `a_parameter_stored_in_memory_and_read_back_is_believed_as_it_arrived_below_its_first_level`
                    // goes silent.
                    let stored = written
                        .contents_at_most
                        .stored_under(bound, self.deepest_pointer);
                    // **What the store carries, poured into each allocation it
                    // lands in**, as `Cell::poured` builds it, which names
                    // every field. **After the landing and the exposure**:
                    // poured
                    // first, a container already reachable makes what it
                    // carries reachable too, and the landing then records it
                    // inside itself: a pointer this check cannot place cannot
                    // point into what it carries when only this store made
                    // that reachable. See `Known::pour`.
                    //
                    // The marks, each for its reason: **a load out of a marked
                    // allocation is one still when it is stored**, so the
                    // allocation it is stored in is marked too: `*t4 = *t2;`,
                    // or the same through a local, was followed as nothing and
                    // read in silence, found by review, and not any lost
                    // value, for the reason `Held::stale_read` gives
                    // (ADR-0045); the slot a call could not replace, for the
                    // same reason (ADR-0047); **a lost value**, which records
                    // nothing in the container, so that a later free of a lost
                    // value can find it (#503, ADR-0045); and a pointer read
                    // out of caller memory keeps that fact where it is stored
                    // (ADR-0040).
                    let poured = Cell::poured(
                        value.points_to.len(),
                        carried.iter().copied(),
                        written.locals(),
                        &stored,
                        &written,
                        // **No width, for a value that is not a pointer by
                        // its type**: a pointer is given one above whatever it
                        // names, and one with no `bound` names nothing, which
                        // is a null. See `Cell::unrecorded`.
                        !self.may_be_pointer(function, &operation.place),
                    );
                    self.land_contents(
                        value,
                        &landing,
                        &poured,
                        &in_part,
                        Pours::Store {
                            pointer_stored: self.may_be_pointer(function, &operation.place),
                            carried,
                            span: operation.origin.span(),
                        },
                    );
                }
                if let Some(landed) = landed {
                    value.named_after_a_store(&landed);
                }

                if one_step {
                    if targets.is_empty() {
                        return;
                    }

                    // **What is written, before what it is written into.**
                    // Whether this lands in one certain local or in any of
                    // several possible ones is decided below, once the value
                    // is in hand; a pointer that may point at one local is not
                    // a pointer that must, and telling those apart is
                    // ADR-0028.
                    //
                    let written = self.carried(function, &operation.value, value);

                    // **A write this check can be certain about replaces what
                    // the target held.** The set names one local and says it
                    // names all of them, so this write landed in that local
                    // and whatever was there is gone. Union would manufacture
                    // a two-element may-set out of a program that has none,
                    // and ADR-0020 then reads that as real ambiguity and gives
                    // up a proof over it: `int **pp = &p; *pp = q; free(p);
                    // *q = 1;` is a certain use after free and was reported as
                    // a suspicion. See ADR-0028, which is where ADR-0019's
                    // rejected option was taken up once the flag above made
                    // "one target" distinguishable from "at most one target".
                    //
                    // The whole row, as an assignment to the target would
                    // replace it: the sites, what it had lost, the proof about
                    // the set it named and the edge alike. `unproved` is the
                    // one thing the direct assignment does that this does not,
                    // for the reason the paragraph below gives, which is about
                    // the allocation rather than about the local.
                    //
                    // **Both halves of "the edge is all of it" are read here.**
                    // The flag above lives in `Held`, so an assignment to the
                    // pointer destroys it, and that is right for the assignment
                    // itself and wrong for an escape: `int ***ppp = &pp; pp =
                    // &p; opaque(ppp); *pp = q;` gave `pp` a fresh row after
                    // something had already taken its address, and the
                    // replacement fired on an edge anybody could have
                    // overwritten since. `Known::escaped` is the half that
                    // outlives an assignment, which is ADR-0018's rule for
                    // which struct a fact belongs in, and it is why the answer
                    // cannot be recorded on the local whose address is taken.
                    // See ADR-0028. An analysis that proves anything positive
                    // about a local has to answer for its address escaping.
                    if let Some(at) = certain {
                        value.replace(at, written);
                        return;
                    }

                    // **The union and nothing else.** A write through an alias
                    // does not unprove what the target held, which #155's rule
                    // for a direct assignment would suggest it should. The
                    // difference is whose fact is at stake: `unproved` writes
                    // on the *sites*, which are shared, so doing it here wiped
                    // a `Freed` that a second local holding the same allocation
                    // had proved. `free(p); *pp = 0; *p = 1;` went from a
                    // proved use after free to a suspicion, and exit 1 to exit
                    // 0, with the write carrying nothing at all.
                    //
                    // Nothing is lost by leaving it out. The target's address
                    // was taken, so ADR-0017 answers `Reached::Lost` for it
                    // wherever a report is made, and `Known::settle` applies
                    // the heap half over the merged value at every join. See
                    // ADR-0019.
                    for target in targets {
                        value.target_union(target, &written);
                    }

                    return;
                }

                // A write through any other projection changes what a pointer
                // points at rather than which allocation a local holds, and
                // this check follows locals.
                let Some(at) = Strong::assigned(&operation.place) else {
                    return;
                };
                let destination = operation.place.local;
                match &operation.value {
                    // **Not an optimisation.** `int *p = malloc(4);` lowers to
                    // a call into a temporary and a copy out of it, so without
                    // this the allocation never leaves the temporary and the
                    // headline program is not an error at all.
                    Rvalue::Use(Operand::Copy(source)) if source.projection.is_empty() => {
                        let indeterminate = value.holds_fresh_read(LocalIdx::of(source.local));
                        value.replace(at, value.points_to[source.local].clone());
                        // See `Known::holds_fresh_read`.
                        value.points_to[destination].fresh_read = indeterminate;
                    }
                    // **Arithmetic on a pointer is followed.** C17 6.5.6 p8
                    // keeps the result inside the object the operand points
                    // into, so `p + 1` may hold whatever `p` holds, and `p[i]`
                    // is that addition: 6.5.2.1 p2 defines `E1[E2]` as
                    // `(*((E1)+(E2)))`. This arm used to clear the destination
                    // instead, on the stated ground that nothing read the
                    // precision yet. Something does now, and until it did the
                    // cost was invisible: `free(p); p[i] = 42;` was silence
                    // rather than a diagnostic, which is the worst answer this
                    // compiler has.
                    //
                    // **The pointer operand, and the integer beside it is not
                    // one.** Which is which is a question about types and
                    // [`built_from`] reads them: 6.5.6 p8 keeps the result
                    // inside the object the *pointer* points into, so a site
                    // the index happens to be is not a site the subscript may
                    // reach. A parameter is a site, so until this was read
                    // `p[i]` unioned the allocation `p` holds with the site `i`
                    // is, and a live site stops the result being proved:
                    // `free(p); p[i] = 42;` was a warning where
                    // `free(p); p[0] = 42;` was an error. See ADR-0030.
                    //
                    // Read before the write, so `p = p + 1` keeps what `p`
                    // held rather than clearing it and unioning the result.
                    Rvalue::Binary { op, lhs, rhs } => {
                        let mut reached = built_from(
                            *op,
                            [lhs, rhs],
                            value,
                            |local| self.is_pointer(function, local),
                            |place| self.may_be_pointer(function, place),
                            |place| self.reads_a_byte(function, place),
                            |local, depth| self.reads_caller_memory(local, depth, value),
                            |local| self.is_array(function, local),
                        );
                        // **The sites travel, and the edge only where the offset
                        // may be zero.** C17 6.5.6 p8 keeps the result inside
                        // the object the operand points into, which is why the
                        // allocation comes along; what the edge does is
                        // [`Held::moved_by_arithmetic`]'s. A literal zero the
                        // lowering did not fold is an offset that may be zero,
                        // and is followed. See ADR-0019 and ADR-0021.
                        reached.moved_by_arithmetic(self.moved_by(function, *op, lhs, rhs));
                        value.replace(at, reached);
                    }
                    // A read through a projection holds what memory records
                    // there, marked as a load, which the report reads beside
                    // `Reached::Partial` and proves nothing with; what a call
                    // reaches and what a write stores know it too. See
                    // `Allocations::read_through` and ADR-0067.
                    Rvalue::Use(Operand::Copy(source)) => {
                        let read = self.read_through(function, source, value);
                        value.replace(at, read);
                    }
                    // A constant or a unary operator. Neither is a pointer this
                    // check can follow: C17 6.5.3.3 gives unary `+`, `-` and
                    // `~` arithmetic operands only, and `!` yields an `int`.
                    Rvalue::Use(Operand::Constant(_)) | Rvalue::Unary { .. } => {
                        value.replace(at, Held::none(value.points_to.len()));
                    }
                    // The address of a place is not an allocation this check
                    // follows: nothing here frees a local's own storage, which
                    // is `StorageDead` and is the lifetime phase's.
                    //
                    // **But the place whose address is taken stops being
                    // something anything here proved.** A write through the
                    // pointer that just escaped can put a different allocation
                    // in it, and this check does not follow what a pointer
                    // points at, so it would not see the write. `Rvalue::Address`
                    // is the only way a local's address is taken in this IR, so
                    // this is the one door, and leaving it open is what made
                    // `int **pp = &p; *pp = q; free(q); free(p);` silent about
                    // a double free.
                    Rvalue::Address(taken) => {
                        value.replace(at, Held::none(value.points_to.len()));
                        // **The edge and the bit, and both are needed.** The
                        // bit outlives everything done to this destination and
                        // is what keeps an escaped local unproved; the edge
                        // dies with the destination and is what lets a write
                        // through it be followed. See ADR-0019.
                        value.points_to[destination].name_local(LocalIdx::of(taken.local));
                        // **And the set is all of it, where the address is of
                        // the local itself.** `Held::clear` ran a line above,
                        // so this destination points at exactly this local,
                        // which is what lets a write through it replace rather
                        // than union. See ADR-0028.
                        //
                        // `&*pp` is not that. C17 6.5.3.2 p3 makes it `pp`, so
                        // the place it names is what `pp` points at and not
                        // `pp`, while the edge above can only name a local. The
                        // union is the right answer to an edge that names the
                        // wrong thing and a replacement is not, so the flag
                        // stays set and the write stays a may-write. No C
                        // reaches this: the lowering applies the same clause and
                        // folds `&*pp` to a copy of `pp`. Another frontend need
                        // not, which is `docs/c-family.md`'s reason for the IR
                        // expressing the shape at all.
                        //
                        // **An element of a local array is the shape C does
                        // reach**: the lowering converts `a` to `&a[0]`, an
                        // index under this `Address`, and a store through it is a
                        // may-write for the same answer, since one element
                        // written does not replace what the rest of the array
                        // holds (#621).
                        value.points_to[destination].writes_elsewhere =
                            !taken.projection.is_empty();
                        // **The element it points at**, where what was taken
                        // is an array's start: the lowering builds an index
                        // only as a constant zero, and every other element
                        // is reached by arithmetic. See `Held::element`.
                        if self.is_array(function, LocalIdx::of(taken.local))
                            && taken
                                .projection
                                .iter()
                                .all(|step| matches!(step, Projection::Index(Operand::Constant(0))))
                        {
                            value.points_to[destination].element = Some(0);
                        }
                        // And an access through it is not an access of the
                        // local the edge names, unless what was taken is that
                        // local's own start. See `Held::inner_address`.
                        value.points_to[destination].inner_address = !taken
                            .projection
                            .iter()
                            .all(|step| matches!(step, Projection::Index(Operand::Constant(0))));
                        value.escaped[taken.local] = true;
                        value.unproved(LocalIdx::of(taken.local));
                        // What the local holds is reachable to every call from
                        // here, as an exposure is. See ADR-0042.
                        let routes = value.closure(value.sites_of(taken.local).collect());
                        value.noticed(self.sources, Some(operation.origin.span()), &routes);
                    }
                }

                // **After the match rather than inside it**, so that every arm
                // is covered and the next one is covered before it is written.
                // Placed per arm, this was two calls and a hole: `p = r + i;`
                // lowers to arithmetic into a temporary and a copy out of it,
                // so the arm that looks like the arithmetic case is reached
                // through the copy, and a mutation of the arithmetic arm broke
                // nothing at all. One call cannot be put in the wrong place.
                value.unproved(LocalIdx::of(destination));
                // What an escaped local is given, every call reaches through
                // `Known::reach_of`, as an exposure does. Here and not in
                // `Known::unproved`, which `settle` also runs at a join, where
                // one arm's read and the other arm's call would be paired.
                // See ADR-0042.
                if value.escaped[destination] {
                    let routes = value.closure(value.sites_of(destination).collect());
                    value.noticed(self.sources, Some(operation.origin.span()), &routes);
                }
            }
            // Storage beginning or ending says nothing about what the local
            // held before, and what it holds now is nothing.
            Element::StorageLive { local, origin: _ } => {
                value.clear(*local);
                // **A new object, indeterminate again** (C17 6.2.4 p6), so an
                // array written late in one turn of a loop is unwritten at
                // the start of the next. See `Known::init`. **Whatever its
                // type**, as at entry (ADR-0073 rule 2).
                value.init[local] = Init::Unwritten;
                // And every element of it, with nothing written in part and
                // nothing handed over. See `Known::element_init`.
                value.element_init[local].unwrite_all();
                value.exposed_in_part[local] = false;
                value.stored_in_part[local] = false;
                // **And no call holds its address yet**: one kept from an
                // earlier lifetime became indeterminate when that one ended
                // (C17 6.2.4 p2), so a call is no longer a write of it. Kept
                // across, `stash(m)` in one turn of a loop made `tick()` in
                // the next a write of a new `m`, and a read nothing wrote was
                // silent. See `Known::exposed` of a local.
                value.exposed[local] = false;
                // Nor was it handed to one untyped, for the same reason.
                // See `Known::handed_untyped`. No case tells this from
                // keeping the mark, measured: what the mark reaches is
                // written only where it is exposed, and that is cleared just
                // above.
                value.handed_untyped[local].clear();
                // **Nor does any pointer recorded as able to name it**, for
                // the same reason: a pointer a call wrote in an earlier turn
                // named the old object, so a write through it is no write of
                // the new one (#661). See `Held::named_at_read`.
                for held in value.points_to.iter_mut() {
                    held.named_at_read.remove(LocalIdx::of(local));
                }
                // **Nor does a pointer that held its address**: the address
                // is indeterminate now, so a pointer copied in an earlier
                // turn is no write of the new object, `*b = m;
                // memcpy(&t, src, 8);` across turns (#664). The edge
                // is removed and nothing more. Marking the pointer `lost` as
                // well was tried and rejected: a write through a lost pointer
                // lands in every local such code names, which turned a
                // certain `SC0411` `main` gave into doubts, and it doubted
                // `arr[0] = &x; *arr[0]` every turn. A read through such a
                // pointer is undefined and is still silent, #665. An
                // allocation's record of the address is left: removing it
                // decides no case, measured, once the pointer is not marked.
                // See ADR-0057.
                for held in value.points_to.iter_mut() {
                    held.unname_local(LocalIdx::of(local));
                }
            }
            Element::StorageDead { origin: _, local } => value.clear(*local),
        }
    }

    fn terminator(&self, function: &Function, terminator: &Terminator, value: &mut Self::Value) {
        // Before the `let ... else` below, which returns for every terminator
        // that is not a call. A `Terminator::Branch` reads its condition and a
        // condition that is exactly a place never becomes an element, so
        // leaving it to the arm that handles calls would be silent about
        // `(*p ? 1 : 0) + (free(p), 0)`.
        value.met(dereferenced_in_terminator(terminator));

        let Terminator::Call {
            callee,
            arguments,
            destination,
            then,
            origin,
        } = terminator
        else {
            // Nothing else moves an allocation. Written out rather than `_`,
            // so that a terminator added later has to be answered for here.
            match terminator {
                Terminator::Goto(_)
                | Terminator::Branch { .. }
                | Terminator::Return
                | Terminator::Abnormal { .. } => return,
                Terminator::Call { .. } => unreachable!("the let above took it"),
            }
        };

        // **What the call is handed is carried forwards, as a dereference is
        // by the line above.** The callee's body reads it, and C17 6.5.2.2
        // p10 leaves that body indeterminately sequenced with a later call in
        // the same full expression, unless that call encloses this one, whose
        // arguments it orders before itself (ADR-0043, in `used_before`).
        // Before the transfer below, for `PendingRead`'s reason:
        // the sites are the ones held where the call is reached, which
        // `what_a_call_was_handed_is_carried_as_it_was_before_the_call` holds
        // where the call writes into the local it was handed. The
        // arguments are exactly the ones `handed` asks, from one function, so
        // the two readers cannot disagree about which. Dropping this loop
        // silences `a_pointer_handed_to_a_call_the_check_meets_first_is_reported`.
        // See ADR-0042.
        for place in handed_places(self, function, *callee, arguments) {
            let reached = value.handed_reached(place);
            value.meeting(origin.span(), place, Read::Argument, &reached);
        }

        // What the call does to what it was handed, before what it leaves
        // behind, which is the order the two happen in.
        // A `Reached::Lost` moves nothing, because there is nothing to move:
        // what it says is that this call touched something the check was not
        // following, which is a fact about the report rather than about the
        // lattice.
        let kind = self.callee(*callee);
        // `realloc` is asked about its first argument only, as `reported` asks
        // it. No C program shows the difference, since its size holds no
        // allocation; it is written the same way in both places so that the
        // two readings of one rule cannot drift.
        let handed = match kind {
            // **Touches nothing**: what its arguments read was asked above,
            // and nothing after it runs. See `Callee::Ends`.
            Callee::Ends => return,
            Callee::Reallocates => &arguments[..arguments.len().min(1)],
            Callee::Frees
            | Callee::Allocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Opaque => &arguments[..],
        };
        let touched = Self::touching(handed.iter(), value);
        let sites = || named(&touched);

        // **What a later call may have done to an old allocation is not what
        // the `realloc` did**, so a fact naming one this call touches is
        // forgotten, before anything this call records: a free of `p`, a
        // second `realloc` of it, or `g(p)` between the call and the branch
        // left `p` unproven, and the null arm brought it back live. Found by
        // review. See ADR-0039.
        let touched_now: Vec<Site> = sites().collect();
        value.forget_reallocs_touching(|site| touched_now.contains(&site));

        // **An allocation this may free stays fresh.** Clearing it here was
        // tried, to spare a read out of freed memory a second report, and
        // `fresh` is an intersection that nothing sets again: a `realloc`
        // that fails leaves the old block live and unwritten (C17 7.22.3.5
        // p4), and a free of one of two allocations leaves the other, so a
        // read on either path built in silence or as a doubt. Mutation: clear
        // it for a `realloc`, and
        // `a_pointer_read_out_of_the_old_block_after_a_failed_realloc_is_reported`
        // builds. See ADR-0057.

        // **What a `realloc` with a non-zero constant size was handed, while
        // it is all live**, before the transfer below makes it unproven: the
        // branch on the result is what says which it became. **Not for a
        // pointer a call this check cannot read wrote**, which may hold what
        // it does not name, as at a free (#497): `r` is `u` on one arm and `q`
        // on the other, and a fact naming `q` alone proved `free(q)` a double
        // free on the arm where `realloc` was handed `u` (#499). See ADR-0039.
        let realloced: Option<Vec<(Site, Option<Span>)>> = match (kind, &arguments[..]) {
            (Callee::Reallocates, [Operand::Copy(old), Operand::Constant(size), ..])
                if *size != 0 && old.projection.is_empty() =>
            {
                let held = &value.points_to[old.local];
                let old: Vec<(Site, Option<Span>)> = held
                    .sites()
                    .filter_map(|site| match value.state[site] {
                        SiteState::Live(made) => Some((site, made)),
                        SiteState::Freed { .. }
                        | SiteState::Unknown
                        | SiteState::Reachable
                        | SiteState::Unheld => None,
                    })
                    .collect();
                (!held.lost
                    && !held.foreign
                    && !old.is_empty()
                    && old.len() == held.sites().count())
                .then_some(old)
            }
            (Callee::Reallocates, _)
            | (
                Callee::Frees
                | Callee::Allocates
                | Callee::ReturnsFirst
                | Callee::Copies
                | Callee::Ends
                | Callee::Opaque,
                _,
            ) => None,
        };

        // **A free of what may be the caller's may free anything the caller
        // can see**, as a call this check cannot read may: `f(p, p)` hands
        // `free(b); return *a;` one allocation twice. **Before the `match`,
        // because the free's own arm returns early** for a pointer read out
        // of memory, which is what a pointer read out of caller memory is:
        // after it, `q = *pp; free(q); return **pp;` went silent. Found by
        // review. See ADR-0040.
        // **A free or a `realloc` of a value this check lost may release any
        // lost value**, so every one is marked as one that may be gone, before
        // the `match` for the reason the caller's rule below gives. A lost
        // local, a load, or a place read through memory, each of which may
        // hold one: `t = *box; free(t);` frees what `free(*box)` frees, and
        // was read in silence after, found by review. What is marked is still
        // only what is lost, not the load: a load may be null, and marking it
        // doubted a read behind `if (r)` of a pointer `free(NULL)` left alone,
        // found by review of #501. See `Known::lost_may_be_gone`.
        // **And a call this check cannot read may release one**, since what
        // it is handed it may free: a lost local or one read out of memory a
        // lost value may be in, or memory that reaches an allocation holding
        // one. `release(u)`, and `*holder = u; release(holder);`, both freed
        // `u` and left `r = *box; v = *r;` read in silence (#505). The same
        // memory the `Callee::Opaque` arm treats as possibly freed, and
        // nothing a call is not handed: `release_all()` is not a trigger,
        // unless an escaped local may hold a lost value, below.
        //
        // **Spelled any way it can be handed.** A load handed as an operand,
        // `release(*box)`, names nothing and is read out of `box`, which is
        // marked rather than anything in it; and a local's address,
        // `release_pp(&u)`, hands what the local holds, so the locals it
        // points at are asked as the argument is, and their allocations are
        // in what it reaches. Each read `*r` in silence. Found by review.
        //
        // **And what an escaped local may hold**, as `Known::reach_of` has a
        // named value's call reach it: the call may have been handed the
        // local's address now, earlier or by another route. `stash_pp(&w);
        // w = u; r = fetch(); free(u);` read `*r` in silence where a named
        // `u` was reported (#515). **And what it points at**, whose closure
        // `reach_of` adds too: `*box = u; s = *box; stash_ppp(&w); w = box;
        // release_all();` freed what `s` holds, read after in silence.
        // Found by review.
        //
        // Computed once, for the trigger here and for the call's result
        // below, so that the two cannot disagree about what it was handed.
        let opaque = match kind {
            Callee::Opaque => true,
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Ends => false,
        };
        let hands_a_lost_value = opaque && {
            let mut handed_memory: Vec<Site> = sites().collect();
            for local in value
                .escaped
                .indices()
                .filter(|&local| value.escaped[local])
            {
                handed_memory.extend(value.points_to[local].sites());
            }
            for argument in handed {
                handed_memory.extend(self.read_out(function, argument, value));
                if let Operand::Copy(place) = argument {
                    let pointed: Vec<LocalIdx> = value.written_through(place.local);
                    for target in pointed {
                        handed_memory.extend(value.points_to[target].sites());
                    }
                }
            }
            let may_hold_lost = |held: &Held| held.lost || held.lost_read;
            handed.iter().any(|argument| match argument {
                Operand::Copy(place) => {
                    may_hold_lost(&value.points_to[place.local])
                        || (!place.projection.is_empty()
                            && value.marked_below(place.local, derefs(place), |contents| {
                                contents.lost()
                            }))
                        || value
                            .written_through(place.local)
                            .into_iter()
                            .any(|target| may_hold_lost(&value.points_to[target]))
                }
                Operand::Constant(_) => false,
            }) || value
                .closure(handed_memory)
                .into_iter()
                .any(|site| value.contents[site].lost())
                || value
                    .escaped
                    .indices()
                    .any(|local| value.escaped[local] && may_hold_lost(&value.points_to[local]))
        };
        if hands_a_lost_value {
            value.lost_may_be_gone();
            value.lost_exposed = true;
        }

        let may_free = match kind {
            Callee::Frees | Callee::Reallocates => true,
            Callee::Allocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Ends
            | Callee::Opaque => false,
        };
        if may_free
            && handed.iter().any(|argument| match argument {
                Operand::Copy(place) => {
                    let held = &value.points_to[place.local];
                    !place.projection.is_empty() || held.beyond().unrecorded_address()
                }
                Operand::Constant(_) => false,
            })
        {
            value.lost_may_be_gone();
        }

        if may_free && self.frees_callers(handed, value) {
            value.callers_memory_may_be_freed();
        }

        match kind {
            Callee::Ends => {
                unreachable!("`Allocations::terminator` returns for it where `handed` is decided")
            }
            Callee::Frees => {
                let reached: Vec<Site> = sites().collect();

                // **A free of a local that may hold something this check
                // cannot name does not say which allocation went.** The sites
                // below are what the local is *thought* to hold, and a call
                // this check cannot read may have written a fresh pointer over
                // it since; writing `Freed` on them would be a proof about an
                // allocation the callee may have swapped out. The site is
                // shared, so that proof is handed to every other local holding
                // it, which is how a program C defines became an `error` no
                // flag suppresses. See ADR-0029.
                //
                // **It supersedes both rules below and skips nothing else.**
                // Neither of them applies once the set is not known to be what
                // was freed: the one is about which member of a set went, and
                // the other about a single member going. An early return can
                // leave the conservative rules behind it unrun, and the rule
                // here is the conservative one.
                //
                // **What it does skip is the destination.** The fall-through
                // path clears the local `free` is written into and this does
                // not, which the `reached.len() > 1` branch below does too.
                // The C frontend gives `free` none, and one another frontend
                // gives it is `void` and holds none of this; measured, putting
                // the clear back inside this branch leaves the whole workspace
                // green.
                if Self::holds_something_unnameable(arguments, value) {
                    for site in reached {
                        // **A free cannot un-free an allocation.** A site an
                        // earlier free this check *could* follow has already
                        // proved is not something this call has anything to
                        // say about, and the site is shared, so blanking it
                        // takes the proof away from every other local holding
                        // it. Found by review;
                        // `a_free_this_check_could_not_follow_leaves_a_proved_free_alone`
                        // is the program and is the guard.
                        if !matches!(value.state[site], SiteState::Freed { .. }) {
                            value.state[site] = SiteState::Unknown;
                        }
                    }

                    return;
                }

                // **A may-set is not a must-set, and this is where the two used
                // to be confused.** Freeing a local that may hold either of two
                // allocations frees exactly one of them; writing `Freed` on
                // both claimed each was certainly freed, and a later free of
                // one of them by name was then a proved double free about a
                // program that frees each exactly once on one of its paths.
                //
                // So nothing is written on the members. They stop being
                // provable, and the fact that one of them went is recorded on
                // the local that named the set, which is the only thing in this
                // lattice that names a set. See ADR-0020.
                if reached.len() > 1 {
                    for site in &reached {
                        value.state[*site] = SiteState::Unknown;
                    }

                    for argument in arguments {
                        // A constant frees nothing and a projection names a
                        // place this check does not follow, which is what
                        // `Allocations::touching` already said about both.
                        //
                        // **No program reaches the second of those, and it is
                        // here anyway.** `free` takes one argument, and an
                        // argument with a projection reaches no site at all,
                        // so this branch is not entered with one. What it
                        // would cost if that changed is a proof recorded
                        // against the pointer rather than against what it
                        // points at, which is the false proof this whole rule
                        // is against.
                        //
                        // Measured, and the narrow claim is the true one:
                        // *removing* the guard breaks nothing, because the
                        // projection here is always empty. Negating it breaks
                        // two named tests, because then nothing is recorded at
                        // all.
                        let Operand::Copy(place) = argument else {
                            continue;
                        };
                        if place.projection.is_empty() {
                            value.points_to[place.local].freed = Some(Freeing::new(origin.span()));
                        }
                    }

                    return;
                }

                for site in reached {
                    // Whatever the site was known to have come from survives
                    // the free: the diagnostic wants to name it.
                    let made = match value.state[site] {
                        SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                        SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
                    };
                    // **A free already ordered before here is the one to
                    // keep.** It is what proves anything about what follows,
                    // and it is what the diagnostic has to point at for the
                    // proof to be readable: `free(p); x = (free(p), 0) + *p;`
                    // is a proved use after free because of the first line,
                    // and replacing it with the second would name a free that
                    // is in the same unsequenced expression as the use and
                    // leave the reader with two carets that prove nothing.
                    // Without this the second free took the proof away with
                    // it, which review measured. See ADR-0022.
                    //
                    // It is also the earliest, which is what this variant's
                    // own doc comment has always said it holds.
                    let freed = match value.state[site] {
                        SiteState::Freed { freed, .. } if freed.sequenced => freed,
                        _ => Freeing::new(origin.span()),
                    };
                    value.state[site] = SiteState::Freed { made, freed };
                }
            }
            // It does not free what it is passed, which is the whole of why the
            // name is read.
            Callee::Allocates => {}
            // May have freed it, and has not if it failed. See ADR-0039.
            Callee::Reallocates => {
                for site in sites().collect::<Vec<_>>() {
                    value.state[site].may_be_freed();
                }
            }
            // Frees nothing; what it is handed is out of this check's sight
            // from now on, and a local whose address it is handed may have
            // been written through it, as ADR-0029 says of an opaque call.
            Callee::ReturnsFirst | Callee::Copies => {
                // **A copy of an object carries what it contains**: what each
                // allocation the destination holds may contain gains what the
                // source does, which `levels_below` answers for an allocation
                // and, through its edge, for a local's address; and where the source
                // may hold one this check stopped following, so may the
                // destination. Before what follows, which is the whole family's.
                // See ADR-0039 and ADR-0045.
                //
                // **Either argument may be a place of dereferences**, `memcpy(*pp,
                // *ps, 8)`, read as a store and a load through the same place
                // are: the destination's allocations are what `*pp` may point
                // at, and what is copied is one level below what `*ps` may
                // point at. Read only as plain locals, those built in silence.
                // Found by review.
                let copies = match kind {
                    Callee::Copies => true,
                    Callee::Frees
                    | Callee::Allocates
                    | Callee::Reallocates
                    | Callee::ReturnsFirst
                    | Callee::Ends
                    | Callee::Opaque => false,
                };
                if copies {
                    if let [Operand::Copy(dest), Operand::Copy(source), ..] = &arguments[..] {
                        let (into_depth, from_depth) = (derefs(dest), derefs(source));
                        let placed = (dest.projection.is_empty() || into_depth > 0)
                            && (source.projection.is_empty() || from_depth > 0);
                        if placed {
                            let (copied, copied_locals) =
                                value.levels_below(source.local, from_depth + 1);
                            // **What a copy that leaves carries is stored out**,
                            // as a store's is, asked before it lands: the sites
                            // a load of the source reads, and everything they
                            // hold. Without it `memcpy(pp, b, 8); free(x);`
                            // with `*b = x` was silent where `*pp = *b;` is
                            // `SC0414` (#684). See `Known::stored_out`.
                            let marks = self.store_at(terminator, value).is_some_and(|store| {
                                match store.kind {
                                    StoreKind::Copy => store.leaves,
                                    // `store_at` makes a copy alone; listed so
                                    // that a kind added is answered here.
                                    StoreKind::Direct | StoreKind::Through { .. } => false,
                                }
                            });
                            if marks {
                                for site in value.closure(copied.iter().copied().collect()) {
                                    value.stored_out[site] = true;
                                }
                            }
                            // **What it copies is a load of the source one level
                            // further**, `memcpy(box, pp, 8)` being `*box =
                            // *pp;`, built by the load's own `Known::load_into`
                            // rather than by a copy of it to drift (#777): the
                            // sites and addresses one level below the source,
                            // whatever type the source was declared with, since
                            // `memcpy` copies bytes and the usual source is a
                            // `void *`, and every mark a load sets. Among them
                            // whether what was stored there may be lost or gone,
                            // a slot a call could not replace (ADR-0047), what
                            // was read out of caller memory (ADR-0040), and
                            // whether it may be a lost value: `*box = u;
                            // memcpy(box2, box, 8); free(u); r = *box2;` read
                            // `*r` in silence until it was carried. And whether
                            // it reaches an array only by #639's rule, which a
                            // copy written apart once left out where a store
                            // stored such a pointer: `box[0] = m + 1;
                            // memcpy(&g, box, 8); g[0] = &x;` believed `m[0]`
                            // written where `g = box[0];` doubts it. Found by
                            // review, each.
                            let mut read = Held::from_marks(
                                value.points_to.len(),
                                Marks {
                                    lost: false,
                                    freed: None,
                                    writes_elsewhere: true,
                                    offset: Offset::Zero,
                                    loaded: true,
                                    foreign: false,
                                    stale_read: false,
                                    unreplaced_read: false,
                                    lost_read: false,
                                    from_caller: false,
                                    returned_by: None,
                                    fresh_read: false,
                                    believed: None,
                                    contents_at_most: Levels::default(),
                                    inner_address: false,
                                    element: None,
                                    array_edge: false,
                                    stored_through: false,
                                },
                            );
                            let caller =
                                self.reads_caller_memory(source.local, from_depth + 1, value);
                            value.load_into(
                                &mut read,
                                source.local,
                                from_depth + 1,
                                &|local| self.is_array(function, local),
                                caller,
                            );
                            // As of this copy. See `Held::named_at_read`.
                            for named in value.locals_named_by_unknown_code() {
                                read.named_at_read.set(named);
                            }
                            // Where it lands, as a store one dereference
                            // further would. See `store::Landing`.
                            let landing = self.landing(
                                value,
                                dest.local,
                                into_depth + 1,
                                into_depth == 0,
                                StoreKind::Copy,
                            );
                            // **Into a local's address, the copy lands in the
                            // local**: `memcpy(&s, &r, n)` stores into `s`
                            // what `r` holds, every mark with it, since C17
                            // 7.24.2.1 p2 copies the bytes. The locals are the
                            // ones the destination's address edges name, or,
                            // for a place of dereferences, the locals the
                            // level above may be, as for a store's deep
                            // targets. How it lands in them is below. See
                            // ADR-0039 and ADR-0040.
                            //
                            // **Whether what it copies may be a pointer that
                            // reaches an array only by #639's rule** is the
                            // load's `array_edge`, carried into a local with the
                            // rest of the value, as a store through the address
                            // gives it, and into memory as the arrays it marks
                            // where it is poured, `Known::stored_in_part`.
                            // Without either `e = m + 1; memcpy(box, &e, 8); f =
                            // box[0]; *f = &x;`, and the same into `&f`, left
                            // `m[0]` believed written where the store form is
                            // doubted.
                            let targets = landing.targets.clone();
                            // **Believed as a load at the local's type would be**,
                            // `s = *(T **)&v`: that type, no wider than the
                            // source's memory recorded, as `believed_read`
                            // answers a load (ADR-0061). Carried with no belief,
                            // `memcpy(&s, &v, 8)` of a whole `int`'s address was
                            // doubted at `*s` where `s = v;` is not (#769).
                            let recorded = value
                                .memory_levels(source.local, from_depth + 1, self.deepest_pointer)
                                .at(0);
                            // **As its type where what it copies is read
                            // fresh**, as `believed_read` believes such a load:
                            // the local then holds what nothing wrote, as one
                            // never assigned does, and a read of it is
                            // reported where it is read, once; nothing was
                            // stored to record a width. Carried with none,
                            // `memcpy(&s, b, 8)` of fresh `b` was doubted again
                            // at `*s` where `memcpy(m, b, 8)` was not (#791).
                            // **At depth zero only**, the one level `*source`
                            // asks. See ADR-0057 and ADR-0061.
                            let fresh = from_depth == 0
                                && self.reads_fresh(
                                    function,
                                    &Place {
                                        local: source.local,
                                        projection: vec![Projection::Deref],
                                    },
                                    value,
                                );
                            // **And it replaces the local where it is the
                            // store's certain one**, as `s = v;` does: one
                            // local and nothing else (`Landing::strong`),
                            // and a constant length of exactly its size. Anywhere
                            // else by union, since `n` may copy
                            // part of a pointer, or none, and a replacement
                            // would prove things about a value nobody wrote
                            // (#395). By union alone, the local's own null was
                            // joined in and took the belief away.
                            let replaces = self.copy_replaces_one_local(
                                function,
                                value,
                                arguments.first(),
                                self.constant_argument(*callee, arguments, 2),
                            );
                            for &target in &targets {
                                let mut each = read.clone();
                                let local = function.locals().nth(target.index());
                                let as_read = local
                                    .and_then(|local| {
                                        self.pointee_size(function, &Place::local(local))
                                    })
                                    .unwrap_or(0);
                                let width = as_read.min(recorded.unwrap_or(0));
                                each.believed = if fresh {
                                    Some(as_read)
                                } else {
                                    self.read_from_fits(
                                        function,
                                        source.local,
                                        from_depth + 1,
                                        width,
                                        value,
                                    )
                                    .then_some(width)
                                };
                                // The one target, which is all `targets`
                                // holds where the copy replaces it; any
                                // other is a may-write, as a union is.
                                match replaces {
                                    Some(at) if at.local() == target => value.replace(at, each),
                                    // **Naming no element**: a copy that does
                                    // not replace the local may write part of
                                    // a pointer, or none, so what it leaves is
                                    // not the element the source names, and
                                    // taking it made `memcpy(&g, &e, 4); *g =
                                    // &x;` a certain write of `m[1]`, which
                                    // nothing wrote. Found by review.
                                    _ => {
                                        each.element = None;
                                        value.target_union(target, &each);
                                    }
                                }
                            }
                            // **A copy into memory this check cannot place lands
                            // where a store there would**: `memcpy(w, s, 8)`
                            // with `w` a load was believed where `*w = *s;`
                            // was doubted. Found by review. See ADR-0044.
                            // **And the bound on what it copies**, which a copy
                            // carries as a store of a load would: what the
                            // source's memory is believed to hold, from the
                            // type it arrived as or a load at its type, and
                            // what was stored in the memory it points at.
                            // `memcpy(b, m, 8)` with `m` a `void **`
                            // parameter, or `b` filled from memory that held a
                            // `char *` nothing recorded, handed `b` on as an
                            // `int **` with nothing asked (#632). **At every
                            // level**, as a store records it (#634).
                            let mut copied_bound = self.contents_read(function, source, value);
                            copied_bound.bound_by(&value.memory_levels(
                                source.local,
                                from_depth + 1,
                                self.deepest_pointer,
                            ));
                            let poured = Cell::poured(
                                value.points_to.len(),
                                copied.iter().copied(),
                                copied_locals.iter().copied(),
                                &copied_bound,
                                &read,
                                // A copy records the width of the bytes it
                                // copies (#711), so nothing it leaves is
                                // without one; and marking a copy out of
                                // memory nothing wrote doubted its width
                                // beside the `SC0411` already said there,
                                // measured on #767.
                                false,
                            );
                            let in_part: Vec<LocalIdx> = if read.array_edge {
                                copied_locals.iter().copied().collect()
                            } else {
                                Vec::new()
                            };
                            // Where it lands, unplaced too, with what it
                            // carries, as a store's landing is given. See
                            // `Allocations::land_contents`.
                            self.land_contents(value, &landing, &poured, &in_part, Pours::Copy);
                        }
                    }
                }
                // What a pointer it was handed may be, read out of memory, as
                // for an opaque call, exposed, and what the copy writes of what
                // is unwritten. See `Allocations::initialised_by` and ADR-0040.
                let reach = self.reach(function, handed, sites(), value);
                self.initialised_by(
                    function,
                    value,
                    Wrote::Copy {
                        dest: arguments.first(),
                        reach,
                        span: origin.span(),
                        length: self.copied_pointers(*callee, arguments),
                        bytes: self.constant_argument(*callee, arguments, 2),
                        source: arguments.get(1),
                    },
                );
            }
            Callee::Opaque => {
                // **What the caller stored is out of sight from here on**, as
                // it is after a free of the caller's pointer. See ADR-0040.
                value.callers_memory_may_be_freed();
                // **What it holds it may have freed**: what it is handed, by
                // name or as a load, the allocations the memory it was handed
                // holds however deep, and what an argument read out of memory
                // may be. Not the locals whose address that memory holds,
                // which `Known::closure` does not follow; what the call
                // reaches through one is answered by `held_out_of_reach`,
                // below. `q = *t; release(q);` and `*d = a; release_in(d);` may
                // each free `a`'s allocation, and neither can replace `a`, so
                // a later call by address asks about it. Not `reach`, which
                // adds every escaped local's sites to every call, so
                // `grow(&a)` would reach `a` there and the in-out idiom would
                // be doubted at the next call by address; what only an address
                // or an earlier exposure reaches stays `Reachable`, below. See
                // ADR-0047.
                //
                // **A proved free stays proved**, whatever the call was
                // handed, because no call un-frees an allocation (ADR-0017).
                // Writing `Unknown` over what an argument named turned a
                // proved use after free into a doubt, which a hatch only lists:
                // `free(p); g(r); return *p;` built in a hatch where `r` may
                // be `p` (#262).
                let mut handed_memory = self.handed_to_opaque_call(function, handed, value);
                for site in value.closure(handed_memory.clone()) {
                    value.state[site].may_be_freed();
                }

                // **Everything this call could reach, and everything reached
                // before it by code this check cannot read**, is unproven
                // after it, because any of it may be what this call frees.
                // What it can reach includes what an argument read out of
                // memory may be, which `reach` says. See ADR-0039.
                let reach = self.reach(function, handed, sites(), value);
                value.expose(self.sources, reach, Some(origin.span()));
                value.unproved_exposed();
                // Pending from here, and not at its own exposure above, which
                // `Known::noticed` leaves out by `by` anyway. See ADR-0042.
                let span = origin.span();
                value
                    .calls
                    .insert((span.file().index(), span.start(), span.end()), span);

                // **A hatch may have reached anything, so every allocation
                // still live is unproven after it.** The loop above is about
                // the sites the arguments name, and this check does not follow
                // a pointer stored into memory it does not model: `*box = p;
                // drop_inner(box);` hands the callee `p`'s allocation one
                // level down, where no argument names it. Everywhere else that
                // gap is answered by the callee's own body being checked, and
                // a hatch's body is the one whose unproven conclusions are
                // listed rather than reported. So what the body cannot answer
                // the caller assumes the worst of, which is ADR-0032's default
                // with nothing declared to narrow it. A proved free stays
                // proved: nothing a callee does un-frees it. See ADR-0038.
                if self.frees_anything(*callee) {
                    for state in &mut value.state {
                        // Reachable: a hatch reaches what it was not handed
                        // only as code this check cannot read. See ADR-0047.
                        state.doubted();
                    }
                }

                // **What it was handed is not all it can reach.** The loop
                // above is about the allocations the arguments name; this is
                // about the *locals* whose addresses are out there, which this
                // call may write a fresh pointer into whether or not it was
                // passed one.
                //
                // **`Callee::Frees` and `Callee::Allocates` are not here**, and
                // what says so is what each is handed rather than a sentence
                // forbidding the write: C17 7.22.3.4 gives `malloc` a size and
                // no address at all, and 7.22.3.3 gives `free` the pointer's
                // *value*, which p2 of that subclause requires to be one an
                // allocation function returned and which 7.22.3 requires to be
                // disjoint from every other object. Neither is ever handed
                // `&p`. That is the whole reason the name is read. `realloc`
                // is handed a pointer's value too, by 7.22.3.5 p2, and is not
                // here either; the library functions that return their first
                // argument can be handed `&p`, and do this in their own arm.
                //
                // **No test holds this**: marking at either arm leaves the
                // whole workspace green, measured. The record says so rather
                // than leaving the next reader to find out by widening it.
                // See ADR-0029.
                //
                // **Every local, because a callee's write has no type this
                // function knows.** The other producer of this fact narrows by
                // the type it writes, which is ADR-0031; a callee's body is not
                // read, so there is nothing here to narrow by. Narrowing this
                // one would need an annotation saying what a callee writes,
                // and `_Nonnull` is not one.
                //
                // **Written, though, only where it can reach**: the locals
                // whose address it was handed, what it can find from there and
                // from everything handed over before, which
                // `Known::expose_locals` walks. `lost` and `foreign` stay on
                // every escaped local, since a route missed there is a free in
                // silence; a route missed here is a report. `int **p = m;
                // tick(); q = *p;` read `m` in silence (#643). See ADR-0057.
                //
                // **No case holds `lost` and `foreign` wider than this set**,
                // measured: giving them the walk's answer as well leaves the
                // whole suite green, since every route the corpus has is one
                // the walk finds. They stay wide because a route it misses
                // would cost a free nobody is told about, not because a case
                // says so.
                //
                // **What an argument may be the address of**: the locals its
                // address edges name, which `written_through` answers for a
                // plain local, or, for a place of dereferences, the locals the
                // level above may be. **An argument naming none hands none**:
                // one this check lost reads as nothing below it. A pointer
                // moved off an array keeps its edge (#639), so `fill(&m[1] -
                // 1)` may write `m`. A report from one naming none is one the
                // reader sees, and widening it would silence every local
                // beside such a call. See ADR-0057.
                let handed_locals = Self::locals_handed_to_opaque_call(handed, value);
                value.expose_locals(handed_locals);
                let may_write: Table<LocalIdx, bool> = Table::from_vec(
                    value
                        .exposed
                        .locals()
                        .map(|local| value.exposed[local])
                        .collect(),
                );
                // And an array handed through a pointer moved off it is
                // written in part, as a store through that pointer is: asked
                // of every local the call may write, since it may write
                // through any pointer it reaches, `fill(&e)` through `e = m +
                // 1`. See `built::write_effects`.
                let mut partial: Vec<(LocalIdx, ArrayWrite)> = Vec::new();
                for argument in handed {
                    if let Operand::Copy(place) = argument {
                        let lands = may_write.indices().filter(|&local| may_write[local]);
                        for (local, write) in super::built::write_effects(
                            self.unit,
                            function,
                            value,
                            place.local,
                            derefs(place) + 1,
                            lands,
                            true,
                        ) {
                            if !partial.iter().any(|(each, _)| *each == local) {
                                partial.push((local, write));
                            }
                        }
                    }
                }
                // **And an array handed over in part before is written in
                // part again**: a call this check cannot read may write what
                // an earlier one kept, and only an element of it was in
                // reach. See `Known::exposed_in_part`. Not one this call is
                // handed whole, at its start, which it may write all of:
                // `fill(m)` after `fill(&m[1])`. Found by review of #667.
                let handed_whole = |local: LocalIdx| {
                    handed.iter().any(|argument| match argument {
                        Operand::Copy(place) if place.projection.is_empty() => {
                            let held = &value.points_to[place.local];
                            held.names_local(local) && held.element == Some(0) && !held.array_edge
                        }
                        _ => false,
                    })
                };
                for (local, &may) in may_write.enumerated() {
                    if may
                        && value.exposed_in_part[local]
                        && !partial.iter().any(|(each, _)| *each == local)
                        && !handed_whole(local)
                    {
                        partial.push((local, ArrayWrite::Maybe));
                    }
                }
                partial.retain(|(local, _)| may_write[*local]);
                for (local, _) in &partial {
                    value.exposed_in_part[*local] = true;
                }
                let pending = value.array_writes_before(partial);
                value.replaced(|_| true, |local| may_write[local]);
                value.array_writes_after(pending);
                // **And as what each local it reaches was handed**, which is
                // the most its body can be asked to have written there: a
                // local reached as a `void *` takes a `void *` store, asked
                // its start and no width, and one reached below a `void *`
                // takes whatever the body converts the pointer to. `put(pv,
                // &c)` with `pv` an `int *`'s address converted and `put`
                // taking `void **` left a `char`'s address believed a whole
                // `int` (#632). The walk is the one a departure takes, so the
                // two cannot read a call's arguments differently.
                // An argument past every parameter, to a variadic or an
                // unprototyped callee, is read as its own type, as a
                // departure reads it; a constant hands no local.
                let declared = self.unit.function(*callee);
                let parameters: Vec<LocalId> = declared.parameters().collect();
                for (index, argument) in arguments.iter().enumerate() {
                    let ty = match (parameters.get(index), argument) {
                        (Some(&parameter), _) => declared.local(parameter),
                        (None, Operand::Copy(place)) => match self.unit.place_ty(function, place) {
                            Some(ty) => ty,
                            None => continue,
                        },
                        (None, Operand::Constant(_)) => continue,
                    };
                    let Ty::Pointer(read_as) = self.unit.ty(ty) else {
                        continue;
                    };
                    let (locals, sites, untyped_sites) = slots_leaving(
                        self,
                        function,
                        read_as,
                        &Rvalue::Use(argument.clone()),
                        value,
                    );
                    for (local, read_as) in locals {
                        self.handed(value, local, read_as);
                    }
                    // And the memory it reached, as the locals are. See
                    // `Known::handed_untyped`.
                    for (site, read_as) in sites {
                        self.handed_site(value, site, Some(read_as));
                    }
                    for site in untyped_sites {
                        self.handed_site(value, site, None);
                    }
                }
                // **And below whatever an earlier call was handed untyped**,
                // as it is now (#679): that callee may have kept the address
                // and store through it at this call, so `keep(&v); v = &w;
                // other(); use1(w);` handed a one-byte allocation on in
                // silence. The same walk, from each local and memory at each
                // type it was recorded at. It costs a doubt about
                // whatever is put below a marked local or memory and handed
                // on after any later call, by the user's decision. See
                // `Known::handed_untyped` and ADR-0060.
                let marked_locals: Vec<(LocalId, Option<TyId>)> = function
                    .locals()
                    .flat_map(|local| {
                        value.handed_untyped[local]
                            .iter()
                            .map(move |&read_as| (local, read_as))
                    })
                    .collect();
                let marked_sites: Vec<(Site, Option<TyId>)> = value
                    .handed_untyped
                    .sites()
                    .flat_map(|site| {
                        value.handed_untyped[site]
                            .iter()
                            .map(move |&read_as| (site, read_as))
                    })
                    .collect();
                if !marked_locals.is_empty() || !marked_sites.is_empty() {
                    let (locals, sites, untyped_sites) =
                        slots_from(self, function, value, marked_locals, marked_sites);
                    for (local, read_as) in locals {
                        self.handed(value, local, read_as);
                    }
                    for (site, read_as) in sites {
                        self.handed_site(value, site, Some(read_as));
                    }
                    for site in untyped_sites {
                        self.handed_site(value, site, None);
                    }
                }
                // **And what it wrote there, or into memory it was handed, may
                // be what it was handed**: `get(&r, u)` and `put(box, u)` may
                // store `u`, and `free(u)` then made `*r` a use of freed
                // memory read in silence, for a named allocation as for a
                // lost one (#510). So every escaped local may hold, and every
                // allocation the call was handed may contain, what it was
                // handed. Only that, not everything reachable to code this
                // check cannot read: the reachable set doubted `keep(a);
                // get(&p); *p = 1;`, the output-parameter idiom ADR-0017
                // declines to report, wherever any allocation was exposed. A
                // local that gains a site is a load for its readers, so a use
                // of it is a doubt and never a proof; one that gains none is
                // left as it was, so `get(&u); free(u);` keeps its words. See
                // ADR-0044.
                //
                // **A local's address hands what the local holds**, as it does
                // for the trigger above: `w = u; get(&r, &w);` may store `u`
                // in `r`, and `Known::closure` does not follow a local, so
                // `*r` after `free(u)` was read in silence. Found by review.
                // At any depth, since `pa = &a; get(&r, &pa);` hands `a`'s
                // too and read `*r` in silence the same way (#513).
                let mut deeper: Vec<Site> = Vec::new();
                for argument in handed {
                    if let Operand::Copy(place) = argument {
                        let first = value.written_through(place.local);
                        for &target in &first {
                            handed_memory.extend(value.points_to[target].sites());
                        }
                        for target in value.beneath(first) {
                            deeper.extend(value.points_to[target].sites());
                        }
                    }
                }
                let shallow = value.closure(handed_memory.clone());
                handed_memory.extend(deeper);
                let stored: Vec<Site> = value.closure(handed_memory).into_iter().collect();
                // **What only a deeper level hands is kept out of a local that
                // holds a local's address**: `pp`, in `pp = &p; opaque(&pp);`,
                // points at `p` and not at the allocation `p` holds, and giving
                // it that allocation doubted `*pp = q`, the output-parameter
                // idiom ADR-0017 declines to report; so did giving `pa`, in
                // `two(&pa, &pb);`, what `pb`'s chain hands. Only for those
                // sites, so what one level hands is held as before this rule:
                // kept out of it, `k = 0; r = &k; get(&r, u);` read `*r` after
                // `free(u)` in silence, and so did `void *` spellings in which
                // such a local really may hold it. Read from `Held::locals`
                // rather than from a declared type, which nothing here
                // enforces; so a deeper pointer a callee stores into a local
                // that holds an address, through `void *` or a cast, is
                // missed, as it was before. Found by review. See ADR-0044.
                let deep: Table<Site, bool> = Table::from_vec(
                    value
                        .state
                        .indices()
                        .map(|site| !shallow.contains(&site))
                        .collect(),
                );
                for local in value.escaped.indices() {
                    if !value.escaped[local] {
                        continue;
                    }
                    // **What the callee may write here is believed whole, as
                    // its result is**, since it is a store in the callee's
                    // body, asked there; and only where what the local held
                    // before is whole too, since a callee that writes
                    // nothing leaves that. Believing it whatever it held let
                    // `keep(&a); a = a + 5; other();` pass `a` on as whole.
                    // A belief nothing here can show stays off. See ADR-0060.
                    //
                    // **No wider than it was handed as**, at this call or an
                    // earlier one ([`Known::handed_at_most`], #632).
                    let believed = function.locals().nth(local.index()).and_then(|id| {
                        let width = self.pointee_size(function, &Place::local(id))?;
                        let width =
                            value.handed_at_most[local].map_or(width, |most| width.min(most));
                        let whole = matches!(
                            fits(self, function, id, width, value, false),
                            None | Some(Fit::Inside)
                        );
                        whole.then_some(width)
                    });
                    let holds_an_address = value.points_to[local].may_be_a_locals_address();
                    // As of this load, and before the borrow below. See
                    // `Held::named_at_read`.
                    let named_now = value.locals_named_by_unknown_code();
                    let handed_contents = value.handed_contents_at_most[local].clone();
                    let held = &mut value.points_to[local];
                    held.believed = believed;
                    // **And what its memory holds, no more than it was
                    // handed as at every level below** (#636), bounding what
                    // it held, since a callee that writes nothing leaves
                    // that. See `Known::handed_contents_at_most`.
                    held.contents_at_most.bound_by(&handed_contents);
                    for &site in &stored {
                        if !(holds_an_address && deep[site]) {
                            held.hold(site, Offset::Unknown);
                        }
                    }
                    if !stored.is_empty() {
                        held.loaded = true;
                        for &named in &named_now {
                            held.named_at_read.set(named);
                        }
                    }
                    if hands_a_lost_value {
                        held.lost_read = true;
                    }
                }
                for &container in &stored {
                    for &site in &stored {
                        value.contents[container].name_site(site);
                    }
                }
                // **What it wrote there it handed over**, as a callee's
                // result or what a parameter points at is, so the next call
                // this check cannot read may free it: `acquire(&r);
                // release(); return *r;` was believed (#394). Nothing is
                // said until such a call, so `get(&p); *p = 1;`, the
                // output-parameter idiom ADR-0017 declines to report, is
                // not. See ADR-0040.
                for (held, &escaped) in value.points_to.iter_mut().zip(&value.escaped) {
                    if escaped {
                        held.from_caller = true;
                    }
                }

                // **And what it cannot replace keeps what it may have freed.**
                // The other half of the line above: a local whose address the
                // call cannot reach still holds what the call reached through
                // another one, `b = a; release_ref(&b);`, and is asked about
                // it at the next call by address. Only a local read after
                // this call counts, and after a call that does not return
                // nothing is (ADR-0051). What the call was handed comes
                // first, since a holder it was handed is one it may replace.
                // See ADR-0047.
                value.handed_to_a_call(handed);
                value.held_out_of_reach(|local| {
                    then.is_some_and(|then| self.live_in[then.index()][local.index()])
                });
            }
        }

        let Some(place) = destination else {
            return;
        };
        let Some(at) = Strong::assigned(place) else {
            // `*box = f();` writes an allocation this does not place. See
            // `Known::init`. **No C program reaches this**, measured: the
            // lowering calls into a temporary and stores that, which the
            // store transfer clears for. It is here for a frontend that
            // does not.
            if derefs(place) > 0 {
                value.written_all();
            }
            // And an escaped local it may land in, as an unplaced store
            // does. See `Known::init`.
            replaced_by(self.unit, function, place, value);
            return;
        };
        // The call's result writes the local it lands in. See
        // `Known::init`.
        value.written_local(LocalIdx::of(place.local));

        match kind {
            Callee::Ends => {
                unreachable!("`Allocations::terminator` returns for it where `handed` is decided")
            }
            // `free` returns nothing, and the C frontend gives it no
            // destination. A frontend that gives it one gives it a `void`
            // local, which holds none of this.
            Callee::Frees => {
                value.replace(at, Held::none(value.points_to.len()));
                return;
            }
            // What it returns is its first argument, unmoved: C17 7.24.2 to
            // 7.24.6 say so of each. See ADR-0039.
            Callee::ReturnsFirst | Callee::Copies => {
                let first = match arguments.first() {
                    Some(Operand::Copy(source)) if source.projection.is_empty() => {
                        value.points_to[source.local].clone()
                    }
                    // `memset(*tab, 0, 4)` returns a pointer read out of
                    // memory, and says so, as an assignment of `*tab` would.
                    // **Nothing holds this**: the call has just exposed what
                    // `*tab` reaches, so a later call handed the result finds
                    // it exposed already. It is here so that the day this
                    // family stops exposing what it is handed, its result is
                    // not a silence. See ADR-0040.
                    Some(Operand::Copy(source)) => self.read_through(function, source, value),
                    Some(Operand::Constant(_)) | None => Held::none(value.points_to.len()),
                };
                value.replace(at, first);
                return;
            }
            Callee::Allocates | Callee::Reallocates | Callee::Opaque => {}
        }

        // A call leaves behind something this function did not have before, and
        // the local it landed in is what names it. **Live rather than joined
        // with what was there**: a second turn of a loop through the same call
        // is a second allocation, and carrying the first one's `Freed` across
        // would report a double free for code that allocates each time round.
        let result = LocalIdx::of(place.local);
        let site = Site::of(result);
        // Read before the rebirth clears it: a loop through this call writes
        // this site every turn, and what the call returns may be last turn's
        // allocation, exposed, which the site number cannot tell apart.
        let was_exposed = value.exposed[site];
        value.replace(at, Held::none(value.points_to.len()));
        // At its start: the site is whatever the call handed back, so the
        // value is that value and not an offset into it.
        value.points_to[result].hold(site, Offset::Zero);
        // **The span only where this check saw an allocation.** Every call's
        // destination is a site, because a call this cannot read may hand back
        // anything and a site is how that is tracked. But `allocated here` is a
        // claim, and `void *p = bar();` gives no evidence that `bar` allocated
        // anything. Naming that line was a caret asserting something nothing
        // had established, so a site whose call is not an allocation function
        // this check reads by name, `malloc`, `calloc`, `aligned_alloc` or
        // `realloc`, is `Live(None)` and the diagnostic leaves the label off.
        let made = match kind {
            Callee::Allocates | Callee::Reallocates => Some(origin.span()),
            Callee::Opaque | Callee::Frees | Callee::ReturnsFirst | Callee::Copies => None,
            Callee::Ends => {
                unreachable!("`Allocations::terminator` returns for it where `handed` is decided")
            }
        };
        value.reborn(site, made);
        value.extent[site] = Some(self.made_extent(function, *callee, kind, arguments, place));
        // **What a call this check cannot read returns is believed whole**,
        // as a parameter is, and bounded by what it may return that this
        // check recorded: an allocation this function handed it is folded in
        // at its size. The `return` in its body is where the belief is asked,
        // where that body is one this check reads. See ADR-0060.
        let opaque = match kind {
            Callee::Opaque => true,
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Ends => false,
        };
        if opaque {
            value.points_to[result].believed = self.pointee_size(function, place);
            // And what its memory holds as its type says, as a parameter's.
            value.points_to[result].contents_at_most = self
                .unit
                .place_ty(function, place)
                .and_then(|ty| self.type_levels(ty))
                .unwrap_or_default();
        }
        // **Its contents are indeterminate** where its row says so, and for
        // `realloc` where it was handed a pointer that is certainly null, as
        // `malloc` for the size (C17 7.22.3.5 p3), a constant `0` or a local
        // that names nothing, or one that names only allocations nothing has
        // written, whose indeterminate bytes it copies (p2). The row says it
        // rather than a name read here, which had read `realloc(NULL, n)` as
        // copying an old object (#689). See `Leaves`.
        //
        // A nonzero constant, or a place reached through a dereference, is
        // read as the old object: neither is null in a defined program, and
        // the null a dereference could read is undefined to read already.
        let fresh = match self.library(*callee).and_then(|row| row.leaves) {
            Some(Leaves::Indeterminate) => true,
            Some(Leaves::TheOld) => match arguments.first() {
                Some(Operand::Constant(0)) => true,
                Some(Operand::Copy(handed)) => {
                    handed.projection.is_empty()
                        && (value.names_nothing(handed.local)
                            || value.names_only_fresh(handed.local))
                }
                Some(Operand::Constant(_)) | None => false,
            },
            Some(Leaves::ZeroBits) | None => false,
        };
        if fresh {
            value.init[site] = Init::Unwritten;
        } else if let (Some(Leaves::TheOld), Some(Operand::Copy(handed))) = (
            self.library(*callee).and_then(|row| row.leaves),
            arguments.first(),
        ) {
            // **And may be indeterminate where the old block may have been**,
            // fresh or maybe so beside what else it names: it copies those
            // bytes as well (#724). See `Known::init`.
            if handed.projection.is_empty() && value.may_name_fresh(handed.local) {
                value.init[site].doubt();
            }
        }
        // **`realloc`'s new object holds what the old one held**, C17 7.22.3.5
        // p2, so what a pointer stored in the old one may hold, the new one
        // may. Reborn with nothing in it, a table grown by `realloc` and then
        // handed to a call exposed nothing it held. See ADR-0039.
        match kind {
            Callee::Ends => {
                unreachable!("`Allocations::terminator` returns for it where `handed` is decided")
            }
            Callee::Reallocates => {
                let old: Vec<Site> = handed
                    .iter()
                    .filter_map(|argument| match argument {
                        Operand::Copy(source) if source.projection.is_empty() => Some(source.local),
                        Operand::Copy(_) | Operand::Constant(_) => None,
                    })
                    .flat_map(|local| value.sites_of(local).collect::<Vec<_>>())
                    .filter(|&old| old != site)
                    .collect();
                // **And the bound on what it holds**, from the pointer it was
                // handed as from the old allocation below: a `void **`
                // parameter grown by `realloc` and read through an `int **`
                // was believed four bytes wide (#632).
                for argument in handed {
                    if let Operand::Copy(source) = argument {
                        if source.projection.is_empty() {
                            let handed = value.points_to[source.local].contents_at_most.clone();
                            value.contents[site].bound_stored(&handed);
                        }
                    }
                }
                // **What the old allocation held, the new one holds after the
                // copy**: its record poured whole, the marks with the rows,
                // since what the old allocation may hold of a reborn site is
                // what the new one holds (ADR-0045), the caller's mark for the
                // same reason (ADR-0040), and whether a lost value was stored
                // in it, or a free of one after `box = realloc(box, 16);`
                // finds nothing in the new allocation to mark, found by
                // review (a cell's `lost`). See `Known::pour`.
                for old in old {
                    let held = value.contents[old].clone();
                    value.pour(&[site], &held, &[]);
                }
                if let Some(old) = realloced {
                    value.realloced[site] = Some(Realloced {
                        at: origin.span(),
                        old,
                    });
                    value.points_to[result].returned_by = Some(site);
                }
            }
            Callee::Frees
            | Callee::Allocates
            | Callee::ReturnsFirst
            | Callee::Copies
            | Callee::Opaque => {}
        }
        // **Or any allocation code this check cannot read may reach**, which a
        // call it cannot read may hand back: `stash(p); q = fetch();` may make
        // `q` be `p`. At an offset nobody said, since what comes back may point
        // into one. These are the allocations the call has just unproven, so
        // the result is as doubtful as what it may be and never less, which is
        // what a widened set has to be, since widening one can make this check
        // say less. See ADR-0039.
        match kind {
            Callee::Ends => {
                unreachable!("`Allocations::terminator` returns for it where `handed` is decided")
            }
            Callee::Opaque => {
                // **What it returned may hold a pointer the callee kept**, the
                // same fact as a parameter's memory from the other side of a
                // call, so the next free of the caller's or call this check
                // cannot read stales it. Here rather than where the function
                // starts, because the call that returns it cannot have freed
                // through it: marked from the start, this very call staled its
                // own result. **In a loop it does anyway**, from the second
                // turn: the site names last turn's result as well, the mark
                // from then is still on it, and `reborn` keeps `stale_read`
                // because a site's allocations are not told apart. So a load
                // out of each turn's result is doubted, a false report the
                // reader can see. See ADR-0040.
                value.contents[site].mark_from_caller();
                // **And it may be the lost value it was handed**, `id(*box)`
                // returning `u` (#508), **or one an earlier call was handed**,
                // as it holds every exposed site: `stash(u); r = fetch();`
                // (#511). The trigger above has already set the fact if this
                // call was handed one. **Lost at once**, as a named value the
                // call may have freed is doubted at once. A mark that waited
                // for a later free of a lost value, `Held::lost_read`, left
                // `stash(u); free(u); r = fetch(); *r` and `release_all()` in
                // place of the free read in silence, since nothing after the
                // call made it lost. Found by review. See ADR-0045.
                if value.lost_exposed {
                    value.points_to[result].lost = true;
                }
                let exposed: Vec<Site> = value
                    .exposed
                    .sites()
                    .filter(|&other| other != site && value.exposed[other])
                    .collect();
                for other in exposed {
                    value.points_to[result].hold(other, Offset::Unknown);
                }
                // **And any escaped local's address**, for the same reason
                // and with the same reach: `id(&x)` may return `&x`, and
                // `stash(&x); fetch()` may too. `Held::locals` is how a free
                // learns it, through `Held::may_be_a_locals_address`, and how
                // a write through the result is followed into the locals it
                // may land in. See ADR-0039.
                for local in value
                    .escaped
                    .indices()
                    .filter(|&local| value.escaped[local])
                {
                    value.points_to[result].name_local(local);
                }
                if was_exposed {
                    value.state[site] = SiteState::Unknown;
                }
                // **And what it returns is exposed**: the callee had the pointer,
                // and may have kept it where the next call can reach it.
                value.expose(self.sources, [site], Some(origin.span()));
            }
            Callee::Frees
            | Callee::Allocates
            | Callee::Reallocates
            | Callee::ReturnsFirst
            | Callee::Copies => {}
        }
        // After the state, because this is what takes it away again. No C
        // reaches here with an escaped destination: the lowering writes every
        // call that returns something into a fresh temporary and copies it
        // out, and one that returns `void` nowhere, so the copy above is
        // what a C program goes through. Another frontend need not, and
        // `a_call_into_a_local_whose_address_escaped` builds the shape by hand.
        value.unproved(result);
    }
}

/// Add `ty` to the types `kept` records, once. See `Known::handed_untyped`.
fn remember(kept: &mut Vec<Option<TyId>>, ty: Option<TyId>) {
    if !kept.contains(&ty) {
        kept.push(ty);
    }
}

/// How many `*`s deep a type is: `0` for one that is no pointer. What
/// [`Allocations::deepest_pointer`] is the largest of.
///
/// Written out rather than as a `matches!`, so a kind of type added later
/// is asked whether it has a pointee.
pub(super) fn pointer_depth(unit: &TranslationUnit, ty: TyId) -> usize {
    match unit.ty(ty) {
        Ty::Pointer(pointee) => 1 + pointer_depth(unit, pointee),
        Ty::Int | Ty::Char | Ty::Void => 0,
        // As deep as its element: `int ***a[2]` holds pointers three `*`s
        // deep, which `Allocations::deepest_pointer` has to be long enough for.
        // **Held by nothing, measured**: answering `0` changes no case,
        // because every use of an array converts it into a temporary of the
        // element's pointer type, which is one `*` deeper still.
        Ty::Array { element, .. } => pointer_depth(unit, element),
    }
}

/// [`Allocations::reads_caller_memory`], asked of a value no program shows.
#[cfg(test)]
mod caller_memory {
    use crate::dataflow::Analysis;
    use crate::ir::{Function, TranslationUnit, Ty};
    use crate::source::{SourceMap, Span};
    use crate::target::Target;

    /// A local this check lost that still holds a pointer parameter's site
    /// reads the caller's memory, though the walk of its levels refuses it.
    ///
    /// Mutation: drop the test of `exposed_parameters` from
    /// `reads_caller_memory`. This fails.
    #[test]
    fn a_lost_local_holding_a_parameters_site_reads_caller_memory() {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "f");
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let ptr = unit.push_type(Ty::Pointer(int));
        let ptr_ptr = unit.push_type(Ty::Pointer(ptr));
        let mut function = Function::new(Span::new(file, 0, 1), int, [ptr_ptr]);
        let pp = function.parameters().next().expect("the one parameter");
        let block = function.reserve_block();
        function.fill_block(
            block,
            crate::ir::Block {
                elements: vec![],
                terminator: crate::ir::Terminator::Return,
            },
        );
        let func = unit.push_function(function);
        let analysis = crate::memory::analysis(&sources, &unit, func);
        let mut value = analysis.on_entry();
        assert!(analysis.reads_caller_memory(pp, 1, &value));
        value.points_to[pp].lost = true;
        assert!(analysis.reads_caller_memory(pp, 1, &value));
    }

    /// A local holding the site of what a call this check cannot read
    /// returned reads memory the callee handed over, though that site's
    /// memory carries no mark of it; and one holding a site nothing returned
    /// does not.
    ///
    /// **No program shows the first half**: the `Callee::Opaque` arm marks
    /// the cell of what the call returned `from_caller` as it makes the site,
    /// and `marked_below` answers for that before this clause is asked.
    /// Dropping the clause alone fails no corpus case, the mark alone two,
    /// and both eight.
    ///
    /// Mutation: drop the opaque-result clause from `reads_caller_memory`,
    /// or answer `false` for `Callee::Opaque` in `opaque_results_of`. The
    /// first assertion fails.
    #[test]
    fn a_local_holding_what_an_opaque_call_returned_reads_callee_memory() {
        use crate::ir::{Block, Origin, Place, Terminator};
        use crate::memory::index::{LocalIdx, Site};
        use crate::memory::parts::Offset;

        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "f get");
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let ptr = unit.push_type(Ty::Pointer(int));
        let get = unit.push_function(Function::new(Span::new(file, 2, 5), ptr, []));
        let mut function = Function::new(Span::new(file, 0, 1), int, []);
        let result = function.push_local(ptr);
        let other = function.push_local(ptr);
        let holder = function.push_local(ptr);
        let call = function.reserve_block();
        let exit = function.reserve_block();
        function.fill_block(
            call,
            Block {
                elements: vec![],
                terminator: Terminator::Call {
                    callee: get,
                    arguments: vec![],
                    destination: Some(Place::local(result)),
                    then: Some(exit),
                    origin: Origin::Written(Span::new(file, 2, 5)),
                },
            },
        );
        function.fill_block(
            exit,
            Block {
                elements: vec![],
                terminator: Terminator::Return,
            },
        );
        let func = unit.push_function(function);
        let analysis = crate::memory::analysis(&sources, &unit, func);
        let mut value = analysis.on_entry();
        value.points_to[holder].hold(Site::of(LocalIdx::of(result)), Offset::Zero);
        assert!(analysis.reads_caller_memory(holder, 1, &value));

        let mut value = analysis.on_entry();
        value.points_to[holder].hold(Site::of(LocalIdx::of(other)), Offset::Zero);
        assert!(!analysis.reads_caller_memory(holder, 1, &value));
    }
}
