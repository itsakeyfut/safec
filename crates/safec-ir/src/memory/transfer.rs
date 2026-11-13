//! The transfer functions of the analysis, [`Allocations`], whose fields are
//! in `allocations.rs`: what each element, terminator and edge of a function
//! does to the [`Known`] value, and what the analysis asks of the IR to answer
//! that.

use std::collections::{BTreeMap, BTreeSet};

use crate::dataflow::Analysis;
use crate::ir::{
    BinOp, BlockId, DOES_NOT_RETURN, Element, FuncId, Function, LocalId, Operand, Place,
    Projection, Rvalue, Terminator, Ty, TyId,
};
use crate::source::Span;

use super::LostReason;
use super::allocations::{Allocations, innermost_element, stored_pointer};
use super::built::{built_from, named};
use super::fit::{Fit, fits};
use super::index::{Idx, Loc, LocalIdx, Site, Table, Target};
use super::init::Wrote;
use super::known::{ELEMENTS_KEPT, Known};
use super::parts::{
    At, Cell, Earliest, Extent, Freeing, Held, Init, Inits, Levels, Marks, PendingRead, PlaceInit,
    Reached, SiteState, Width, earlier, joined_carried,
};
use super::store::{Landing, StoreKind, Strong};
use crate::library::{Callee, Library};
use crate::reads::{dereferenced_in_element, dereferenced_in_terminator, derefs};

/// **What a write writes, as the transfer asks it**: whether what it
/// writes may be a pointer, how many levels below it hold pointers, and how
/// many bytes it writes. A store says these from the type of the place it
/// writes; a library copy, which has no type, says them from what it copies
/// and its length. Said once where the write is read, so that no rule below
/// asks a type of its own (the memory model rebuilt on #924, stage 2): the
/// store's rules read the place's type at each one, and a copy, whose place
/// is a `void *`, could not be handed to them.
#[derive(Clone, Copy)]
pub(super) struct Write {
    /// Whether what is written may be a pointer.
    pub(super) pointer: bool,
    /// How many levels below what is written hold pointers.
    pub(super) levels: usize,
    /// How many bytes are written, where that is known.
    pub(super) width: Option<u64>,
}

/// What [`Allocations::land_contents`] is told of one write beyond where it
/// lands.
pub(super) enum Pours {
    /// An assignment: whether what it stores may be a pointer, the
    /// allocations what it stores holds, and where it is.
    Store {
        pointer_stored: bool,
        carried: Vec<Site>,
        span: Span,
        /// The one block it replaces whole, where
        /// [`Allocations::replaces`] says it does.
        replaces: Option<Site>,
    },
    /// A library copy, and the one block it replaces whole, where
    /// [`Allocations::replaces`] says it does.
    Copy { replaces: Option<Site> },
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
                    Some(*op),
                    &[lhs, rhs],
                    value,
                    |local| self.is_pointer(function, local),
                    |place| self.may_be_pointer(function, place),
                    |place| self.reads_a_byte(function, place),
                    |local, depth| self.reads_caller_memory(local, depth, value),
                    |local| self.is_array(function, local),
                    self.stride(function, lhs, rhs),
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
            // **A unary operator keeps what its operand reaches**, as a
            // binary one with no pointer operand does, by the same function
            // (#806). See `built::built_from`.
            Rvalue::Unary { operand, .. } => {
                let mut reached = built_from(
                    None,
                    &[operand],
                    value,
                    |local| self.is_pointer(function, local),
                    |place| self.may_be_pointer(function, place),
                    |place| self.reads_a_byte(function, place),
                    |local, depth| self.reads_caller_memory(local, depth, value),
                    |local| self.is_array(function, local),
                    None,
                );
                reached.moved_by_arithmetic(None);
                reached
            }
            // A constant, an address. Neither is a pointer this check
            // follows to an allocation, so the target is given nothing: a
            // write this check cannot follow is not evidence that the old
            // contents are gone.
            Rvalue::Use(Operand::Constant(_)) | Rvalue::Address(_) => {
                Held::none(value.points_to.len())
            }
        }
    }

    /// The per-element record each local starts with: a bit set for every
    /// element of an array of at most `ELEMENTS_KEPT` pointers that is
    /// unwritten at entry, and none for any other local. See
    /// `Known::written`.
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

    /// How many bytes the one operand of `lhs op rhs` that is a pointer, or
    /// an array, points at: what a constant added to it moves it by, each,
    /// in bytes (C17 6.5.6 p8). `None` where neither operand is, or both
    /// are, or what it points at has no size, `void`'s: a distance that
    /// cannot be counted is never one (ADR-0092 rule 1). See
    /// `built::offset_of`.
    fn stride(&self, function: &Function, lhs: &Operand, rhs: &Operand) -> Option<u64> {
        let sizes: Vec<Option<u64>> = [lhs, rhs]
            .into_iter()
            .filter_map(|operand| match operand {
                Operand::Copy(place) => match self.unit.ty(self.unit.place_ty(function, place)?) {
                    Ty::Pointer(pointee) => Some(self.unit.size_of(pointee)),
                    Ty::Array { element, .. } => Some(self.unit.size_of(element)),
                    Ty::Int | Ty::Char | Ty::Void => None,
                },
                Operand::Constant(_) => None,
            })
            .collect();
        match sizes[..] {
            [size] => size,
            _ => None,
        }
    }

    /// **The one block a write of `width` bytes at `offset` replaces
    /// whole**, where it certainly does: `*b = 0;` after `*b = x; free(x);`
    /// leaves nothing of `x` in `b`, and a store that only added to what the
    /// block may hold doubted the free-then-null idiom at every door that
    /// asks it (#656, ADR-0094 rule 1). **The store and the library copy
    /// both ask it** (#927): `memcpy(b, &z, 8)` with `z` null is the same
    /// write, and was still doubted. All of these hold, each because without
    /// it the block may keep what it held:
    ///
    /// - landing in exactly one site and nothing else
    ///   ([`Landing::certain`](super::store::Landing::certain));
    /// - at the block's start, `At::Bytes(0)`: `b[k] = 0;` may land past
    ///   the end and leave what `b` holds;
    /// - as wide as the whole block, whose extent is exactly that constant
    ///   size: `contents` is kept per site, not per slot, so a block of two
    ///   slots keeps the other's pointer. Only an allocator or `realloc`
    ///   handed a constant makes an exact extent; a parameter's block and an
    ///   opaque call's result are at least their type (`made_extent`);
    /// - the site made on no cycle of the graph (`Allocations::made_once`,
    ///   ADR-0082), so it stands for one block: a site made again on a
    ///   later turn keeps what the earlier block held (ADR-0045), and
    ///   replacing it forgot that.
    ///
    /// `realloc`'s result is one block too, though it may be the one it was
    /// handed: what is read through the old pointer after it is reported
    /// there, and the old site keeps its own record.
    pub(super) fn replaces(
        &self,
        landing: &Landing,
        offset: At,
        width: Option<u64>,
        value: &Known,
    ) -> Option<Site> {
        let site = landing.certain()?.site();
        let width = width?;
        let whole = offset == At::Bytes(0) && value.extent[site] == Some(Extent::Exactly(width));
        (whole && self.made_once[site.local()]).then_some(site)
    }

    /// What a store through `place` replaces, by [`Self::replaces`]: one
    /// dereference, at the width of what it stores.
    fn store_replaces(
        &self,
        place: &Place,
        wrote: Write,
        landing: &Landing,
        value: &Known,
    ) -> Option<Site> {
        if place.projection.as_slice() != [Projection::Deref] {
            return None;
        }
        self.replaces(
            landing,
            value.points_to[place.local].offset,
            wrote.width,
            value,
        )
    }

    /// Record a store narrower than a pointer as writing part of one, in each
    /// block or local its pointer names that is wide enough to hold a
    /// pointer: a pointer later read out of that slot, or let leave, is made
    /// of the bytes it wrote and others (#913). `*ip = 1;` with `ip` an
    /// `int *` into memory of pointers, or `*c = 1;` with `c` a `char *`
    /// (#917), was counted as writing the slot whole. See
    /// `Known::part_written`, ADR-0087 and ADR-0092 rule 5.
    ///
    /// **Wherever it lands, at any depth**: one dereference in what its
    /// pointer names, and deeper in what the level above may be, the
    /// store's own [`Landing`]. Asked at one dereference alone, `**cc = 1;`
    /// with `*cc` into memory of pointers wrote the slot whole where `*c =
    /// 1;` is part of a pointer; found by the architecture review of
    /// 2026-11-13. A store at a position this check cannot count, a `char`
    /// loop's `c[i] = 0;` among them, is a may-write of the slot as well
    /// (ADR-0094 rule 3, #893). **Not memory narrower than a pointer**,
    /// which holds no pointer to make of it. A store of a pointer's whole
    /// width is not narrow.
    fn note_narrow_store(&self, landing: &Landing, wrote: Write, at: Span, value: &mut Known) {
        let target = self.unit.target();
        let pointer = u64::from(target.pointer_bits() / target.char().bits());
        if !wrote.width.is_some_and(|width| width < pointer) {
            return;
        }
        match landing.deep {
            0 => {}
            1 => self.note_part_of_a_pointer(landing.pointer, at, value),
            _ => {
                let places: Vec<Loc> = landing
                    .containers
                    .iter()
                    .map(|&site| Loc::Heap(site))
                    .chain(landing.targets.iter().map(|&local| Loc::Local(local)))
                    .collect();
                self.note_less_than_a_pointer(places, at, value);
            }
        }
    }

    /// Record a write through `local` at `at` as writing part of a pointer
    /// in each block or local it names that is wide enough to hold one, where
    /// it points at a position counted from the start. One body for a
    /// narrow store and a partial `memset`. See [`Self::note_narrow_store`].
    pub(super) fn note_part_of_a_pointer(&self, local: LocalId, at: Span, value: &mut Known) {
        let held = &value.points_to[local];
        let places: Vec<Loc> = held
            .sites()
            .map(Loc::Heap)
            .chain(held.locals().map(Loc::Local))
            .collect();
        self.note_less_than_a_pointer(places, at, value);
    }

    /// **A write of less than a pointer into `places`**, whoever wrote it: a
    /// store narrower than a pointer, a `memset` or a library copy whose
    /// length ends part way through one. **One rule for the three** (the
    /// memory model rebuilt on #924, stage 2): written three times, they
    /// noted different places, and `c[0] = one;`, `memset(c, 1, 1);` and
    /// `memcpy(c, &one, 1);` over one stored pointer gave three answers.
    ///
    /// **Every place not known to be narrower than a pointer**, which holds
    /// no pointer to be part of: noted, `memcpy(&k, &i, 4)` of an `int` into
    /// an `int` moved the report off the copy that wrote half a pointer,
    /// found by review. A place of no constant size may hold one, and is
    /// noted, whatever the write's position.
    ///
    /// **And what was stored through a pointer there is no longer what a
    /// read through it reads**: `*m = x; *c = 1; q = *m;` with `c` into `m`'s
    /// block read half of `x` in silence, found while measuring #917.
    pub(super) fn note_less_than_a_pointer(
        &self,
        places: impl IntoIterator<Item = Loc>,
        at: Span,
        value: &mut Known,
    ) {
        let target = self.unit.target();
        let pointer = u64::from(target.pointer_bits() / target.char().bits());
        let narrower = |extent: Option<Extent>| matches!(extent, Some(Extent::Exactly(size)) if size < pointer);
        let places: Vec<Loc> = places
            .into_iter()
            .filter(|&place| !narrower(value.extent[place]))
            .collect();
        for &written in &places {
            value.note_part_written(self.sources, written, at);
        }
        value.written_over(&places);
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
    /// **[`Ty::is_pointer`] answers, an exhaustive `match`, because the
    /// answer for a kind nobody has added yet is not `false`.** A type this
    /// does not recognise is dropped from an addition that has a pointer
    /// beside it, and a dropped operand is a site nothing reports.
    pub(super) fn is_pointer(&self, function: &Function, local: LocalId) -> bool {
        self.unit.ty(function.local(local)).is_pointer()
    }

    /// Whether a place may hold a pointer, by its type.
    ///
    /// **`None` answers yes.** [`TranslationUnit::place_ty`](crate::ir::TranslationUnit::place_ty) has no type for a
    /// `Deref` of something that is not a pointer, which the lowering does not
    /// build and a hand-built unit can, and a place whose type this cannot
    /// name is one it cannot narrow: [`replaced_by`](super::built::replaced_by) reads it the same way.
    ///
    /// **A `char` answers no**, though C17 6.5 p7 lets one copy a pointer a
    /// byte at a time and [`replaced_by`](super::built::replaced_by) reads it as reaching everything for
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
            levels.push(Width::AtMost(width));
        }
        Some(Levels::new(levels))
    }

    /// Whether a value of type `ty` points at memory that holds pointers,
    /// through an array to its element as [`Self::type_levels`] reads it:
    /// `void **` and `int *(*)[2]` do, `int *` and `void *` do not. See
    /// [`Held::arrived_holds_pointer`] and ADR-0073 rule 4.
    ///
    /// Written out rather than as a `matches!`, so a kind of type added
    /// later, a structure holding a pointer, is asked here too.
    pub(super) fn first_level_holds_a_pointer(&self, ty: TyId) -> bool {
        let Ty::Pointer(stored) = self.unit.ty(ty) else {
            return false;
        };
        stored_pointer(self.unit, stored).is_some()
    }

    /// Mark what a write that leaves the function carries: a store through
    /// a pointer the caller can read, or a library copy into one. One body
    /// for both, since the copy arm once marked `stored_out` and not
    /// `left_at`, and `memcpy(out, &b, 8)` handed the caller a block nothing
    /// wrote in silence where `*out = b` is `SC0411` (#890).
    ///
    /// - **`carried`**, the blocks the written value names, are left holding
    ///   pointers, for the function's exit to ask whether they were written,
    ///   and **so are the blocks they hold**, as many levels down as the type
    ///   reads pointers (`levels`, through `Known::held_below`): `*box = b;
    ///   *out = box;` hands the caller `b`'s block as surely as `*out = box;
    ///   *box = b;` does, and was silent where the second is `SC0411` (#900).
    ///   See `Known::left_at`, ADR-0076 and ADR-0085.
    /// - **`carried` and `read_out`**, what a pointer read out of memory may
    ///   be, are stored out, and so is **everything those allocations hold,
    ///   at every level**, as `freed_behind` asks what leaves: a box stored
    ///   out holds what was stored in it before, and `*b = x; *pp = b;
    ///   free(x);` left the caller a freed pointer one level down in silence.
    ///   Found by review of #650. See `Known::stored_out`.
    pub(super) fn mark_leaving(
        &self,
        value: &mut Known,
        carried: Vec<Site>,
        read_out: Vec<Site>,
        levels: usize,
        at: Span,
    ) {
        for site in value.held_below(carried.clone(), levels) {
            value.left_at[site] = Some(match value.left_at[site] {
                Some(before) => earlier(self.sources, before, at),
                None => at,
            });
        }
        let mut out = carried;
        out.extend(read_out);
        for site in value.closure(out) {
            value.stored_out[site].note(self.sources, at);
        }
    }

    /// How many levels below a value of type `ty` hold pointers, counted
    /// while [`Self::first_level_holds_a_pointer`] holds and stepping to the
    /// pointee through arrays. `int ***` is `2`: the block it points at, and
    /// the blocks that block holds. `int **` is `1`, and `int *` and
    /// `void *` are `0`. These are the levels a block that leaves is asked
    /// at. See ADR-0085.
    pub(super) fn levels_holding_pointers(&self, ty: TyId) -> usize {
        let mut levels = 0;
        let mut ty = innermost_element(self.unit, ty);
        while self.first_level_holds_a_pointer(ty) {
            levels += 1;
            let Ty::Pointer(below) = self.unit.ty(ty) else {
                break;
            };
            ty = innermost_element(self.unit, below);
        }
        levels
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
            return value.points_to[place.local].contents_at_most();
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
    ///
    /// **A `void *` at what its record says**, which no type bounds: `void
    /// *w = *pv;` with `*pv` holding a `char`'s address reads `w` as one
    /// byte wide, so `char *p = w; *p = 1;` is proved where it was unknown
    /// (#631). Only a width the record holds, `Width::AtMost`: where it
    /// holds none, or the type's, which `void` has none of, it believes
    /// nothing, as before. See ADR-0093 rule 4.
    pub(super) fn believed_read(&self, function: &Function, place: &Place, value: &Known) -> u64 {
        let typed = self.typed_width(function, place);
        let as_read = typed.unwrap_or(0);
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
        believed_width(typed, bound)
    }

    /// How wide what `place` points at is as its type: `None` for a `void
    /// *`, whose pointee has no size to bound a width, and `0` for a place
    /// that is no pointer.
    pub(super) fn typed_width(&self, function: &Function, place: &Place) -> Option<u64> {
        match self
            .unit
            .place_ty(function, place)
            .map(|ty| self.unit.ty(ty))
        {
            Some(Ty::Pointer(pointee)) => self.unit.size_of(pointee),
            Some(Ty::Int | Ty::Char | Ty::Void | Ty::Array { .. }) | None => Some(0),
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
    pub(super) fn land_contents(
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
                replaces,
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
                //
                // **And the blocks it carries, and every block reachable from
                // them, keep what was known of their writes**: the exposure
                // believes every exposed block written
                // by code this check cannot read (`Known::expose`), and a store
                // runs no code. Believed here, `t = *out; *t = b;` called `b`
                // written and nothing asked it again (#858). A call this check
                // cannot read believes it written later, as it does any exposed
                // block. The closure, not the carried blocks alone: a block one
                // level below the stored one was believed written by the same
                // store, found by review. See ADR-0077.
                let before = value.written.clone();
                let stored: Vec<Site> = value.closure(carried.clone()).into_iter().collect();
                let keep = |value: &mut Known| {
                    for &site in containers.iter() {
                        if before[site].whole() == Init::Maybe {
                            value.written[site].set(Init::Maybe);
                        }
                    }
                    for &site in &stored {
                        value.written[site] = before[site].clone();
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
                    // **What it replaces holds nothing else first**, so the
                    // pour below leaves what this store carries alone
                    // (ADR-0094 rule 1, #656).
                    if let Some(site) = replaces.filter(|_| !elsewhere) {
                        value.contents[site] = Cell::none(value.points_to.len());
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
            Pours::Copy { replaces } => {
                if elsewhere {
                    value.landed_where_unknown_code_points(poured);
                }
                // What it replaces holds nothing else first, as a store's
                // does (#927).
                if let Some(site) = replaces.filter(|_| !elsewhere) {
                    value.contents[site] = Cell::none(value.points_to.len());
                }
                value.pour(containers, poured, in_part);
            }
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
    pub(super) fn read_through(&self, function: &Function, source: &Place, value: &Known) -> Held {
        // **The load door's answer for every field, never `..`**, so a field
        // added to `Known` answers here for what a load does with it. A load
        // writes no field: what it reads becomes a `Held`, which the
        // assignment puts in its destination. See ADR-0091.
        let Known {
            // Read: what the pointer read through names, at every level, by
            // `Known::load_into` and `Self::contents_read`.
            points_to: _,
            // Not read by a load.
            state: _,
            escaped: _,
            handed_away: _,
            // Read: what code this check cannot read may name, for
            // `Held::named_at_read`.
            exposed: _,
            // Read: what memory holds where it is read, by
            // `Known::load_into`.
            contents: _,
            // Not read by a load.
            realloced: _,
            // Read: whether what is read was written, slot by slot where the
            // place keeps slots, by `Self::reads_fresh`.
            written: _,
            // Not read by a load: about reads and calls in flight.
            pending: _,
            calls: _,
            exposed_after_call: _,
            // Read: how wide what is read out of is, by `Self::contents_read`
            // and `Self::read_from_fits`.
            extent: _,
            handed_at_most: _,
            // Not read by a load.
            exposed_in_part: _,
            // Read: an array reached only by #639's rule, by
            // `Known::load_into`.
            stored_in_part: _,
            // Not read by a load: about where a block has left, which the
            // doors it leaves by and the exit ask.
            stored_out: _,
            left_at: _,
            held_levels: _,
            part_written: _,
            // Not read by a load: what a call may have stored is asked where
            // the value is handed on.
            handed_untyped: _,
        } = value;
        // Every mark a load gives before it reads anything, each said, so a
        // mark added is answered here (#732); what it reads is
        // `Known::load_into`'s, and whether it is fresh is below.
        let mut held = Held::from_marks(
            value.points_to.len(),
            Marks {
                lost: false,
                freed: None,
                writes_elsewhere: true,
                offset: At::Bytes(0),
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
                //
                // Level 0 of `widths`, below.
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
                widths: self.contents_read(function, source, value).stored_under(
                    Width::of_belief(
                        self.pointee_size(function, source)
                            .map(|_| self.believed_read(function, source, value)),
                    ),
                    usize::MAX,
                ),
                // And whether that type holds pointers in its memory, which
                // `0` there does not say. See `Held::arrived_holds_pointer`.
                arrived_holds_pointer: self
                    .unit
                    .place_ty(function, source)
                    .is_some_and(|ty| self.first_level_holds_a_pointer(ty)),
                inner_address: false,
                element: None,
                site_element: None,
                array_edge: false,
                stored_through: false,
                // Memory records no null, so what is read out of it may be
                // one (#931).
                may_be_null: true,
            },
        );
        // What could be named as this was read. See `Held::named_at_read`.
        if held.beyond().loaded() {
            for named in value.locals_named_by_unknown_code() {
                held.named_at_read.set(named);
            }
        }
        // **And what was stored where it was read from**, at an offset nobody
        // said, so that a dereference of it can be asked about what it may
        // point at. `loaded` stays set, which is what marks the set as
        // possibly incomplete for every reader. See ADR-0045.
        let depth = derefs(source);
        if held.beyond().loaded() && depth > 0 {
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
            if held.believed().is_some_and(|width| {
                !self.read_from_fits(function, source.local, depth, width, value)
            }) {
                held.believe(None);
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
    pub(super) fn read_from_fits(
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
    pub(super) fn reads_caller_memory(&self, local: LocalId, depth: usize, value: &Known) -> bool {
        let held = &value.points_to[local];
        // At least the local's own allocations, which a load with no
        // dereference in its place, an element, still reads through.
        held.beyond().is_from_caller()
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
    pub(super) fn read_out(
        &self,
        function: &Function,
        operand: &Operand,
        known: &Known,
    ) -> Vec<Site> {
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
            return if held.beyond().any_unknown() {
                known.stored()
            } else {
                Vec::new()
            };
        }
        if place.projection.as_slice() == [Projection::Deref]
            && !held.beyond().any_unknown()
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
                (held.beyond().loaded(), LostReason::ReadOutOfMemory),
                (held.may_be_a_locals_address(), LostReason::MayBeALocal),
                (
                    held.beyond().foreign() && reached.len() > before,
                    LostReason::Other,
                ),
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

    /// **On the edge where an allocation's result is tested null, that
    /// allocation's block does not exist**, so whether it was written asks
    /// nothing there: it is written, vacuously. Without it, `if (b) *b = x;
    /// return b;` joins a path where the call failed, its site `Unwritten`,
    /// with the path where the block was written, and a block possibly
    /// unwritten where it leaves could not be doubted without refusing that
    /// program (#872).
    ///
    /// **Only where nothing else can name the site, for a site made in a
    /// loop.** A site whose every call is on no cycle makes one block an
    /// activation, the one that did not come to exist, and is written
    /// whatever names it (`Allocations::made_once`, ADR-0082). Otherwise a
    /// site names every block its call made, and a block an earlier turn made may still be
    /// held in memory or by another local; writing the site then believed
    /// that block written, and programs `main` reported went silent, found
    /// by #870's adversary. So the rule asks [`Known::held_sites`], which
    /// over-counts by design (ADR-0048), of the value with the tested local,
    /// its copies, and the branch's own condition set aside: a comparison
    /// keeps what its operand reaches (#847), and is the test rather than a
    /// holder. A `realloc`'s site with a fact is
    /// [`Self::branch_on_reallocs_result`]'s.
    ///
    /// **A copy set aside is a copy of this call's result**, never an earlier
    /// call's: [`Known::reborn`] makes every other local that named the site
    /// lost when the call writes it again, and a lost local holds no site.
    ///
    /// **And the tested local holds no block on that edge**, so a store of it
    /// there, `if (!b) *out = b;`, lets nothing leave. See ADR-0080.
    fn branch_on_an_allocations_result(
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
        let returned = {
            let held = &value.points_to[local];
            let mut sites = held.sites();
            let (Some(returned), None) = (sites.next(), sites.next()) else {
                return;
            };
            if held.beyond().lost()
                || held.returned_by != Some(returned)
                || value.realloced[returned].is_some()
            {
                return;
            }
            returned
        };
        // `Terminator::successors` pushes `then` and then `otherwise`, so
        // index 1 is the other arm, as the nullability check reads it.
        if (index == 0) != (on_then == crate::nullability::Nullness::Null) {
            return;
        }
        let mut rest = value.clone();
        for other in function.locals() {
            let held = &value.points_to[LocalIdx::of(other)];
            let mut sites = held.sites();
            if let (Some(only), None) = (sites.next(), sites.next()) {
                if only == returned && held.returned_by == Some(returned) {
                    rest.clear(other);
                }
            }
        }
        rest.clear(condition.local);
        // **A site made at most once an activation needs no holder test**:
        // its one block is the one that did not come to exist, so every
        // copy of the result, a store before the test included, holds null
        // here. `*out = b; if (!b) return;` counted `out`'s memory as a
        // holder and said the block nothing wrote there (#877). A site made
        // in a loop keeps the test, since an earlier turn's block may be
        // what another holder names. See ADR-0082.
        if self.made_once[returned.local()] || !rest.held_sites()[returned] {
            value.written[returned].set(Init::Written);
        }
        value.clear(local);
    }

    /// **On the arm where a pointer was tested not null, it is not**, so
    /// [`Held::may_be_null`] is dropped there: `p = malloc(8); if (!p)
    /// return 0; q = realloc(p, 16);` hands `realloc` an old block, not a
    /// null (#931). Only the tested local; a copy of it taken before the
    /// test still may be null.
    fn branch_on_a_null_test(
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
        // `Terminator::successors` pushes `then` and then `otherwise`, so
        // index 1 is the other arm, as the nullability check reads it.
        if (index == 0) != (on_then == crate::nullability::Nullness::Null) {
            value.points_to[local].may_be_null = false;
        }
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
        if held.beyond().lost() || held.returned_by != Some(returned) {
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
    /// written on the other becomes. See `Known::written`.
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
            written,
            realloced,
            pending,
            calls,
            exposed_after_call,
            extent,
            handed_at_most,
            exposed_in_part,
            stored_in_part,
            stored_out,
            left_at,
            held_levels,
            part_written,
            handed_untyped,
        } = into;
        // May-facts, so a union, as `exposed` of a local is. See
        // `Known::handed_untyped`.
        for (here, there) in handed_untyped.iter_mut().zip(&from.handed_untyped) {
            for ty in there {
                if !here.contains(ty) {
                    here.push(*ty);
                }
            }
        }
        // May-facts, so a union keeping the earlier span, at both kinds of
        // join. See `Known::stored_out` and `Known::part_written`.
        for (here, there) in stored_out.iter_mut().zip(&from.stored_out) {
            here.join(self.sources, *there);
        }
        for (here, there) in part_written.iter_mut().zip(&from.part_written) {
            here.join(self.sources, *there);
        }
        // The larger, so a path that held the block as pointers asks it. See
        // `Known::held_levels`.
        for (here, there) in held_levels.iter_mut().zip(&from.held_levels) {
            *here = (*here).max(*there);
        }
        // A may-fact too, keeping the earlier store where both paths let the
        // block leave. See `Known::left_at`.
        for (here, there) in left_at.iter_mut().zip(&from.left_at) {
            *here = match (*here, *there) {
                (Some(here), Some(there)) => Some(earlier(self.sources, here, there)),
                (here, there) => here.or(there),
            };
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
        // edge of a loop shown to run. A place kept per slot is joined slot by
        // slot, and gains no doubt beyond its slots'. See `Known::written`.
        //
        // **One loop over every place, each kind by its own rule** (#779).
        // Both rules read `state` as it was before this join: `state` is
        // joined below, and must not be before this loop.
        for place in written.indices() {
            let there = &from.written[place];
            let site = match place {
                Loc::Local(_) => {
                    written[place].join(there, back);
                    continue;
                }
                Loc::Heap(site) => site,
            };
            let here = &written[place];
            let (whole_here, whole_there) = (here.whole(), there.whole());
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
            let whole = match (&state[site], &from.state[site]) {
                (_, SiteState::Unheld) => named_beside_unheld(whole_here, whole_there),
                (SiteState::Unheld, _) => named_beside_unheld(whole_there, whole_here),
                _ => whole_here.joined(whole_there, back),
            };
            // **A block's slots joined one by one, each by `Init::joined`**,
            // where either side keeps them: a side that does not counts every
            // slot as its whole is, so a block one turn wrote at one slot and
            // the next wrote whole is written whole at the back edge of a loop
            // shown to run, as each of its slots is, where its whole alone
            // joins as `Maybe`. Found by review: `*p = x; p = p + 1;` in such
            // a loop was doubted. The side that names it where the other does
            // not keeps its own, where its whole survives the join.
            let slots = match (&state[site], &from.state[site]) {
                (_, SiteState::Unheld) => here.slots().map(|slots| (slots.clone(), false)),
                (SiteState::Unheld, _) => there.slots().map(|slots| (slots.clone(), false)),
                _ => match (here.slots(), there.slots()) {
                    (Some(a), Some(b)) if a.len() != b.len() => None,
                    (None, None) => None,
                    (Some(slots), _) | (None, Some(slots)) => {
                        let len = slots.len();
                        let mut joined = here.spread(len);
                        joined.join(&there.spread(len), back);
                        Some((joined, true))
                    }
                },
            };
            // **Where its bytes came from**, as its record is joined: the
            // side that names it where the other does not, and otherwise
            // what both say (`PlaceInit::join`).
            let carried = match (&state[site], &from.state[site]) {
                (_, SiteState::Unheld) => here.carried(),
                (SiteState::Unheld, _) => there.carried(),
                _ => joined_carried(here.carried(), there.carried()),
            };
            written[place] = match slots {
                // Joined from both sides: the block is what its slots say.
                Some((slots, true)) => PlaceInit::slots_of(slots),
                // Kept from the side that names it, where the join left its
                // whole as that side had it.
                Some((slots, false)) if slots.summary() == whole => PlaceInit::slots_of(slots),
                Some((_, false)) | None => PlaceInit::whole_of(whole),
            };
            if let Some(carried) = carried {
                written[place].carried_by(carried);
            }
        }
        // Bounds, so the lower of two at every level. See
        // `Known::handed_at_most`.
        for (here, there) in handed_at_most.iter_mut().zip(&from.handed_at_most) {
            here.bound_by(there);
        }
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

        for (here, there) in points_to.iter_mut().zip(&from.points_to) {
            // **A side where the local holds no block, and nothing holds the
            // site, keeps the other side's `returned_by`**: no block of that
            // site exists there, so `p = 0; if (c) p = malloc(8); if (p) *p =
            // x;` still tests the allocation's result. Not where a `realloc`
            // fact survives the join above, which happens where both sides
            // hold it: `n = realloc(p, 8); if (c) n = 0;` nulls the result by
            // hand on one arm, and keeping `returned_by` there read that arm
            // as a failed call and lost a double free, found while building
            // it. Where only one side holds the fact, the join dropped it, and
            // `branch_on_reallocs_result` has nothing to act on; asking that
            // side's fact as well answered by the order of the arms, found by
            // review. See ADR-0080.
            let no_block = |held: &Held, state: &SiteState, site: Site| {
                held.sites().next().is_none()
                    && matches!(state, SiteState::Unheld)
                    && realloced[site].is_none()
            };
            let kept = match (here.returned_by, there.returned_by) {
                (Some(site), None) if no_block(there, &from.state[site], site) => Some(site),
                (None, Some(site)) if no_block(here, &state[site], site) => Some(site),
                _ => None,
            };
            here.joined(there, self.sources);
            if kept.is_some() {
                here.returned_by = kept;
            }
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
        self.branch_on_an_allocations_result(function, block, terminator, index, value);
        self.branch_on_a_null_test(function, block, terminator, index, value);
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
        // moves from `Bytes(0)` or `Moved` to `Unknown` at a join and never back,
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
        // **A site's slots in `Known::written` climb as its whole record
        // does**, two steps a slot at most, and at most `ELEMENTS_KEPT`
        // elements a site, and the table goes from kept to dropped once:
        // `2 * ELEMENTS_KEPT + 1` a site. Where a pointer points among them,
        // `Held::site_element`, goes from known to not once a local, which
        // the per-local slack above covers (#873).
        let elements = usize::try_from(2 * ELEMENTS_KEPT + 1).unwrap_or(usize::MAX);
        locals * locals * 4
            + locals * (locals + 26)
            + positions * (2 * locals + 3)
            + locals
            + locals * (positions + locals + 2)
            + 3 * locals * self.deepest_pointer * (locals + 2)
            + locals * elements
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
            exposed: Table::filled(false, Target::count(self.locals)),
            contents: Table::filled(Cell::none(self.locals), self.locals),
            realloced: Table::filled(None, self.locals),
            // Nothing has been read yet, so there is nothing a free could be
            // unordered against.
            pending: BTreeMap::new(),
            calls: BTreeMap::new(),
            exposed_after_call: BTreeSet::new(),
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
            handed_at_most: Table::filled(Levels::default(), self.locals),
            // Every place written but the locals `unwritten_at_entry` names,
            // every local but a parameter: a parameter's memory is the
            // caller's, which may have written it, and an allocation is made
            // fresh where it is made. **Per slot for a local array kept so**
            // (`Self::elements_at_entry`), and whole for every other place.
            written: {
                let unwritten = self.unwritten_at_entry();
                let mut written: Table<Loc, PlaceInit> =
                    Table::filled(PlaceInit::whole_of(Init::Written), Loc::count(self.locals));
                for (local, slots) in written.locals().zip(self.elements_at_entry()) {
                    written[local] = if slots.len() > 0 {
                        PlaceInit::slots_of(slots)
                    } else if unwritten[local.index()] {
                        PlaceInit::whole_of(Init::Unwritten)
                    } else {
                        PlaceInit::whole_of(Init::Written)
                    };
                }
                written
            },
            exposed_in_part: Table::filled(false, self.locals),
            stored_in_part: Table::filled(false, self.locals),
            // No call has been handed anything yet.
            // Nothing has been stored anywhere yet.
            stored_out: Table::filled(Earliest::none(), self.locals),
            left_at: Table::filled(None, self.locals),
            held_levels: Table::filled(0, self.locals),
            part_written: Table::filled(Earliest::none(), Loc::count(self.locals)),
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
            known.points_to[parameter].hold(Site::of(LocalIdx::of(parameter)), At::Bytes(0));
            // And what its memory holds is believed as its type says, which
            // the caller asked where it stored it. See `Held::contents_at_most`.
            let contents = self
                .type_levels(function.local(parameter))
                .unwrap_or_default();
            known.points_to[parameter].set_contents(&contents);
            known.points_to[parameter].arrived_holds_pointer =
                self.first_level_holds_a_pointer(function.local(parameter));
            // **And may be null, unless it is `_Nonnull`**: the caller may
            // pass none (#931).
            known.points_to[parameter].may_be_null = function.nonnull(parameter).is_none();
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
        self.joined(into, from, self.runs_once[head.index()].is_some());
    }

    /// A loop shown to run is left only through its head after a whole turn
    /// (ADR-0069 rule 6), so what leaves it is what its back edges brought.
    /// See ADR-0083.
    fn leaves_after_a_turn(&self, head: BlockId) -> Option<crate::runs::ShownToRun> {
        self.runs_once[head.index()]
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
        self.joined(into, from, self.runs_once[head.index()].is_some());
        for place in into.written.indices() {
            // **Not a site the paths into the loop did not name**, which the
            // back edge made and nothing before it wrote.
            if let Loc::Heap(site) = place {
                if matches!(before.state[site], SiteState::Unheld) {
                    continue;
                }
            }
            // **Slot by slot where the place keeps them**, a local array's and
            // a block's alike: a block written at one slot before the loop,
            // `Maybe` whole, and made again by its body, kept that slot written
            // where the whole block did not change. Found by review, a read
            // `main` reported went silent. See `PlaceInit::unwritten_again`.
            into.written[place].unwritten_again(&before.written[place], &from.written[place]);
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
                // **The store door's answer for every field, never `..`**, as
                // the copy's and `realloc`'s are, so a field added to `Known`
                // is `error[E0027]` here too: the store is the door fixed most
                // often, and was outside this table (the memory model rebuilt
                // on #924, stage 2).
                let Known {
                    // Written directly: the value of the local assigned, and
                    // of a local the store lands in through its address.
                    points_to: _,
                    // Written directly where a local's address is taken.
                    escaped: _,
                    // Written directly, `Known::written_local`, and through
                    // `Self::initialised_by`: what the store writes.
                    written: _,
                    // Written directly: how many levels a block it holds was
                    // held as pointers.
                    held_levels: _,
                    // Through `Self::land_contents`: what the blocks it lands
                    // in may contain, and what an unplaced store exposes.
                    contents: _,
                    exposed: _,
                    // Through `Known::noticed`, where what it stores reaches
                    // memory a pending read or call may see.
                    pending: _,
                    calls: _,
                    exposed_after_call: _,
                    // Through `Self::mark_leaving`: a store into memory that
                    // leaves marks what it carries (ADR-0085).
                    stored_out: _,
                    left_at: _,
                    // Through `Self::note_less_than_a_pointer`: a store
                    // narrower than a pointer (ADR-0087).
                    part_written: _,
                    // Read, by `Self::replaces` and the part-of-a-pointer
                    // rule: how wide each place is.
                    extent: _,
                    // Through `Known::pour`, which the landing pours with.
                    stored_in_part: _,
                    // Not written by this arm itself; a store changes none of
                    // these except through a helper named above.
                    state: _,
                    handed_away: _,
                    realloced: _,
                    handed_at_most: _,
                    exposed_in_part: _,
                    handed_untyped: _,
                } = &*value;
                // What this store writes, from the type of the place it
                // writes, said once. See `Write`.
                let place_ty = self.unit.place_ty(function, &operation.place);
                let wrote = Write {
                    pointer: self.may_be_pointer(function, &operation.place),
                    levels: place_ty.map_or(0, |ty| self.levels_holding_pointers(ty)),
                    width: place_ty.and_then(|ty| self.unit.size_of(ty)),
                };
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
                    kind && store.leaves && wrote.pointer
                });
                if marks {
                    let carried: Vec<Site> = self
                        .carried(function, &operation.value, value)
                        .sites()
                        .collect();
                    // How many levels the stored-into type reads pointers at.
                    let levels = wrote.levels;
                    let read_out = match &operation.value {
                        Rvalue::Use(written) => self.read_out(function, written, value),
                        _ => Vec::new(),
                    };
                    self.mark_leaving(value, carried, read_out, levels, operation.origin.span());
                }

                // An assignment to a local writes it, whatever it writes. See
                // `Known::written`.
                if let Some(at) = Strong::assigned(&operation.place) {
                    value.written_local(at.into());
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
                        width: wrote.width,
                    },
                );
                // And part of a pointer, where it is narrower than one. See
                // `Self::note_narrow_store`.
                self.note_narrow_store(&before, wrote, operation.origin.span(), value);

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
                    //
                    // **Not for a local a load out of recorded memory gave
                    // its sites**, which already name what that memory held,
                    // as `*m = *b;` carries exactly what `b`'s blocks hold:
                    // `t = *b; *m = t;` carried everything stored anywhere
                    // besides, `y`'s block among it a level too high, and a
                    // block written before it was returned through `*m` was
                    // reported as one nothing wrote (#920). Only where the
                    // load is believed to reach its record alone, the
                    // question a write through it asks (#726), and names a
                    // site; a pointer lost, or read out of a caller's memory
                    // or a slot a free or a lost value may have left
                    // something in, still carries everything stored. **No
                    // case holds either condition, measured**: what the load
                    // carries of those marks is poured with it anyway
                    // (`Cell::poured`), and a load out of caller memory
                    // stored and read back after a call is doubted either
                    // way; a load that names no site falls back to
                    // everything stored, which only adds to what it may be.
                    // Kept, as that question keeps its own unguarded marks,
                    // because each can only add a doubt.
                    let recorded_load = matches!(
                        &operation.value,
                        Rvalue::Use(Operand::Copy(source))
                            if source.projection.is_empty()
                                && value.points_to[source.local]
                                    .beyond()
                                    .believed_to_reach_only_its_edges()
                                && value.points_to[source.local].holds_a_site()
                    );
                    if let (Rvalue::Use(written), false) = (&operation.value, recorded_load) {
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
                    // `Width::AsTyped`, since no bound is now no belief
                    // (ADR-0061).
                    //
                    // **And any other value at the width of what it names**,
                    // so that `*box = &q; int *t = *box; *t = 1;` proves what
                    // it proved when a load believed its type (ADR-0061).
                    // **One rule with the load of a local's own level**,
                    // `Known::stored_width` (#804), handed the local the
                    // value was copied out of, whose bound a join keeps
                    // where the value's own belief it drops.
                    let held_in = match &operation.value {
                        Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => {
                            Some(LocalIdx::of(place.local))
                        }
                        Rvalue::Use(_)
                        | Rvalue::Address(_)
                        | Rvalue::Unary { .. }
                        | Rvalue::Binary { .. } => None,
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
                    //
                    // **No width, for a value that is not a pointer by its
                    // type**, `Nothing` at its own level: a pointer is given
                    // one above whatever it names, and one with no `bound`
                    // names nothing, which is a null and bounds nothing. See
                    // `Cell::held`.
                    let first = if wrote.pointer {
                        value.stored_width(&written, held_in)
                    } else {
                        Width::Nothing
                    };
                    let stored = written
                        .contents_at_most()
                        .stored_under(first, self.deepest_pointer);
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
                    );
                    self.land_contents(
                        value,
                        &landing,
                        &poured,
                        &in_part,
                        Pours::Store {
                            pointer_stored: wrote.pointer,
                            carried,
                            span: operation.origin.span(),
                            replaces: self.store_replaces(&operation.place, wrote, &landing, value),
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
                            Some(*op),
                            &[lhs, rhs],
                            value,
                            |local| self.is_pointer(function, local),
                            |place| self.may_be_pointer(function, place),
                            |place| self.reads_a_byte(function, place),
                            |local, depth| self.reads_caller_memory(local, depth, value),
                            |local| self.is_array(function, local),
                            self.stride(function, lhs, rhs),
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
                    // **A unary operator keeps what its operand reaches.** C17
                    // 6.5.3.3 gives unary `+`, `-` and `~` arithmetic operands
                    // only, and an integer can hold sites: one a call this
                    // check cannot read returned holds what the call was
                    // handed. A binary operation with no pointer operand keeps
                    // them, by the same function; giving none here lost them
                    // at `-i` (#806). An edge is given up on, as a move by an
                    // unknown amount is. See `built::built_from`.
                    Rvalue::Unary { operand, .. } => {
                        let mut reached = built_from(
                            None,
                            &[operand],
                            value,
                            |local| self.is_pointer(function, local),
                            |place| self.may_be_pointer(function, place),
                            |place| self.reads_a_byte(function, place),
                            |local, depth| self.reads_caller_memory(local, depth, value),
                            |local| self.is_array(function, local),
                            None,
                        );
                        reached.moved_by_arithmetic(None);
                        value.replace(at, reached);
                    }
                    // A constant is not a pointer this check can follow.
                    // **And a `0` is a null**, which the value says (ADR-0093
                    // rule 3): a join keeps another path's belief beside it.
                    Rvalue::Use(Operand::Constant(0)) => {
                        value.replace(at, Held::null(value.points_to.len()));
                    }
                    Rvalue::Use(Operand::Constant(_)) => {
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
                // **And as the local it is held in**, which adds what that
                // local's type says of the memory it points at: an
                // allocator's result arrives as no type, in a `void *`
                // temporary, and is held as a `void **` or an `int **` by the
                // copy out of it (#830). A union, as everywhere else, and
                // after the match for the reason above. See ADR-0073 rule 4.
                if self.first_level_holds_a_pointer(function.local(destination)) {
                    value.points_to[destination].arrived_holds_pointer = true;
                }
                // **And on what it names**, so a `void *` level it reaches the
                // caller through later still asks it (#902). See
                // `Known::held_levels` and ADR-0086.
                let held = self.levels_holding_pointers(function.local(destination));
                if held > 0 {
                    let named: Vec<Site> = value.sites_of(destination).collect();
                    for site in named {
                        value.held_levels[site] = value.held_levels[site].max(held);
                    }
                }
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
            Element::StorageLive { local, origin: _ } => value.storage_live(*local),
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

        self.call(
            function,
            callee,
            arguments,
            destination,
            then,
            origin,
            terminator,
            value,
        );
    }
}

pub(super) use crate::layout::pointer_depth;

/// The width a pointer read out of memory is believed to point at, where
/// it is read as pointing at `typed` bytes and what it was read out of
/// recorded `recorded`: no wider than either, and **a `void *`, `typed`
/// `None`, at what the record says**, which no type bounds (#631, ADR-0093
/// rule 4). Only a width the record holds, `Width::AtMost`; where it holds
/// none, or the type's, which `void` has none of, it believes nothing. One
/// answer for a load and for a library copy into a local (#929).
///
/// Mutation: answer `0` for a `void *`, and
/// `a_void_pointer_read_out_of_memory_is_believed_as_wide_as_its_record_says`
/// and `a_void_pointer_copied_out_of_memory_is_believed_as_wide_as_its_record_says`
/// are `SC0412` again.
pub(super) fn believed_width(typed: Option<u64>, recorded: Width) -> u64 {
    match (typed, recorded) {
        (Some(as_read), recorded) => as_read.min(recorded.bound().unwrap_or(0)),
        (None, Width::AtMost(recorded)) => recorded,
        (None, Width::AsTyped | Width::Unbounded | Width::Nothing) => 0,
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
        value.points_to[pp].lost_its_allocation();
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
        use crate::memory::parts::At;

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
        value.points_to[holder].hold(Site::of(LocalIdx::of(result)), At::Bytes(0));
        assert!(analysis.reads_caller_memory(holder, 1, &value));

        let mut value = analysis.on_entry();
        value.points_to[holder].hold(Site::of(LocalIdx::of(other)), At::Bytes(0));
        assert!(!analysis.reads_caller_memory(holder, 1, &value));
    }
}
