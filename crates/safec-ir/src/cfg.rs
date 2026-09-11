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
/// reachable set, and "can control get here" is answered by asking where a
/// block sits in it rather than by a second table that could disagree.
///
/// Built per function and thrown away: the IR does not change while a pass runs
/// over it, and a `Cfg` that outlived one would be a claim about a function that
/// somebody may since have filled in.
#[derive(Debug)]
pub struct Cfg {
    /// Every block the entry reaches, in an order where a block comes after the
    /// blocks that reach it wherever the graph allows one.
    order: Vec<BlockId>,
    /// Where each block sits in `order`, and `None` for a block the entry does
    /// not reach. Indexed by [`BlockId::index`].
    ///
    /// One walk fills this and `order` together, so nothing here can disagree
    /// with what is in the walk about which blocks run.
    position: Vec<Option<u32>>,
    /// Where control can arrive from, per block, indexed by [`BlockId::index`].
    ///
    /// **Reachable blocks only, on both sides.** A block the entry never
    /// reaches is nobody's predecessor, even where it has an edge out, and has
    /// no predecessors of its own. An analysis joins what arrives at a block,
    /// and a value from a block that never runs is not a value: including one
    /// would make every caller filter, and the first that forgot would join a
    /// bottom into a fixpoint and answer that nothing is known anywhere.
    predecessors: Vec<Vec<BlockId>>,
}

impl Cfg {
    /// Walk one function's graph.
    ///
    /// **Iteratively, and that is not a preference.** The obvious depth-first
    /// search is recursive and is bounded by how deep the graph is, which is
    /// bounded by nothing: `MAX_NESTING` bounds what the parser accepts and says
    /// in its own doc comment that this is not the same as bounding the tree,
    /// which is RK-019's neighbour RK-008 in the review knowledge bank. A chain
    /// of blocks long enough to overflow a stack is a chain of statements, and
    /// the answer to that must not be a process that dies without a diagnostic.
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

        let mut position = vec![None; count];
        for (at, block) in order.iter().enumerate() {
            position[block.index()] = Some(at as u32);
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

        Self {
            order,
            position,
            predecessors,
        }
    }

    /// The order to take the blocks in, which holds exactly the reachable ones.
    pub fn order(&self) -> &[BlockId] {
        &self.order
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
        self.position[block.index()].is_some()
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

    /// Every way into a block is a predecessor of it.
    ///
    /// One of each kind of edge there is, because the walk reads them through
    /// `Terminator::successors` and what that answers for is the thing being
    /// relied on: a terminator kind added later is `error[E0004]` there, and a
    /// field added to one is `error[E0027]`, which is RK-018.
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
                    then: called,
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
    /// fail. Seeding the walk from every block, which is the mutation the
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
    }

    /// A graph deeper than the stack is still walked.
    ///
    /// A chain of blocks is a chain of statements, and nothing bounds how many
    /// of those a C file holds: `MAX_NESTING` bounds what the parser accepts
    /// and says in its own doc comment that this is not the same as bounding
    /// the tree, which is RK-008. So the walk is iterative, and this is what
    /// says so.
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
