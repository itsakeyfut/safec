//! The graph, as something that walks every path at once needs it.
//!
//! [`crate::interp`] walks one path and answers what a program did with the
//! values it was given. An analysis walks every path at once and answers what is
//! true whichever way control went, which needs the edges backwards, an order to
//! take the blocks in, and a way to leave out the blocks that never run. The two
//! read the same [`Function`] and share nothing else.
//!
//! Nothing here reports anything. `docs/roadmap.md` makes Phase 5 the first
//! phase that says something about a C program, and a graph that concluded
//! would be one.

use crate::ir::{BlockId, Function};

/// What a walk over one function's graph needs, worked out once.
///
/// Three questions and one traversal. A depth-first walk from the entry visits
/// exactly the blocks the entry can reach, so the order it produces *is* the
/// reachable set, and what answers "can control get here" is read off that
/// order rather than worked out by a second walk that could disagree with it.
///
/// Built per function and thrown away: the IR does not change while a pass runs
/// over it, and a `Cfg` that outlived one would be a claim about a function that
/// somebody may since have filled in.
#[derive(Debug)]
pub struct Cfg {
    /// Every block the entry reaches, in an order where a block comes after the
    /// blocks that reach it wherever the graph allows one.
    order: Vec<BlockId>,
    /// Whether the entry reaches each block, indexed by [`BlockId::index`].
    ///
    /// Filled from `order` rather than by a second walk, so the two cannot
    /// disagree about which blocks run: there is one traversal and one place
    /// that decides.
    ///
    /// A `bool` rather than where the block sits in `order`, which is the other
    /// thing a walk could leave here. Nothing reads a position, and a number
    /// nobody reads is a number nobody notices is wrong: one was here, and
    /// setting every block's to zero failed no test.
    reachable: Vec<bool>,
    /// Where control can arrive from, per block, indexed by [`BlockId::index`].
    ///
    /// **Reachable blocks only, on both sides.** A block the entry never
    /// reaches is nobody's predecessor, even where it has an edge out, and has
    /// no predecessors of its own. An analysis joins what arrives at a block,
    /// and a value from a block that never runs is not a value: including one
    /// would make every caller filter, and the first that forgot would join a
    /// bottom into a fixpoint and answer that nothing is known anywhere.
    predecessors: Vec<Vec<BlockId>>,
    /// The order [`crate::dataflow::solve`] drains its worklist in: every loop,
    /// nested ones included, before anything after it.
    ///
    /// Each loop's blocks are contiguous and start with its head, the loops and
    /// the blocks outside them come in topological order, and inside a loop the
    /// same holds for what is left once its head is taken out. `order` alone
    /// does not give that:
    /// a reverse postorder puts a loop's head before its body and before its
    /// exit, and says nothing about which of those two comes first, so a later
    /// loop can come before an earlier loop's body. A walk taking blocks in
    /// that order walks the later loop again for every step the earlier one
    /// takes. See ADR-0016.
    drain: Vec<BlockId>,
    /// Where each block sits in `drain`, indexed by [`BlockId::index`].
    ///
    /// Read by the solver to keep its worklist in drain order. A block the
    /// entry does not reach has no position and its entry is never read.
    drained_at: Vec<usize>,
}

impl Cfg {
    /// Walk one function's graph.
    ///
    /// **Iteratively, and that is not a preference.** The obvious depth-first
    /// search is recursive and is bounded by how deep the graph is, which is
    /// bounded by nothing: `MAX_NESTING` bounds what the parser accepts and says
    /// in its own doc comment that this is not the same as bounding the tree.
    /// A chain of blocks long enough to overflow a stack is a chain of
    /// statements, and the answer to that must not be a process that dies
    /// without a diagnostic.
    ///
    /// A block already walked is not walked again, which is what makes this
    /// end at all: a loop's back edge would otherwise be followed forever.
    /// Removing that check does not fail a test, it hangs one, which is a
    /// worse signal than a failure and the only one available for a
    /// non-termination.
    ///
    /// # Panics
    ///
    /// If a block was reserved and never filled, which is [`Function::block`]'s
    /// panic rather than one invented here: a half-built function has no graph
    /// to walk, and answering anything at all for one would be answering about
    /// a program that does not exist yet.
    pub fn of(function: &Function) -> Self {
        let count = function.blocks().len();

        // Depth-first, keeping each block on the stack until its successors are
        // done, which is what makes the order a postorder rather than the order
        // they were reached in. The `usize` beside each block is how many of its
        // successors have been taken care of.
        let mut postorder = Vec::with_capacity(count);
        let mut seen = vec![false; count];
        let mut stack = vec![(function.entry(), 0usize)];
        let mut successors = Vec::new();

        seen[function.entry().index()] = true;
        while let Some((block, next)) = stack.pop() {
            successors.clear();
            function.block(block).terminator.successors(&mut successors);

            match successors.get(next) {
                Some(&successor) => {
                    stack.push((block, next + 1));
                    if !seen[successor.index()] {
                        seen[successor.index()] = true;
                        stack.push((successor, 0));
                    }
                }
                // Every way out of this block has been walked, so everything
                // this block reaches is already in the order and it goes after
                // them.
                None => postorder.push(block),
            }
        }

        let order: Vec<BlockId> = postorder.into_iter().rev().collect();

        let mut reachable = vec![false; count];
        for &block in &order {
            reachable[block.index()] = true;
        }

        // From `order` rather than from every block, which is what keeps an
        // unreachable predecessor out of the answer. A block that arrives twice
        // is here twice: `Branch { then: b, otherwise: b }` is two edges, which
        // is what the graph says, and a join is idempotent so nothing
        // downstream reads a different answer for it.
        let mut predecessors = vec![Vec::new(); count];
        for &block in &order {
            successors.clear();
            function.block(block).terminator.successors(&mut successors);

            for &successor in &successors {
                predecessors[successor.index()].push(block);
            }
        }

        let drain = components(function, &order);
        let mut drained_at = vec![usize::MAX; count];
        for (at, &block) in drain.iter().enumerate() {
            drained_at[block.index()] = at;
        }

        Self {
            order,
            reachable,
            predecessors,
            drain,
            drained_at,
        }
    }

    /// The order to take the blocks in, which holds exactly the reachable ones.
    pub fn order(&self) -> &[BlockId] {
        &self.order
    }

    /// The reachable blocks in the order a forward walk should settle them:
    /// a loop before anything after it, and otherwise as [`Cfg::order`] has
    /// them.
    pub fn drain(&self) -> &[BlockId] {
        &self.drain
    }

    /// Where a reachable block sits in [`Cfg::drain`].
    ///
    /// A block the entry does not reach answers `usize::MAX`, which indexes
    /// nothing in `drain`, so asking about one is a panic where the answer is
    /// used rather than a position somebody else's block holds.
    pub fn drained_at(&self, block: BlockId) -> usize {
        self.drained_at[block.index()]
    }

    /// Where control can arrive at this block from.
    pub fn predecessors(&self, block: BlockId) -> &[BlockId] {
        &self.predecessors[block.index()]
    }

    /// Whether the entry reaches this block at all.
    ///
    /// A block nothing reaches is ordinary rather than exceptional: `int f(void)
    /// { return 1; return 2; }` makes one, and `--emit safety-ir` prints it.
    /// Whether that deserves a diagnostic is a question for whoever decides this
    /// compiler says so; what this owes is the answer.
    pub fn reaches(&self, block: BlockId) -> bool {
        self.reachable[block.index()]
    }

    /// Every loop, one per head: a block a back edge reaches, and every block
    /// that reaches one of those edges' sources backwards without passing the
    /// head.
    ///
    /// **A back edge is the solver's**: an edge from a block at or after its
    /// target in [`Cfg::drain`], which is what `dataflow::solve` joins as one.
    /// Read the same way here so that "the first turn of this loop", which the
    /// memory check asks a loop's reads against, ignores exactly the edges the
    /// solver treats as coming round again. Two readings of which edge is a
    /// back edge would answer two different loops.
    pub fn loops(&self) -> Vec<Loop> {
        let mut loops = Vec::new();
        for &head in &self.drain {
            let mut latches: Vec<BlockId> = Vec::new();
            for &from in self.predecessors(head) {
                if self.drained_at(from) >= self.drained_at(head) && !latches.contains(&from) {
                    latches.push(from);
                }
            }
            if latches.is_empty() {
                continue;
            }
            // Backwards from each latch, marking the head first so the walk
            // stops there rather than leaving the loop by its entry.
            let mut body = vec![false; self.reachable.len()];
            body[head.index()] = true;
            let mut stack = latches.clone();
            while let Some(block) = stack.pop() {
                if std::mem::replace(&mut body[block.index()], true) {
                    continue;
                }
                stack.extend(self.predecessors(block).iter().copied());
            }
            loops.push(Loop {
                head,
                body,
                latches,
            });
        }
        loops
    }

    /// Which reachable block dominates which: every path from the entry to a
    /// block passes through each of its dominators.
    ///
    /// Iterative over [`Cfg::order`], which is a reverse postorder, folding
    /// each block's predecessors by walking both up the tree to where they
    /// meet; it settles once no immediate dominator moves.
    pub fn dominators(&self) -> Dominators {
        let count = self.reachable.len();
        let mut position = vec![usize::MAX; count];
        for (at, &block) in self.order.iter().enumerate() {
            position[block.index()] = at;
        }
        let mut immediate: Vec<Option<BlockId>> = vec![None; count];
        let Some(&entry) = self.order.first() else {
            return Dominators {
                immediate,
                entry: None,
            };
        };
        immediate[entry.index()] = Some(entry);
        let meet = |immediate: &[Option<BlockId>], mut one: BlockId, mut two: BlockId| {
            while one != two {
                while position[one.index()] > position[two.index()] {
                    one = immediate[one.index()].expect("a processed block has one");
                }
                while position[two.index()] > position[one.index()] {
                    two = immediate[two.index()].expect("a processed block has one");
                }
            }
            one
        };
        let mut moved = true;
        while moved {
            moved = false;
            for &block in &self.order[1..] {
                let mut new: Option<BlockId> = None;
                for &from in self.predecessors(block) {
                    if immediate[from.index()].is_none() {
                        continue;
                    }
                    new = Some(match new {
                        None => from,
                        Some(sofar) => meet(&immediate, sofar, from),
                    });
                }
                if new.is_some() && immediate[block.index()] != new {
                    immediate[block.index()] = new;
                    moved = true;
                }
            }
        }
        Dominators {
            immediate,
            entry: Some(entry),
        }
    }
}

/// One loop of a function, as [`Cfg::loops`] finds it.
#[derive(Debug)]
pub struct Loop {
    head: BlockId,
    /// Whether each block is in the loop, its head included, indexed by
    /// [`BlockId::index`].
    body: Vec<bool>,
    /// The blocks a back edge into the head leaves from.
    latches: Vec<BlockId>,
}

impl Loop {
    /// The block every turn starts at.
    pub fn head(&self) -> BlockId {
        self.head
    }

    /// Whether `block` is in the loop, its head included.
    pub fn contains(&self, block: BlockId) -> bool {
        self.body[block.index()]
    }

    /// The blocks a back edge into the head leaves from.
    pub fn latches(&self) -> &[BlockId] {
        &self.latches
    }
}

/// What [`Cfg::dominators`] answers.
#[derive(Debug)]
pub struct Dominators {
    /// Each reachable block's immediate dominator, the entry its own, indexed
    /// by [`BlockId::index`]. `None` for a block the entry does not reach.
    immediate: Vec<Option<BlockId>>,
    entry: Option<BlockId>,
}

impl Dominators {
    /// Whether every path from the entry to `block` passes `dominator`. A
    /// block dominates itself, and a block the entry does not reach is
    /// dominated by nothing.
    pub fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        let mut at = block;
        loop {
            if at == dominator {
                return true;
            }
            match self.immediate[at.index()] {
                Some(up) if Some(at) != self.entry => at = up,
                _ => return false,
            }
        }
    }
}

/// The reachable blocks in an order where every loop, nested ones included,
/// has its blocks together and comes before anything after it.
///
/// The strongly connected components come in topological order. A component
/// that is a loop starts with its head, the block of it that comes first in
/// `order`, and the rest of it is ordered the same way again with the head
/// left out, so a loop inside it is a component of that rest and comes before
/// what follows it inside the outer loop. Without that second step, a loop
/// holding loops in sequence was one component taken in reverse postorder,
/// and was walked as often as before this order existed.
///
/// What is left to order is kept on a stack rather than recursed into, for the
/// reason [`Cfg::of`] gives for its own walk: how deeply loops nest is bounded
/// by nothing. See ADR-0016.
fn components(function: &Function, order: &[BlockId]) -> Vec<BlockId> {
    let count = function.blocks().len();
    let mut position = vec![usize::MAX; count];
    for (at, &block) in order.iter().enumerate() {
        position[block.index()] = at;
    }

    let mut drain = Vec::with_capacity(order.len());
    let mut tarjan = Tarjan::new(count);
    // Sets of blocks still to be ordered. The top is the one whose blocks come
    // next in `drain`.
    let mut pending: Vec<Vec<BlockId>> = vec![order.to_vec()];

    while let Some(members) = pending.pop() {
        let mut found = tarjan.components(function, &members);

        if found.len() == 1 {
            // One component: its head, then the rest of it ordered again.
            let mut component = found.pop().expect("one component was found");
            component.sort_by_key(|member| position[member.index()]);
            drain.push(component[0]);
            if component.len() > 1 {
                pending.push(component.split_off(1));
            }
        } else {
            // Tarjan finishes a component only after every component it
            // reaches, so `found` is last first, and pushing it in that order
            // leaves the first on top.
            pending.extend(found);
        }
    }

    drain
}

/// Tarjan's algorithm over a subset of a function's blocks, kept between calls
/// so that ordering the inside of each loop does not allocate a table the size
/// of the function again.
struct Tarjan {
    /// When the walk first reached a block, or `usize::MAX` if it has not.
    index: Vec<usize>,
    /// The earliest block still on `open` that a block reaches back to.
    low: Vec<usize>,
    /// Whether a block is one of the subset being ordered.
    inside: Vec<bool>,
    on_open: Vec<bool>,
}

impl Tarjan {
    fn new(count: usize) -> Self {
        Self {
            index: vec![usize::MAX; count],
            low: vec![0; count],
            inside: vec![false; count],
            on_open: vec![false; count],
        }
    }

    /// The strongly connected components of the graph `members` make, using
    /// only the edges between two of them, last first.
    ///
    /// Every member is a root in turn, in the order given, because once a
    /// loop's head is left out what remains need not be reachable from any
    /// one block of it. Iterative, for the reason [`components`] gives.
    fn components(&mut self, function: &Function, members: &[BlockId]) -> Vec<Vec<BlockId>> {
        for &member in members {
            self.index[member.index()] = usize::MAX;
            self.inside[member.index()] = true;
        }

        let mut reached = 0;
        let mut open = Vec::new();
        let mut found = Vec::new();
        let mut successors = Vec::new();
        let mut walk = Vec::new();

        for &root in members {
            if self.index[root.index()] != usize::MAX {
                continue;
            }
            self.reach(root, &mut reached, &mut open);
            walk.push((root, 0usize));

            while let Some((block, next)) = walk.pop() {
                successors.clear();
                function.block(block).terminator.successors(&mut successors);

                if let Some(&successor) = successors.get(next) {
                    walk.push((block, next + 1));
                    if !self.inside[successor.index()] {
                        // An edge out of the subset: the block it reaches
                        // has been ordered already, or is ordered later.
                    } else if self.index[successor.index()] == usize::MAX {
                        self.reach(successor, &mut reached, &mut open);
                        walk.push((successor, 0));
                    } else if self.on_open[successor.index()] {
                        // Only a block still open shares a component with
                        // this one. One that is closed is in a component
                        // already finished, which says nothing about where
                        // this block belongs.
                        self.low[block.index()] =
                            self.low[block.index()].min(self.index[successor.index()]);
                    }
                    continue;
                }

                // Every way out of `block` has been walked, so what it reaches
                // back to is also what the block that reached it does.
                if let Some(&(parent, _)) = walk.last() {
                    self.low[parent.index()] =
                        self.low[parent.index()].min(self.low[block.index()]);
                }
                if self.low[block.index()] == self.index[block.index()] {
                    let mut component = Vec::new();
                    loop {
                        let member = open.pop().expect("the block closing a component is open");
                        self.on_open[member.index()] = false;
                        component.push(member);
                        if member == block {
                            break;
                        }
                    }
                    found.push(component);
                }
            }
        }

        for &member in members {
            self.inside[member.index()] = false;
        }
        found
    }

    fn reach(&mut self, block: BlockId, reached: &mut usize, open: &mut Vec<BlockId>) {
        self.index[block.index()] = *reached;
        self.low[block.index()] = *reached;
        *reached += 1;
        open.push(block);
        self.on_open[block.index()] = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Block, FuncId, Operand, Origin, Place, Terminator, TranslationUnit, Ty};
    use crate::source::{SourceMap, Span};
    use crate::target::Target;

    fn spans() -> (SourceMap, Span) {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual("t.c", "int f(void) { return 1; }\n");
        let span = Span::new(file, 14, 22);
        (sources, span)
    }

    /// A unit with one function in it, and that function.
    ///
    /// The unit comes back because a `FuncId` and a `LocalId` cannot be spelled
    /// from outside `ir`, which is deliberate: a handle is a promise that the
    /// thing it names exists. A test that wants a callee pushes one.
    fn a_function(at: Span) -> (TranslationUnit, Function) {
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let function = Function::new(at, int, []);
        (unit, function)
    }

    /// Something for a `Call` to name, since a `FuncId` is only handed out by
    /// the unit that holds the function it names.
    fn callee(unit: &mut TranslationUnit, at: Span) -> FuncId {
        let void = unit.push_type(Ty::Void);
        unit.push_function(Function::declaration(at, void, []))
    }

    /// Where each block sits in the walk, for a test to compare positions
    /// rather than write an order out and be wrong about a detail it is not
    /// testing.
    fn at(cfg: &Cfg, block: BlockId) -> usize {
        cfg.order()
            .iter()
            .position(|&walked| walked == block)
            .unwrap_or_else(|| panic!("block {} is not in the walk", block.index()))
    }

    /// A block is taken after the blocks that reach it.
    ///
    /// A diamond and a loop, because they are the two shapes where the answer
    /// differs from the order the lowering pushed the blocks in. The loop is
    /// the one that matters: its body is pushed before the block that jumps
    /// back to its header, so `Function::blocks`'s order puts the body first.
    ///
    /// Positions rather than the whole order written out, because which of two
    /// arms of a branch comes first is not something this is holding anybody
    /// to, and a test that pinned it would fail for a change that is allowed.
    ///
    /// Mutation: keep the postorder rather than reversing it. The loop's
    /// header comes after its body and this fails, and so does the deep chain.
    /// The mutation the design named, answering `Function::blocks`'s order,
    /// cannot be written here at all: `BlockId` has no constructor outside
    /// `ir`, so this module cannot name a block the walk did not hand it.
    #[test]
    fn a_block_is_visited_after_what_reaches_it() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        // entry -> {left, right} -> join -> header -> body -> header
        //                                          -> exit
        let entry = function.reserve_block();
        let left = function.reserve_block();
        let right = function.reserve_block();
        let join = function.reserve_block();
        let header = function.reserve_block();
        let body = function.reserve_block();
        let exit = function.reserve_block();

        let go = |to| Block {
            elements: Vec::new(),
            terminator: Terminator::Goto(to),
        };
        function.fill_block(
            entry,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Branch {
                    condition: Operand::Constant(0),
                    then: left,
                    otherwise: right,
                    origin: Origin::Written(span),
                },
            },
        );
        function.fill_block(left, go(join));
        function.fill_block(right, go(join));
        function.fill_block(join, go(header));
        function.fill_block(
            header,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Branch {
                    condition: Operand::Constant(0),
                    then: body,
                    otherwise: exit,
                    origin: Origin::Written(span),
                },
            },
        );
        function.fill_block(body, go(header));
        function.fill_block(
            exit,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let cfg = Cfg::of(&function);

        assert_eq!(cfg.order().len(), 7);
        assert_eq!(at(&cfg, entry), 0);
        assert!(at(&cfg, left) < at(&cfg, join), "{:?}", cfg.order());
        assert!(at(&cfg, right) < at(&cfg, join), "{:?}", cfg.order());
        assert!(at(&cfg, join) < at(&cfg, header), "{:?}", cfg.order());
        // The one the declaration order gets wrong: `body` is pushed before the
        // edge back to `header` exists.
        assert!(at(&cfg, header) < at(&cfg, body), "{:?}", cfg.order());
        assert!(at(&cfg, header) < at(&cfg, exit), "{:?}", cfg.order());
    }

    /// A block outside a loop that the walk reaches first, through a side exit,
    /// still drains after the loop.
    ///
    /// `b0` and `b2` are a loop, and `b0` also leaves for `b1`, which goes on
    /// to the exit the loop reaches too. The walk takes `b1` and the exit
    /// before `b2`, so by the time `b2` reaches the exit, the exit is a
    /// component already closed. An edge into a closed component says nothing
    /// about where the block it leaves belongs, which is what Tarjan's test of
    /// whether the target is still open is for.
    ///
    /// Mutation: lower a block's `low` through any edge to a block already
    /// reached, open or not. `b1` joins the loop's component, drains before
    /// `b2`, and this fails.
    #[test]
    fn a_block_reached_first_through_a_side_exit_drains_after_the_loop() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        let b0 = function.reserve_block();
        let b1 = function.reserve_block();
        let b2 = function.reserve_block();
        let b3 = function.reserve_block();

        let branch = |then, otherwise| Block {
            elements: Vec::new(),
            terminator: Terminator::Branch {
                condition: Operand::Constant(0),
                then,
                otherwise,
                origin: Origin::Written(span),
            },
        };
        function.fill_block(b0, branch(b2, b1));
        function.fill_block(
            b1,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Goto(b3),
            },
        );
        function.fill_block(b2, branch(b3, b0));
        function.fill_block(
            b3,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let cfg = Cfg::of(&function);

        assert!(cfg.drained_at(b2) < cfg.drained_at(b1), "{:?}", cfg.drain());
        assert!(cfg.drained_at(b1) < cfg.drained_at(b3), "{:?}", cfg.drain());
    }

    /// Inside a loop, a loop drains before what follows it, as it does outside
    /// one.
    ///
    /// Two loops in sequence inside an outer one are all one strongly
    /// connected component, so ordering components alone would take them in
    /// reverse postorder, which may put the second inner loop's header before
    /// the first one's body. The first assertion says it does here, so that
    /// this keeps meaning something if the walk changes.
    ///
    /// Mutation: have `components` put a component's blocks into the drain in
    /// reverse postorder rather than ordering what follows its head again. The
    /// second assertion fails.
    #[test]
    fn a_loop_inside_a_loop_drains_before_what_follows_it_inside() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        let entry = function.reserve_block();
        let outer = function.reserve_block();
        let (first, first_body) = (function.reserve_block(), function.reserve_block());
        let (second, second_body) = (function.reserve_block(), function.reserve_block());
        let latch = function.reserve_block();
        let exit = function.reserve_block();

        let go = |to| Block {
            elements: Vec::new(),
            terminator: Terminator::Goto(to),
        };
        let branch = |then, otherwise| Block {
            elements: Vec::new(),
            terminator: Terminator::Branch {
                condition: Operand::Constant(0),
                then,
                otherwise,
                origin: Origin::Written(span),
            },
        };
        function.fill_block(entry, go(outer));
        function.fill_block(outer, branch(first, exit));
        function.fill_block(first, branch(first_body, second));
        function.fill_block(first_body, go(first));
        function.fill_block(second, branch(second_body, latch));
        function.fill_block(second_body, go(second));
        function.fill_block(latch, go(outer));
        function.fill_block(
            exit,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let cfg = Cfg::of(&function);

        assert!(at(&cfg, second) < at(&cfg, first_body), "{:?}", cfg.order());
        assert!(
            cfg.drained_at(first_body) < cfg.drained_at(second),
            "{:?}",
            cfg.drain()
        );
        assert_eq!(cfg.drained_at(outer), 1, "{:?}", cfg.drain());
        assert!(
            cfg.drained_at(second_body) < cfg.drained_at(latch),
            "{:?}",
            cfg.drain()
        );
    }

    /// Each loop holds its own blocks and no block outside it, and an inner
    /// loop is a loop of its own inside the outer one.
    ///
    /// Mutation: leave the head unmarked before walking back from the latches.
    /// The walk passes the head and takes in the entry, and this fails.
    /// Mutation: take only the first latch of a head. The outer loop's second
    /// latch is lost and this fails.
    #[test]
    fn a_loop_holds_its_own_blocks_and_an_inner_loop_is_its_own() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        let entry = function.reserve_block();
        let outer = function.reserve_block();
        let inner = function.reserve_block();
        let inner_body = function.reserve_block();
        let after_inner = function.reserve_block();
        let other_latch = function.reserve_block();
        let exit = function.reserve_block();

        let go = |to| Block {
            elements: Vec::new(),
            terminator: Terminator::Goto(to),
        };
        let branch = |then, otherwise| Block {
            elements: Vec::new(),
            terminator: Terminator::Branch {
                condition: Operand::Constant(0),
                then,
                otherwise,
                origin: Origin::Written(span),
            },
        };
        function.fill_block(entry, go(outer));
        function.fill_block(outer, branch(inner, exit));
        function.fill_block(inner, branch(inner_body, after_inner));
        function.fill_block(inner_body, go(inner));
        function.fill_block(after_inner, branch(outer, other_latch));
        function.fill_block(other_latch, go(outer));
        function.fill_block(
            exit,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let cfg = Cfg::of(&function);
        let loops = cfg.loops();
        assert_eq!(loops.len(), 2, "{loops:?}");
        let outer_loop = loops
            .iter()
            .find(|l| l.head() == outer)
            .expect("the outer loop");
        let inner_loop = loops
            .iter()
            .find(|l| l.head() == inner)
            .expect("the inner loop");
        for block in [outer, inner, inner_body, after_inner, other_latch] {
            assert!(outer_loop.contains(block), "{}", block.index());
        }
        for block in [entry, exit] {
            assert!(!outer_loop.contains(block), "{}", block.index());
        }
        assert_eq!(outer_loop.latches().len(), 2, "{outer_loop:?}");
        for block in [inner, inner_body] {
            assert!(inner_loop.contains(block), "{}", block.index());
        }
        for block in [entry, outer, after_inner, other_latch, exit] {
            assert!(!inner_loop.contains(block), "{}", block.index());
        }
    }

    /// A block dominates what every path to it passes, and an arm of a branch
    /// dominates nothing after the join.
    ///
    /// Mutation: take a block's first processed predecessor as its immediate
    /// dominator, without meeting the others. The `then` arm dominates the
    /// join and this fails.
    #[test]
    fn a_block_dominates_what_every_path_to_it_passes() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        let entry = function.reserve_block();
        let then = function.reserve_block();
        let otherwise = function.reserve_block();
        let join = function.reserve_block();
        let head = function.reserve_block();
        let body = function.reserve_block();
        let exit = function.reserve_block();

        let go = |to| Block {
            elements: Vec::new(),
            terminator: Terminator::Goto(to),
        };
        let branch = |then, otherwise| Block {
            elements: Vec::new(),
            terminator: Terminator::Branch {
                condition: Operand::Constant(0),
                then,
                otherwise,
                origin: Origin::Written(span),
            },
        };
        function.fill_block(entry, branch(then, otherwise));
        function.fill_block(then, go(join));
        function.fill_block(otherwise, go(join));
        function.fill_block(join, go(head));
        function.fill_block(head, branch(body, exit));
        function.fill_block(body, go(head));
        function.fill_block(
            exit,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let dominators = Cfg::of(&function).dominators();
        for block in [entry, then, otherwise, join, head, body, exit] {
            assert!(dominators.dominates(entry, block), "{}", block.index());
            assert!(dominators.dominates(block, block), "{}", block.index());
        }
        for arm in [then, otherwise] {
            for after in [join, head, body, exit] {
                assert!(
                    !dominators.dominates(arm, after),
                    "{} {}",
                    arm.index(),
                    after.index()
                );
            }
        }
        assert!(dominators.dominates(join, body));
        assert!(dominators.dominates(head, body));
        assert!(dominators.dominates(head, exit));
        assert!(!dominators.dominates(body, exit));
        assert!(!dominators.dominates(body, head));
    }

    /// A loop whose back edge leaves from two blocks into its body is one
    /// component, and drains before its exit.
    ///
    /// The loops above have a body of one block, whose back edge reaches the
    /// header directly, so a block learns it is inside a loop from its own
    /// edge. Here the first block of the body learns it only from the second,
    /// which is what Tarjan's passing of `low` up to the block that reached it
    /// is for.
    ///
    /// Mutation: do not pass a finished block's `low` to its parent. The body
    /// becomes a component of its own, drains after the exit, and this fails.
    #[test]
    fn a_loop_whose_back_edge_leaves_from_deeper_in_is_one_component() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        let entry = function.reserve_block();
        let header = function.reserve_block();
        let first = function.reserve_block();
        let second = function.reserve_block();
        let exit = function.reserve_block();

        let go = |to| Block {
            elements: Vec::new(),
            terminator: Terminator::Goto(to),
        };
        function.fill_block(entry, go(header));
        function.fill_block(
            header,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Branch {
                    condition: Operand::Constant(0),
                    then: first,
                    otherwise: exit,
                    origin: Origin::Written(span),
                },
            },
        );
        function.fill_block(first, go(second));
        function.fill_block(second, go(header));
        function.fill_block(
            exit,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let cfg = Cfg::of(&function);

        for block in [header, first, second] {
            assert!(
                cfg.drained_at(block) < cfg.drained_at(exit),
                "{:?}",
                cfg.drain()
            );
        }
    }

    /// Every block of a loop drains before every block after it, and a loop's
    /// header before its body.
    ///
    /// Three loops in sequence, which is the shape `Cfg::order` leaves open: a
    /// reverse postorder may put the second loop's header before the first
    /// loop's body, and here it does, which the first assertion says so that
    /// this test keeps meaning something if the walk changes.
    ///
    /// Mutation: keep the components in the order Tarjan finishes them,
    /// rather than reversed. Mutation: answer `order` for `drain`. The second
    /// assertion fails under either.
    #[test]
    fn a_loop_drains_before_what_comes_after_it() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        let entry = function.reserve_block();
        let loops: Vec<(BlockId, BlockId)> = (0..3)
            .map(|_| (function.reserve_block(), function.reserve_block()))
            .collect();
        let exit = function.reserve_block();

        let go = |to| Block {
            elements: Vec::new(),
            terminator: Terminator::Goto(to),
        };
        function.fill_block(entry, go(loops[0].0));
        for (k, &(header, body)) in loops.iter().enumerate() {
            let after = loops.get(k + 1).map_or(exit, |&(next, _)| next);
            function.fill_block(
                header,
                Block {
                    elements: Vec::new(),
                    terminator: Terminator::Branch {
                        condition: Operand::Constant(0),
                        then: body,
                        otherwise: after,
                        origin: Origin::Written(span),
                    },
                },
            );
            function.fill_block(body, go(header));
        }
        function.fill_block(
            exit,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let cfg = Cfg::of(&function);
        let (first, second) = (loops[0], loops[1]);

        assert!(at(&cfg, second.0) < at(&cfg, first.1), "{:?}", cfg.order());
        assert!(
            cfg.drained_at(first.1) < cfg.drained_at(second.0),
            "{:?}",
            cfg.drain()
        );
        for window in loops.windows(2) {
            let (earlier, later) = (window[0], window[1]);
            for block in [earlier.0, earlier.1] {
                for after in [later.0, later.1, exit] {
                    assert!(
                        cfg.drained_at(block) < cfg.drained_at(after),
                        "{:?}",
                        cfg.drain()
                    );
                }
            }
        }
        for &(header, body) in &loops {
            assert!(
                cfg.drained_at(header) < cfg.drained_at(body),
                "{:?}",
                cfg.drain()
            );
        }
        assert_eq!(cfg.drain().len(), cfg.order().len());
        assert_eq!(cfg.drained_at(entry), 0);
    }

    /// Every way into a block is a predecessor of it.
    ///
    /// One of each kind of edge there is, because the walk reads them through
    /// `Terminator::successors` and what that answers for is the thing being
    /// relied on: a terminator kind added later is `error[E0004]` there, and a
    /// field added to one is `error[E0027]`, because that match names every
    /// field rather than letting `..` walk past a new one.
    ///
    /// Both arms of a branch to one block are two edges and two entries. The
    /// graph says two, a join is idempotent, and a `Cfg` that deduplicated
    /// would answer about a graph the IR does not hold.
    ///
    /// Mutation: push the predecessor once for a branch whose arms agree. The
    /// last assertion fails. Mutation: drop the `Abnormal` arm from
    /// `Terminator::successors`. The third fails, and so does that function's
    /// own test.
    ///
    /// That predecessors hold reachable blocks only is not held by this test
    /// and cannot be: they are taken from the walk, and `BlockId` has no
    /// constructor outside `ir`, so nothing in this module can name a block the
    /// walk did not reach. The compiler holds it rather than an assertion.
    #[test]
    fn where_control_can_arrive_from() {
        let (_sources, span) = spans();
        let (mut unit, mut function) = a_function(span);

        let entry = function.reserve_block();
        let called = function.reserve_block();
        let handler = function.reserve_block();
        let twice = function.reserve_block();

        function.fill_block(
            entry,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Call {
                    callee: callee(&mut unit, span),
                    arguments: Vec::new(),
                    destination: Some(Place::local(function.return_place())),
                    then: Some(called),
                    origin: Origin::Written(span),
                },
            },
        );
        function.fill_block(
            called,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Abnormal { to: handler },
            },
        );
        function.fill_block(
            handler,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Branch {
                    condition: Operand::Constant(0),
                    then: twice,
                    otherwise: twice,
                    origin: Origin::Written(span),
                },
            },
        );
        function.fill_block(
            twice,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let cfg = Cfg::of(&function);

        assert_eq!(cfg.predecessors(entry), &[]);
        assert_eq!(cfg.predecessors(called), &[entry]);
        assert_eq!(cfg.predecessors(handler), &[called]);
        assert_eq!(cfg.predecessors(twice), &[handler, handler]);
    }

    /// A block the entry does not reach is not walked, and says so.
    ///
    /// Ordinary rather than exceptional: `int f(void) { return 1; return 2; }`
    /// makes one, and `--emit safety-ir` prints it. Measured, not imagined.
    ///
    /// The unreachable block has an edge out, which is what separates this from
    /// a test that only counts: a walk that started everywhere would put both
    /// it and what it points at in the answer, and a join over its value would
    /// be a join over a value that never exists.
    ///
    /// Mutation: answer `true` from `reaches`. The two negative assertions
    /// fail. Mutation: start every block's position in `drain` at zero rather
    /// than past its end. The last assertion fails, and that is what keeps a
    /// position nothing runs from naming the entry. Seeding the walk from every block, which is the mutation the
    /// design named, cannot be written: this module cannot name a block that
    /// `Function` did not hand it, and `Function` hands out one entry.
    #[test]
    fn a_block_nothing_reaches_is_not_walked() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        let entry = function.reserve_block();
        let stranded = function.reserve_block();
        let after = function.reserve_block();

        function.fill_block(
            entry,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );
        function.fill_block(
            stranded,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Goto(after),
            },
        );
        function.fill_block(
            after,
            Block {
                elements: Vec::new(),
                terminator: Terminator::Return,
            },
        );

        let cfg = Cfg::of(&function);

        assert_eq!(cfg.order(), &[entry]);
        assert!(cfg.reaches(entry));
        assert!(!cfg.reaches(stranded));
        assert!(!cfg.reaches(after));
        assert_eq!(cfg.predecessors(after), &[]);
        assert_eq!(cfg.drain(), &[entry]);
        assert_eq!(cfg.drained_at(entry), 0);
        assert_eq!(cfg.drained_at(stranded), usize::MAX);
    }

    /// A graph deeper than the stack is still walked.
    ///
    /// A chain of blocks is a chain of statements, and nothing bounds how many
    /// of those a C file holds: `MAX_NESTING` bounds what the parser accepts
    /// and says in its own doc comment that this is not the same as bounding
    /// the tree. So the walk is iterative, and this is what says so.
    ///
    /// Ten thousand, which overflows a recursive walk on every platform this
    /// runs on and takes milliseconds here.
    ///
    /// Mutation: write the walk recursively. This does not fail, it aborts:
    /// a stack overflow kills the process rather than the test, and the run
    /// ends with the harness reporting that a test binary died. That is a
    /// worse signal than a failure and it is still a signal, which is the
    /// trade this test makes.
    #[test]
    fn a_graph_deeper_than_a_stack_is_still_walked() {
        let (_sources, span) = spans();
        let (_unit, mut function) = a_function(span);

        const DEEP: usize = 10_000;
        let chain: Vec<BlockId> = (0..DEEP).map(|_| function.reserve_block()).collect();

        for (index, &block) in chain.iter().enumerate() {
            let terminator = match chain.get(index + 1) {
                Some(&next) => Terminator::Goto(next),
                None => Terminator::Return,
            };
            function.fill_block(
                block,
                Block {
                    elements: Vec::new(),
                    terminator,
                },
            );
        }

        let cfg = Cfg::of(&function);

        assert_eq!(cfg.order().len(), DEEP);
        assert_eq!(cfg.order()[0], chain[0]);
        assert_eq!(cfg.order()[DEEP - 1], chain[DEEP - 1]);
    }
}
