//! Whether an access fits the object its pointer reaches: the width
//! question the walk and the report both ask.
//!
//! In a file of its own so that the walk does not read it out of the
//! report, and a change to what is reported cannot change what the walk
//! believes. See ADR-0060 for the question and ADR-0063 for the split.

use crate::ir::{Function, LocalId, Operand, Place, Rvalue, Ty, TyId};
use crate::reads::derefs;
use crate::source::Span;

use super::allocations::{Allocations, innermost_element};
use super::index::{LocalIdx, Site};
use super::known::Known;
use super::parts::{At, Extent, SiteState, same};

/// What this check can say about whether an access fits the object its
/// pointer reaches.
///
/// Three answers and no fourth: proved, which is silence; outside, which is a
/// proof; and unknown. A pointer that holds nothing is not one of them and
/// is `None` from [`fits`], because a null or absent pointer is the
/// nullability check's question and not a size. See ADR-0060.
pub(super) enum Fit {
    Inside,
    Outside {
        extent: Option<u64>,
        made: Option<Span>,
    },
    Unknown,
}

/// Whether an access of `width` bytes through `local`'s own value fits the
/// object it reaches, or `None` where `local` holds nothing to reach.
///
/// **What it is believed or was read to take, and nothing else proves it.**
/// The order of the tests is the order in which each stops this check
/// knowing anything:
///
/// 1. Holding no site, no local's address, nothing read out of memory or
///    written by a call, nothing lost, and at its start, is a null or absent
///    pointer, which is `None` here. **At its start**, because arithmetic on
///    a null or on a local's address leaves a value holding none of those
///    that is not null either, and a null test on it takes the nullability
///    check's doubt away: `int *q = p + 1; if (q) *q = 1;` with `p` null or
///    `&x` was answered by neither check. Found by review.
/// 2. Where `doubt_unseen_writes`, for an access through the local itself:
///    lost, written by a call this check cannot read, or an escaped local
///    holding anything: whatever is known about it may be stale (ADR-0017,
///    ADR-0018), so it is unknown. The escape is asked because a write
///    through the escaped address can put any pointer in the local where
///    this check does not see it, so what it holds proves nothing.
/// 3. A value read out of memory on every path is believed by the type it was
///    read as, and nothing more ([`Held::believed`](super::parts::Held::believed)),
///    nor more than what that memory is recorded to hold. One read on some
///    paths only is unknown.
/// 4. Off its start, or perhaps off it: unknown, since no distance is
///    carried (ADR-0036).
/// 5. An edge that may be the address of something inside a local rather
///    than the local: unknown, since the local's size is not that object's
///    ([`Held::inner_address`](super::parts::Held::inner_address)). Not
///    [`Held::writes_elsewhere`](super::parts::Held::writes_elsewhere), which
///    a call's result and a join with a null path both set while every edge
///    names a whole local: asking it made `int *q = f(&x); return *q;`
///    unknown.
/// 6. Otherwise every object it may reach, each site's [`Extent`] and each
///    local whose address it holds at that local's size, is folded. Every
///    lower bound at least `width` is proved; every one exact and below
///    `width` is outside; anything between is unknown, since a may-set
///    proves nothing from one member: an access that fits one of two
///    allocations a local may hold may still overflow the other.
pub(super) fn fits(
    analysis: &Allocations<'_>,
    function: &Function,
    local: LocalId,
    width: u64,
    known: &Known,
    doubt_unseen_writes: bool,
) -> Option<Fit> {
    let held = &known.points_to[local];
    let holds_something = held.sites().next().is_some()
        || held.offset != At::Bytes(0)
        || held.may_be_a_locals_address()
        || held.beyond().unrecorded_address();
    if !holds_something {
        return None;
    }
    // **Where a value leaves, or a local is reached through its own address,
    // none of these is doubted**, because each
    // stands for a write that was asked where it happened. The escape stands
    // for writes this check did not see, and each marks the local
    // (`Known::replaced`): one through a pointer it cannot pin down is a
    // store in this function, asked as one; one by a call it cannot read is
    // a store in the callee's body, asked there. `lost` is one of those, or a
    // site made again by the same call, which makes it with the same size
    // every time, so the sites folded below answer for it. Doubting them
    // where a value leaves refused `grow(&a); grow(&a);` with `a` a whole
    // allocation, the output-parameter idiom, since the first call is what
    // marks `a`; doubting the escape of a local reached through its address
    // doubted every `**pi`, since that address is what escapes it. An access
    // through the local itself still doubts all three, which costs a false
    // report and not a silence. See ADR-0060.
    if doubt_unseen_writes
        && (held.beyond().stopped_following_or_written_unseen() || known.escaped[local])
    {
        return Some(Fit::Unknown);
    }
    // **What a call may have written is no wider than the local was handed
    // as**, on any path ([`Known::handed_at_most`]). The belief the call left
    // is a proof and goes at a join with a path that made no call, while the
    // bound is kept: `if (c) put(pv, &c);` with `put` taking `void **` left
    // `v` holding nothing believed and nothing named, and a read through its
    // address believed a whole `int` (#632).
    if held.beyond().foreign()
        && known.handed_at_most[local]
            .at(0)
            .bound()
            .is_some_and(|most| most < width)
    {
        return Some(Fit::Unknown);
    }
    let sites = held
        .sites()
        .map(|site| known.extent[site].unwrap_or(Extent::AtLeast(0)));
    // `LocalId` cannot be built from an index outside the IR, so the locals
    // are walked and kept where the edge names them.
    let locals = function
        .locals()
        .filter(|target| held.names_local(LocalIdx::of(target)))
        .map(|target| {
            analysis
                .unit
                .size_of(function.local(target))
                .map_or(Extent::AtLeast(0), Extent::Exactly)
        });
    let extents: Vec<Extent> = sites.chain(locals).collect();

    // **No wider than what the memory it was read from is known to hold**,
    // where that is recorded: a `void *` slot written with a `char`'s address
    // and read back through an `int **` alias, or an `int *` copied in by
    // `memcpy`, arrives believed four bytes wide and is known to name one.
    // The belief is the floor where nothing is recorded, and the record is
    // the floor where it says less. Found by review. Where the slot is a
    // local, the belief is kept only where the local's value fits, by
    // `Allocations::read_through`, and a pointer handed on is asked at the
    // width its receiver believes, by `report::contents_leaving` (#627); a pointer
    // off its start stored as a `void *` is asked at the store.
    if let Some(believed) = held.believed() {
        let recorded = extents.iter().all(|extent| extent.lower() >= width);
        return Some(if believed >= width && recorded {
            Fit::Inside
        } else {
            Fit::Unknown
        });
    }
    if held.beyond().loaded() || held.offset != At::Bytes(0) {
        return Some(Fit::Unknown);
    }
    if held.may_be_a_locals_address() && held.inner_address {
        return Some(Fit::Unknown);
    }

    match folded(&extents, width) {
        Fit::Outside { extent, .. } => {
            let mut made: Option<Option<Span>> = None;
            for site in held.sites() {
                let here = match known.state[site] {
                    SiteState::Live(made) | SiteState::Freed { made, .. } => made,
                    SiteState::Unknown | SiteState::Reachable | SiteState::Unheld => None,
                };
                made = Some(match made {
                    Some(already) => same(already, here),
                    None => here,
                });
            }
            Some(Fit::Outside {
                extent,
                made: made.flatten(),
            })
        }
        fit => Some(fit),
    }
}

/// What every object a pointer may reach, by its extent, answers for an
/// access of `width` bytes: proved where every lower bound is at least
/// `width`, outside where every one is exact and below it, and unknown
/// otherwise, since a may-set proves nothing from one member. No allocation
/// is named; the caller knows which sites these were.
pub(super) fn folded(extents: &[Extent], width: u64) -> Fit {
    if extents.iter().all(|extent| extent.lower() >= width) {
        return Fit::Inside;
    }
    let exact: Option<Vec<u64>> = extents
        .iter()
        .map(|extent| match extent {
            Extent::Exactly(bytes) if *bytes < width => Some(*bytes),
            Extent::Exactly(_) | Extent::AtLeast(_) => None,
        })
        .collect();
    match exact {
        Some(bytes) => {
            // **One size to name only where every object agrees**, as one
            // allocation is named only where every site agrees: naming the
            // largest of two said `an object of 2` about a pointer that may
            // reach one of 1. Found by review.
            let extent = bytes
                .first()
                .copied()
                .filter(|first| bytes.iter().all(|other| other == first));
            Fit::Outside { extent, made: None }
        }
        None => Fit::Unknown,
    }
}

/// The locals whose value a pointer read `depth` dereferences below `local`
/// may be, and whether it is certainly the one local named, with no memory
/// beside it.
///
/// **The walk every reader of a load takes**, [`Known::levels_below`], one
/// level short of the pointer, so that what it was read from is what a load
/// would read: a local's address stored in an allocation, `*m = &v;`, is
/// found through `m` as it is through `pi = &v;`. Walking the address edges
/// alone left `***m = 1` believing `v` after `v = p + 4;`. Found by review.
pub(super) fn read_from_locals(
    function: &Function,
    known: &Known,
    local: LocalId,
    depth: usize,
) -> (Vec<LocalId>, bool) {
    let Some(above) = depth.checked_sub(1) else {
        return (Vec::new(), false);
    };
    let (sites, locals) = known.levels_below(local, above);
    let certain = sites.is_empty() && locals.len() == 1;
    // `LocalId` cannot be built from an index outside the IR, so the locals
    // are walked and kept where the walk names them.
    let locals = function
        .locals()
        .filter(|target| locals.contains(&LocalIdx::of(target)))
        .collect();
    (locals, certain)
}

/// What one local a pointer may have been read from answers for the access,
/// where it may also have been read from somewhere else.
///
/// **A proof only where the pointer is certainly that local's value.** A
/// may-set proves nothing from one member, as [`fits`] folds one: with `pi`
/// either `&v` or `&u`, `**pi` fits wherever `pi` is `&v`, so `u` being too
/// narrow is a doubt and not a proof. Found by review.
pub(super) fn only_if_certain(fit: Fit, certain: bool) -> Fit {
    match fit {
        Fit::Outside { .. } if !certain => Fit::Unknown,
        fit => fit,
    }
}

/// What the locals a pointer read through `place` may have been read from
/// answer for an access of `width` bytes through it, and the one local where
/// it is certainly that local's value, or `None` where it cannot be a local's.
///
/// **Asked where a pointer read out of memory is handed on**, as
/// [`may_not_fit`](super::report::may_not_fit) asks it where one is gone through: `put(*pi)` and
/// `memset(*pi, 0, 4)` hand on `v`'s value, which no store asked, as `**pi`
/// goes through it. Believing the type `*pi` is read as let both through
/// after `v` was given one `char`. Found by review.
pub(super) fn through_locals(
    analysis: &Allocations<'_>,
    function: &Function,
    place: &Place,
    width: u64,
    known: &Known,
) -> Option<(Fit, Option<LocalId>)> {
    let (locals, certain) = read_from_locals(function, known, place.local, derefs(place));
    let mut answer: Option<Fit> = None;
    for &local in &locals {
        // A local holding nothing is a null, which is the nullability
        // check's, and adds nothing here.
        let fit = match fits(analysis, function, local, width, known, false) {
            Some(fit) => only_if_certain(fit, certain),
            None => Fit::Inside,
        };
        answer = Some(match (answer, fit) {
            (_, Fit::Outside { extent, made }) => Fit::Outside { extent, made },
            (Some(Fit::Outside { extent, made }), _) => Fit::Outside { extent, made },
            (Some(Fit::Unknown), _) | (_, Fit::Unknown) => Fit::Unknown,
            (None | Some(Fit::Inside), Fit::Inside) => Fit::Inside,
        });
    }
    let named = match locals.as_slice() {
        [local] if certain => Some(*local),
        _ => None,
    };
    answer.map(|fit| (fit, named))
}

/// How many bytes one object of the type `ty` points to takes, where `ty`
/// is a pointer, and `None` for anything else.
///
/// **Zero for a `void *`, and not `None`.** A `void *` leaves believing no
/// width, but whoever converts it back believes it starts an object, so it is
/// asked that much: a width of zero makes [`fits`] answer from the start
/// alone. Answering `None` asked a `void *` nothing, and a pointer off its
/// start stored through a `void **` alias and read back through an `int **`
/// one was believed whole (#624).
///
/// **The same answer `Allocations::pointee_size` gives a load**, and on
/// purpose: what a pointer is asked where it leaves is what it is believed
/// where it arrives, so the two have to agree. See ADR-0060.
pub(super) fn pointee_width(analysis: &Allocations<'_>, ty: TyId) -> Option<u64> {
    match analysis.unit.ty(ty) {
        Ty::Pointer(pointee) => Some(analysis.unit.size_of(pointee).unwrap_or(0)),
        // An array is not a pointer, so nothing it points at has a width.
        Ty::Int | Ty::Char | Ty::Void | Ty::Array { .. } => None,
    }
}

/// The locals a value leaving this function may point at, where it is a
/// pointer to pointers: a local's address taken here, or one a local's value
/// may hold. Empty for a constant and for anything this check does not
/// follow to a local.
fn locals_reached(function: &Function, value: &Rvalue, known: &Known) -> Vec<LocalId> {
    match value {
        Rvalue::Address(place) if place.projection.is_empty() => vec![place.local],
        Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => {
            let held = &known.points_to[place.local];
            function
                .locals()
                .filter(|target| held.names_local(LocalIdx::of(target)))
                .collect()
        }
        Rvalue::Use(_) | Rvalue::Address(_) | Rvalue::Unary { .. } | Rvalue::Binary { .. } => {
            Vec::new()
        }
    }
}

/// What [`slots_from`] finds: each local with the type the receiver reads it
/// as, or `None` below a type that stops being a pointer; each site with the
/// type the receiver reads what it holds as; and each site it reached below
/// such a type, which a departure asks nothing of and a call marks (#679).
pub(super) type Slots = (Vec<(LocalId, Option<TyId>)>, Vec<(Site, TyId)>, Vec<Site>);

/// Every slot a pointer leaving as a pointer to `read_as` lets leave with it,
/// with the type the receiver reads each as: the locals it may point at, at
/// `read_as`, and so on down, with `None` below a type that stops being a
/// pointer; and the memory it may point at, at `read_as`, and so on down
/// while the receiver's type is a pointer, and below that only for the
/// locals it may hold.
///
/// **One walk for every question about what a call or a store lets leave**:
/// `report::contents_leaving` asks each slot whether it fits, and
/// `Allocations::terminator` takes from it the narrowest type a call was
/// handed each local as. Two walks would answer the two differently.
pub(super) fn slots_leaving(
    analysis: &Allocations<'_>,
    function: &Function,
    read_as: TyId,
    value: &Rvalue,
    known: &Known,
) -> Slots {
    // What the departing value points at, seeded as a load assigned first
    // would hold it: a local's own value names its sites and its address
    // edges; a pointer read out of memory and handed straight on, `use(*b)`,
    // names what the walk every load takes finds there, which handed
    // nothing on before (found by review); a local's address is
    // [`locals_reached`]'s; and what arithmetic builds is asked by
    // [`leaving`] and names nothing to walk.
    let (seed_sites, seed_locals): (Vec<Site>, Vec<LocalId>) = match value {
        Rvalue::Use(Operand::Copy(place)) if derefs(place) > 0 => {
            let (sites, locals) = known.levels_below(place.local, derefs(place));
            let locals = function
                .locals()
                .filter(|local| locals.contains(&LocalIdx::of(local)))
                .collect();
            (sites.into_iter().collect(), locals)
        }
        Rvalue::Use(Operand::Copy(place)) if place.projection.is_empty() => (
            known.points_to[place.local].sites().collect(),
            locals_reached(function, value, known),
        ),
        Rvalue::Use(_) | Rvalue::Address(_) | Rvalue::Unary { .. } | Rvalue::Binary { .. } => {
            (Vec::new(), locals_reached(function, value, known))
        }
    };
    let locals: Vec<(LocalId, Option<TyId>)> = seed_locals
        .into_iter()
        .map(|local| (local, Some(read_as)))
        .collect();
    let sites: Vec<(Site, Option<TyId>)> = seed_sites
        .into_iter()
        .map(|site| (site, Some(read_as)))
        .collect();
    slots_from(analysis, function, known, locals, sites)
}

/// The walk [`slots_leaving`] takes, from slots already seeded with the type
/// the receiver reads each as: what each local and site may point at, and so
/// on down, each read as the type below the one it was reached at.
///
/// **Seeded directly as well**, at `None`, by a call that walks below what an
/// earlier call was handed untyped (`Known::handed_untyped`, #679), so the
/// two read memory the same way.
pub(super) fn slots_from(
    analysis: &Allocations<'_>,
    function: &Function,
    known: &Known,
    mut locals: Vec<(LocalId, Option<TyId>)>,
    mut sites: Vec<(Site, Option<TyId>)>,
) -> Slots {
    let mut asked_locals: Vec<(LocalId, Option<TyId>)> = Vec::new();
    let mut asked_sites: Vec<(Site, TyId)> = Vec::new();
    let mut untyped_sites: Vec<Site> = Vec::new();
    // Each pair is expanded once, behind `asked_locals` and `asked_sites`,
    // and there are finitely many: the locals, the sites, and the types the
    // departing one points through. So the walk ends.
    loop {
        if let Some((local, read_as)) = locals.pop() {
            if asked_locals.contains(&(local, read_as)) {
                continue;
            }
            asked_locals.push((local, read_as));
            let below = read_as.and_then(|ty| pointee(analysis, ty));
            // What it points at is walked too, since a `void *` that
            // believes nothing may still point at a local that holds
            // something believed.
            let read = Rvalue::Use(Operand::Copy(Place::local(local)));
            locals.extend(
                locals_reached(function, &read, known)
                    .into_iter()
                    .map(|next| (next, below)),
            );
            sites.extend(known.points_to[local].sites().map(|site| (site, below)));
        } else if let Some((site, read_as)) = sites.pop() {
            // Below a type that is not a pointer the receiver believes
            // nothing of this memory, and its stores were asked at theirs.
            // **But the locals it may hold are still reached** (#677): a
            // callee handed `&c` as bytes, with `c` an allocation holding
            // `&w`, may store any pointer into `w`, and `use1(w)` was
            // silent. Each such site is expanded once, behind
            // `untyped_sites`, so the walk still ends.
            let Some(read_as) = read_as else {
                if !untyped_sites.contains(&site) {
                    untyped_sites.push(site);
                    sites.extend(known.contents[site].sites().map(|next| (next, None)));
                    locals.extend(
                        function
                            .locals()
                            .filter(|local| known.contents[site].names_local(LocalIdx::of(local)))
                            .map(|next| (next, None)),
                    );
                }
                continue;
            };
            if asked_sites.contains(&(site, read_as)) {
                continue;
            }
            asked_sites.push((site, read_as));
            let below = pointee(analysis, read_as);
            sites.extend(known.contents[site].sites().map(|next| (next, below)));
            locals.extend(
                function
                    .locals()
                    .filter(|local| known.contents[site].names_local(LocalIdx::of(local)))
                    .map(|next| (next, below)),
            );
        } else {
            break;
        }
    }
    (asked_locals, asked_sites, untyped_sites)
}

/// The type `ty` points to, where it is a pointer or an array of them, and
/// `None` otherwise.
///
/// **Through an array to its element**, as the call's transfer reads one:
/// `put(&a)` with `a` an `int **[1]` holding `&w` and `put` taking
/// `int **(*)[1]` reached `w` below a type that stopped being a pointer, and
/// a well-typed out-parameter was doubted once such a local is believed
/// nothing (#677). Found by review.
pub(super) fn pointee(analysis: &Allocations<'_>, ty: TyId) -> Option<TyId> {
    match analysis.unit.ty(innermost_element(analysis.unit, ty)) {
        Ty::Pointer(pointee) => Some(pointee),
        Ty::Int | Ty::Char | Ty::Void => None,
        Ty::Array { .. } => unreachable!("`innermost_element` strips every array"),
    }
}

/// [`fits`], asked of values no program isolates.
#[cfg(test)]
mod tests {
    use super::{Fit, fits};
    use crate::dataflow::Analysis;
    use crate::ir::{Block, Function, Terminator, TranslationUnit, Ty};
    use crate::memory::index::LocalIdx;
    use crate::source::{SourceMap, Span};
    use crate::target::Target;

    /// A function with a `char` `c`, an `int [2]` `a` and an `int *` `p`,
    /// whose only block returns, and the analysis of it.
    fn asked(
        check: impl FnOnce(
            &crate::memory::allocations::Allocations<'_>,
            &Function,
            [crate::ir::LocalId; 3],
        ),
    ) {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "f");
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let char = unit.push_type(Ty::Char);
        let pair = unit.push_type(Ty::Array {
            element: int,
            length: 2,
        });
        let ptr = unit.push_type(Ty::Pointer(int));
        let mut function = Function::new(Span::new(file, 0, 1), int, []);
        let c = function.push_local(char);
        let a = function.push_local(pair);
        let p = function.push_local(ptr);
        let block = function.reserve_block();
        function.fill_block(
            block,
            Block {
                elements: vec![],
                terminator: Terminator::Return,
            },
        );
        let func = unit.push_function(function);
        let analysis = crate::memory::analysis(&sources, &unit, func);
        check(&analysis, unit.function(func), [c, a, p]);
    }

    /// An edge that may be the address of something inside a local is not
    /// read at that local's size: four bytes through `&a[1]` are not shown
    /// to fit, though `a` is eight. The same edge to the whole of `a` fits.
    ///
    /// **No program isolates it**: `&a[1]` handed on is doubted by another
    /// rule first, measured with `memset(&a[1], 0, 8)`.
    ///
    /// Mutation: drop the `inner_address` test from `fits`. The first
    /// assertion fails.
    #[test]
    fn an_edge_inside_a_local_is_not_read_at_the_local_s_size() {
        asked(|analysis, function, [_, a, p]| {
            let mut known = analysis.on_entry();
            let held = &mut known.points_to[p];
            held.name_local(LocalIdx::of(a));
            held.writes_elsewhere = false;
            held.inner_address = true;
            assert!(matches!(
                fits(analysis, function, p, 4, &known, false),
                Some(Fit::Unknown)
            ));
            known.points_to[p].inner_address = false;
            assert!(matches!(
                fits(analysis, function, p, 4, &known, false),
                Some(Fit::Inside)
            ));
        });
    }

    /// A belief is no proof where what the pointer is recorded to name is
    /// narrower: believed four bytes wide and naming a one-byte `char`, it
    /// is not shown to fit; naming the eight-byte `a`, it is.
    ///
    /// **No program isolates it**: a load is believed no wider than what its
    /// memory was recorded to hold (ADR-0061), so the belief that arrives
    /// here is already the narrower one.
    ///
    /// Mutation: drop `&& recorded` from the belief's answer in `fits`. The
    /// first assertion fails.
    #[test]
    fn a_belief_is_no_wider_than_what_the_pointer_is_recorded_to_name() {
        asked(|analysis, function, [c, a, p]| {
            let mut known = analysis.on_entry();
            let held = &mut known.points_to[p];
            held.name_local(LocalIdx::of(c));
            held.writes_elsewhere = false;
            held.read_out_of_memory();
            held.believe(Some(4));
            assert!(matches!(
                fits(analysis, function, p, 4, &known, false),
                Some(Fit::Unknown)
            ));
            let held = &mut known.points_to[p];
            held.unname_local(LocalIdx::of(c));
            held.name_local(LocalIdx::of(a));
            assert!(matches!(
                fits(analysis, function, p, 4, &known, false),
                Some(Fit::Inside)
            ));
        });
    }
}
