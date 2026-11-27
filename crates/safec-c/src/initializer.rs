//! What a braced initializer initializes: C17 6.7.9 p17 to p21 worked out
//! against the type it initializes, into the scalars and the aggregates
//! taken whole that its items write, each with its path from the object.
//!
//! **One walk, asked by both its readers**: the type check holds each item
//! to what it initializes (6.5.16.1 by way of p11), and the lowering writes
//! it there. Two walks would be two answers to which member `{1, 2}` puts
//! the `2` in, and the one that drifted would build a program that does not
//! do what the checked one says. See ADR-0137.

use std::collections::HashSet;

use crate::ast::{Ast, Declaration, Designator, ExprId, Initializer, Type, TypeId};
use crate::sema::Resolution;
use safec_ir::source::{SourceMap, Span};

/// Whether `value`, of type `of`, initializes an aggregate of `ty` whole
/// rather than its first scalar: a struct of a compatible type (C17 6.7.9
/// p13), or a string literal for an array of character type (p14). One
/// answer for the type check and the lowering, as the walk is.
pub fn initializes_whole(
    ast: &Ast,
    resolution: &Resolution,
    of: Option<TypeId>,
    value: ExprId,
    ty: TypeId,
) -> bool {
    match ast.ty(ty) {
        Type::Struct { .. } => of.is_some_and(|of| {
            matches!(ast.ty(of), Type::Struct { .. })
                && ast.compatible_unqualified(of, ty, &|a, b| resolution.same_tag(a, b))
        }),
        Type::Array { element, .. } => {
            matches!(ast.expr(value), crate::ast::Expr::String { .. })
                && matches!(
                    ast.ty(ast.unqualified(*element)),
                    Type::Integer {
                        rank: safec_ir::ir::Rank::Char,
                        ..
                    }
                )
        }
        _ => false,
    }
}

/// One step from an object to a subobject of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// The member at this index among its struct's members in the order
    /// written, as `types::Member::path` counts them.
    Member(usize),
    /// The element at this index.
    Element(u64),
}

/// What one item of an initializer writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    /// The subobject, from the object initialized.
    pub path: Vec<Step>,
    /// The subobject's type, as its declaration wrote it.
    pub ty: TypeId,
    /// What it is initialized with.
    pub value: ExprId,
    /// Whether the subobject is an aggregate the item initializes whole: a
    /// struct from an expression of its type (p13), or an array of
    /// character type from a string literal (p14). Otherwise it is a scalar.
    pub whole: bool,
}

/// Why an initializer could not be worked out, each a span to point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// More items than the object has subobjects to take them, which p2
    /// forbids.
    TooMany(Span),
    /// `{}`, which C17's grammar has no form of, and which C23 gives a
    /// meaning this compiler does not build yet.
    Empty(Span),
    /// A subobject whose element count this stage cannot say: an array of
    /// unknown size the parser could not complete (`Parser::completed`), or
    /// one of a struct this place cannot see the definition of.
    Unsized(Span),
    /// A designator that names no subobject of the current object: a
    /// member it has none of, an index that is no integer constant or is
    /// past the array's end, or either for an object of the wrong kind
    /// (p6, p7).
    Designator(Span),
}

/// What an object is made of, for initializing it.
enum Kind {
    /// No subobjects: one item initializes it.
    Scalar,
    /// `count` elements of `element`.
    Array { element: TypeId, count: u64 },
    /// The members an initializer reaches, in order, each with its index
    /// among the struct's members: every named member and every anonymous
    /// one, but no unnamed bit-field (p9) and no flexible array member
    /// (6.7.2.1 p18); for a union, only the first of them (p17).
    Struct(Vec<(usize, TypeId)>),
}

/// The walk, with what it reads of the program.
#[allow(missing_debug_implementations)] // Two closures, which have no `Debug`.
pub struct Shapes<'a> {
    /// The source, for a designator's member name.
    pub sources: &'a SourceMap,
    /// The tree.
    pub ast: &'a Ast,
    /// Where each struct is defined.
    pub resolution: &'a Resolution,
    /// The value of an array's length, where it is a constant.
    pub length: &'a dyn Fn(ExprId) -> Option<u64>,
    /// Whether the expression initializes an aggregate of this type whole:
    /// a struct of a compatible type, or a string literal for an array of
    /// character type.
    pub whole: &'a dyn Fn(ExprId, TypeId) -> bool,
}

impl Shapes<'_> {
    /// The leaves `init` writes in an object of `ty`, in the order written,
    /// each that a later one wholly covers left out: a later initializer
    /// for a subobject overrides an earlier one (p19), and the earlier is
    /// not evaluated, as `clang` builds it. What is left is disjoint where
    /// one does not hold another, so writing them in order is writing what
    /// C gives.
    ///
    /// Mutation: in `Shapes::place`, take an expression item for an
    /// aggregate subobject whole rather than its first scalar; the run of
    /// `a_braced_initializer_fills_arrays_and_structs`, whose lists elide
    /// braces, fails. Mutation: keep every leaf; the run of
    /// `a_designated_initializer_names_its_subobject` reads an overridden
    /// value.
    pub fn resolve(&self, ty: TypeId, init: &Initializer) -> Result<Vec<Leaf>, Fault> {
        let mut leaves = Vec::new();
        self.object(ty, init, &mut Vec::new(), &mut leaves)?;
        // From the last: a leaf is kept unless a later one's path is a
        // prefix of its own. A set of the paths seen, so each leaf asks its
        // own prefixes and not every later leaf.
        let mut seen: HashSet<Vec<Step>> = HashSet::new();
        let mut kept = Vec::with_capacity(leaves.len());
        for leaf in leaves.into_iter().rev() {
            let covered = (0..=leaf.path.len()).any(|len| seen.contains(&leaf.path[..len]));
            seen.insert(leaf.path.clone());
            if !covered {
                kept.push(leaf);
            }
        }
        kept.reverse();
        Ok(kept)
    }

    /// Every scalar of an object of `ty`, in order, with its path and type:
    /// what an initializer that does not name them all sets to zero
    /// (p21). `None` past `limit` of them, or where one's count is unknown.
    pub fn scalars(&self, ty: TypeId, limit: usize) -> Option<Vec<(Vec<Step>, TypeId)>> {
        // **Counted first, by arithmetic over the type**, so that the walk
        // below never visits more than `limit` scalars, and skips an array
        // of elements with none: a struct of a billion empty ones walked
        // element by element took minutes (#1241 review).
        if self.count(ty)? > limit as u64 {
            return None;
        }
        let mut found = Vec::new();
        let mut work = vec![(Vec::new(), ty)];
        while let Some((path, ty)) = work.pop() {
            match self.kind(ty)? {
                Kind::Scalar => {
                    found.push((path, ty));
                    if found.len() > limit {
                        return None;
                    }
                }
                Kind::Array { element, count } => {
                    if self.count(element) == Some(0) {
                        continue;
                    }
                    for index in (0..count).rev() {
                        let mut inner = path.clone();
                        inner.push(Step::Element(index));
                        work.push((inner, element));
                    }
                }
                Kind::Struct(members) => {
                    for &(index, member) in members.iter().rev() {
                        let mut inner = path.clone();
                        inner.push(Step::Member(index));
                        work.push((inner, member));
                    }
                }
            }
        }
        Some(found)
    }

    /// How many scalars an object of `ty` holds, a union counting its first
    /// member's; `None` where that is unknown or past `u64`.
    pub fn count(&self, ty: TypeId) -> Option<u64> {
        match self.kind(ty)? {
            Kind::Scalar => Some(1),
            Kind::Array { element, count } => self.count(element)?.checked_mul(count),
            Kind::Struct(members) => members.iter().try_fold(0u64, |sum, &(_, member)| {
                sum.checked_add(self.count(member)?)
            }),
        }
    }

    /// The type of the subobject `step` reaches in an object of `ty`.
    /// Every member, a union's past its first included, since a designator
    /// reaches any of them.
    pub fn subobject(&self, ty: TypeId, step: Step) -> Option<TypeId> {
        match (self.ast.ty(ty), step) {
            (Type::Array { element, .. }, Step::Element(_)) => Some(*element),
            (Type::Struct { .. }, Step::Member(index)) => self
                .members(ty)?
                .1
                .into_iter()
                .find(|&(at, _)| at == index)
                .map(|(_, member)| member),
            _ => None,
        }
    }

    /// Whether an object of `ty` holds a union, anywhere in it, whose first
    /// member is smaller than the union, by `size`: one an initializer
    /// writing its first member leaves bytes of unwritten.
    pub fn holds_short_union(
        &self,
        ty: TypeId,
        size: &mut dyn FnMut(TypeId) -> Option<u64>,
    ) -> bool {
        let mut work = vec![ty];
        let mut seen = 0usize;
        while let Some(ty) = work.pop() {
            // Each type once per path is enough to answer; the bound keeps
            // a pathological nesting from running long.
            seen += 1;
            if seen > 1 << 16 {
                return true;
            }
            match self.ast.ty(ty) {
                Type::Array { element, .. } => work.push(*element),
                Type::Struct { .. } => {
                    let Some((union, members)) = self.members(ty) else {
                        continue;
                    };
                    if union
                        && members
                            .first()
                            .is_some_and(|&(_, first)| size(first) != size(ty))
                    {
                        return true;
                    }
                    work.extend(members.iter().map(|&(_, member)| member));
                }
                _ => {}
            }
        }
        false
    }

    /// Whether `ty` is a union.
    pub fn union(&self, ty: TypeId) -> bool {
        matches!(self.ast.ty(ty), Type::Struct { union: true, .. })
    }

    /// The object of `ty` at `path`, initialized from `init`.
    fn object(
        &self,
        ty: TypeId,
        init: &Initializer,
        path: &mut Vec<Step>,
        leaves: &mut Vec<Leaf>,
    ) -> Result<(), Fault> {
        let (items, span) = match init {
            Initializer::Expr(value) => {
                leaves.push(Leaf {
                    path: path.clone(),
                    ty,
                    value: *value,
                    whole: !matches!(self.kind(ty), Some(Kind::Scalar)),
                });
                return Ok(());
            }
            Initializer::List { items, span } => (items, *span),
            // Only ever a list's item, which `Shapes::list` takes apart.
            Initializer::Designated { span, .. } => return Err(Fault::Designator(*span)),
        };
        match self.kind(ty).ok_or(Fault::Unsized(span))? {
            // p11: a scalar's initializer is one expression, optionally
            // in braces; a designator names nothing in one (p7).
            Kind::Scalar => match items.as_slice() {
                [] => Err(Fault::Empty(span)),
                [Initializer::Designated { span, .. }] => Err(Fault::Designator(*span)),
                [one] => self.object(ty, one, path, leaves),
                [_, extra, ..] => Err(Fault::TooMany(extra.span(self.ast))),
            },
            Kind::Array { .. } | Kind::Struct(_) => {
                // p14: a string literal for an array of character type,
                // "optionally enclosed in braces", at any depth, so
                // `{{"ab"}, {"c"}}` gives each row its literal (#1241
                // review). The parser drops the braces of a declarator's
                // own, so its length counts the literal.
                if let [Initializer::Expr(value)] = items.as_slice() {
                    if (self.whole)(*value, ty)
                        && matches!(self.ast.expr(*value), crate::ast::Expr::String { .. })
                    {
                        leaves.push(Leaf {
                            path: path.clone(),
                            ty,
                            value: *value,
                            whole: true,
                        });
                        return Ok(());
                    }
                }
                self.list(ty, items, span, path, false, leaves).map(|_| ())
            }
        }
    }

    /// The braced list `items` for the aggregate of `ty` at `path`, item by
    /// item (p17 to p20). The current object is a stack of frames, the
    /// list's own at the bottom: a designation starts again from it and
    /// descends to what it names; an item without one goes to the next
    /// subobject of the deepest frame, a frame that has run out giving way
    /// to the one below it; and an expression for an aggregate it does not
    /// take whole pushes that aggregate's frame and goes to its first
    /// subobject (p20). `unbounded` lets the list's own array grow to what
    /// the items reach, for `Shapes::extent`. The elements the list's own
    /// array is given, one past the last reached.
    ///
    /// A recursion through `Shapes::object` for each braced item, bounded
    /// by how deep the parser lets a list nest; a loop otherwise.
    fn list(
        &self,
        ty: TypeId,
        items: &[Initializer],
        span: Span,
        path: &[Step],
        unbounded: bool,
        leaves: &mut Vec<Leaf>,
    ) -> Result<u64, Fault> {
        if items.is_empty() {
            return Err(Fault::Empty(span));
        }
        let mut stack = vec![
            self.frame(ty, path.to_vec(), unbounded)
                .ok_or(Fault::Unsized(span))?,
        ];
        let mut reached = 0u64;
        for item in items {
            let at = item.span(self.ast);
            let init = match item {
                Initializer::Designated {
                    designation, init, ..
                } => {
                    stack.truncate(1);
                    self.designate(&mut stack, designation, at)?;
                    &**init
                }
                _ => {
                    // The next subobject, past every frame that has run out.
                    while self
                        .next(stack.last().expect("the list's frame stays"))
                        .is_none()
                    {
                        if stack.len() == 1 {
                            return Err(Fault::TooMany(at));
                        }
                        stack.pop();
                        stack.last_mut().expect("one is left").advance();
                    }
                    item
                }
            };
            self.place(&mut stack, init, at, leaves, &mut reached)?;
        }
        Ok(reached)
    }

    /// Move `stack` to the subobject `designation` names, from the list's
    /// own frame: each designator but the last a frame of its own, the last
    /// the next subobject of the deepest one (p17's "current object").
    fn designate(
        &self,
        stack: &mut Vec<Frame>,
        designation: &[Designator],
        at: Span,
    ) -> Result<(), Fault> {
        for (index, designator) in designation.iter().enumerate() {
            let frame = stack.last_mut().expect("the list's frame stays");
            match (designator, &frame.subs) {
                (Designator::Index(value), Subs::Array { count, .. }) => {
                    let position = (self.length)(*value).ok_or(Fault::Designator(at))?;
                    if count.is_some_and(|count| position >= count) {
                        return Err(Fault::Designator(at));
                    }
                    frame.next = position;
                }
                (Designator::Member(name), Subs::Members { .. }) => {
                    let name = self.sources.snippet(*name);
                    let chain = self.find(frame.ty, name).ok_or(Fault::Designator(at))?;
                    // Through each anonymous member on the way, a frame each.
                    let (last, through) = chain.split_last().expect("a found member has a place");
                    for &position in through {
                        let frame = stack.last_mut().expect("the list's frame stays");
                        frame.select(position);
                        let (step, sub) = self.next(frame).ok_or(Fault::Designator(at))?;
                        let mut path = frame.path.clone();
                        path.push(step);
                        stack.push(self.frame(sub, path, false).ok_or(Fault::Designator(at))?);
                    }
                    stack
                        .last_mut()
                        .expect("the list's frame stays")
                        .select(*last);
                }
                _ => return Err(Fault::Designator(at)),
            }
            if index + 1 < designation.len() {
                let frame = stack.last().expect("the list's frame stays");
                let (step, sub) = self.next(frame).ok_or(Fault::Designator(at))?;
                let mut path = frame.path.clone();
                path.push(step);
                stack.push(self.frame(sub, path, false).ok_or(Fault::Designator(at))?);
            }
        }
        Ok(())
    }

    /// Initialize the next subobject of the deepest frame from `init`, and
    /// move past it: a list or an expression that takes it whole is that
    /// subobject's; an expression for an aggregate it does not take whole
    /// goes to the aggregate's first scalar, its frame pushed (p20).
    /// `reached` is one past the last element of the list's own array an
    /// item went to.
    fn place(
        &self,
        stack: &mut Vec<Frame>,
        init: &Initializer,
        at: Span,
        leaves: &mut Vec<Leaf>,
        reached: &mut u64,
    ) -> Result<(), Fault> {
        loop {
            let frame = stack.last().expect("the list's frame stays");
            let Some((step, sub)) = self.next(frame) else {
                // An aggregate with nothing an initializer reaches, entered
                // by elision: the item goes past it.
                if stack.len() == 1 {
                    return Err(Fault::TooMany(at));
                }
                stack.pop();
                stack.last_mut().expect("one is left").advance();
                continue;
            };
            let mut path = frame.path.clone();
            path.push(step);
            if let Subs::Array { .. } = stack[0].subs {
                *reached = (*reached).max(stack[0].next + 1);
            }
            match init {
                Initializer::Expr(value) => {
                    let scalar = matches!(self.kind(sub).ok_or(Fault::Unsized(at))?, Kind::Scalar);
                    if scalar || (self.whole)(*value, sub) {
                        leaves.push(Leaf {
                            path,
                            ty: sub,
                            value: *value,
                            whole: !scalar,
                        });
                        stack.last_mut().expect("the list's frame stays").advance();
                        return Ok(());
                    }
                    stack.push(self.frame(sub, path, false).ok_or(Fault::Unsized(at))?);
                }
                Initializer::List { .. } | Initializer::Designated { .. } => {
                    self.object(sub, init, &mut path, leaves)?;
                    stack.last_mut().expect("the list's frame stays").advance();
                    return Ok(());
                }
            }
        }
    }

    /// The frame of the aggregate of `ty` at `path`, its first subobject
    /// next; an array's count `None` where it is `unbounded`. `None` for a
    /// scalar, or where a count or a definition is not to be had.
    fn frame(&self, ty: TypeId, path: Vec<Step>, unbounded: bool) -> Option<Frame> {
        let subs = match self.ast.ty(ty) {
            Type::Array {
                element, length, ..
            } => Subs::Array {
                element: *element,
                count: if unbounded {
                    None
                } else {
                    Some((self.length)((*length)?)?)
                },
            },
            Type::Struct { .. } => {
                let (union, members) = self.members(ty)?;
                Subs::Members { members, union }
            }
            _ => return None,
        };
        Some(Frame {
            ty,
            path,
            subs,
            next: 0,
            done: false,
        })
    }

    /// The step to and the type of `frame`'s next subobject, or `None`
    /// where it has run out: past the array's end, past the last member,
    /// or a union once one member is initialized.
    fn next(&self, frame: &Frame) -> Option<(Step, TypeId)> {
        match &frame.subs {
            Subs::Array { element, count } => count
                .is_none_or(|count| frame.next < count)
                .then_some((Step::Element(frame.next), *element)),
            Subs::Members { members, .. } => {
                if frame.done {
                    return None;
                }
                let &(index, member) = members.get(usize::try_from(frame.next).ok()?)?;
                Some((Step::Member(index), member))
            }
        }
    }

    /// Where the member spelled `name` is among the members of the struct
    /// `ty` an initializer reaches: its position, or an anonymous member's
    /// followed by the position within it, as `types::Checker::find_member`
    /// finds one for `.`. A recursion, bounded by how deep the parser nests
    /// a member list.
    fn find(&self, ty: TypeId, name: &str) -> Option<Vec<u64>> {
        let declared = self.declared(ty)?;
        let (_, members) = self.members(ty)?;
        for (position, &(index, member)) in members.iter().enumerate() {
            let position = position as u64;
            match declared[index].name {
                Some(spelled) if self.sources.snippet(spelled) == name => {
                    return Some(vec![position]);
                }
                Some(_) => {}
                None => {
                    if let Some(mut within) = self.find(member, name) {
                        within.insert(0, position);
                        return Some(within);
                    }
                }
            }
        }
        None
    }

    /// How many elements of `element` the list `init` initializes, which is
    /// the length of an array of unknown size it is the initializer of (C17
    /// 6.7.9 p22): one past the last element an item reaches, by position
    /// or by designator, brace elision included. Asked by the type check
    /// once every item is typed, since a struct taken whole is one item
    /// where its members elided would be several, and a designator's index
    /// is a value. See `Parser::completed`.
    ///
    /// Mutation: count every item as an element; the run of
    /// `a_braced_initializer_fills_arrays_and_structs` sizes `elided`
    /// wrongly.
    pub fn extent(&self, array: TypeId, init: &Initializer) -> Result<u64, Fault> {
        let Initializer::List { items, span } = init else {
            return Err(Fault::Unsized(init.span(self.ast)));
        };
        self.list(array, items, *span, &[], true, &mut Vec::new())
    }

    /// The members of the struct `ty` as its definition declares them.
    fn declared(&self, ty: TypeId) -> Option<&[Declaration]> {
        match self.ast.ty(ty) {
            Type::Struct {
                members: Some(members),
                ..
            } => Some(members),
            Type::Struct { members: None, .. } => {
                let bound = self.resolution.bound(self.ast, ty);
                let definition = self.resolution.definition(self.resolution.tag(bound))?;
                match self.ast.ty(definition) {
                    Type::Struct {
                        members: Some(members),
                        ..
                    } => Some(members),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether the struct `ty` is a union, and every member an initializer
    /// can reach, each with its index among the members written: every
    /// named member and every anonymous one, but no unnamed bit-field (p9)
    /// and no flexible array member (6.7.2.1 p18). `None` where its
    /// definition is not to be had.
    fn members(&self, ty: TypeId) -> Option<(bool, Vec<(usize, TypeId)>)> {
        let Type::Struct { union, .. } = self.ast.ty(ty) else {
            return None;
        };
        let members = self.declared(ty)?;
        let last = members.len().checked_sub(1);
        let reached = members
            .iter()
            .enumerate()
            .filter(|&(index, member)| {
                let unnamed_bits = member.name.is_none() && member.width.is_some();
                let flexible = Some(index) == last
                    && matches!(self.ast.ty(member.ty), Type::Array { length: None, .. });
                !unnamed_bits && !flexible
            })
            .map(|(index, member)| (index, member.ty))
            .collect();
        Some((*union, reached))
    }

    /// What an object of `ty` is made of, or `None` where an array's count
    /// is unknown or a struct's definition is not to be had.
    fn kind(&self, ty: TypeId) -> Option<Kind> {
        match self.ast.ty(ty) {
            Type::Array {
                element, length, ..
            } => Some(Kind::Array {
                element: *element,
                count: (self.length)((*length)?)?,
            }),
            Type::Struct { .. } => {
                let (union, mut reached) = self.members(ty)?;
                if union {
                    reached.truncate(1);
                }
                Some(Kind::Struct(reached))
            }
            Type::Void
            | Type::VaList
            | Type::Integer { .. }
            | Type::Pointer(_)
            | Type::Function { .. } => Some(Kind::Scalar),
        }
    }
}

/// One aggregate a list is filling, and where in it the next item goes.
struct Frame {
    /// Its type.
    ty: TypeId,
    /// Where it is in the object the list initializes.
    path: Vec<Step>,
    /// What it is made of.
    subs: Subs,
    /// The next element, or the position of the next member among
    /// [`Subs::Members`].
    next: u64,
    /// For a union, whether one member has been initialized, after which
    /// it has run out (p17: a union's list initializes one member).
    done: bool,
}

impl Frame {
    /// Past the subobject just initialized.
    fn advance(&mut self) {
        self.next += 1;
        if matches!(self.subs, Subs::Members { union: true, .. }) {
            self.done = true;
        }
    }

    /// The member at `position` next, as a designator names it: a union's
    /// may be any of its members.
    fn select(&mut self, position: u64) {
        self.next = position;
        self.done = false;
    }
}

/// What a frame's aggregate is made of.
enum Subs {
    /// Elements of `element`, `count` of them or as many as items reach.
    Array { element: TypeId, count: Option<u64> },
    /// The members an initializer reaches, each with its index among the
    /// struct's members, and whether it is a union.
    Members {
        members: Vec<(usize, TypeId)>,
        union: bool,
    },
}
