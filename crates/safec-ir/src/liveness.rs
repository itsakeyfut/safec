//! Which locals may be read from each block's entry before they are written.
//!
//! A fact about a function alone, read by nothing but its IR, so it is at
//! crate level rather than inside the analysis that asks it first: the
//! memory check's join clears what this says is dead (ADR-0048).

use crate::ir::{Element, Function, Operand, Place, Projection, Rvalue, Terminator};

/// Per block of `function`, the locals that may be read from its entry before
/// they are written: a backward fixpoint over the graph.
///
/// **It over-counts reads, and must.** A read it misses clears a local at an
/// edge that something reads later, and a dereference of a local holding
/// nothing says nothing, which is silence. So a local is read wherever it
/// appears in a place other than as the whole destination of an assignment or
/// a call: the base of a projection, an `Index` operand, an operand of an
/// rvalue, a call's argument, a branch's condition, and the return place at
/// `Return`. A local whose address is taken anywhere in the function is live
/// in every block, since a pointer may read it. Every kind of element,
/// rvalue, projection and terminator is written out, so a new one is
/// `error[E0004]` here until it says what it reads. See ADR-0048.
pub(crate) fn live_in(function: &Function) -> Vec<Vec<bool>> {
    let locals = function.locals().len();
    let mut always = vec![false; locals];
    // What each block reads before writing it, and what it writes.
    let mut reads_first: Vec<Vec<bool>> = Vec::new();
    let mut writes: Vec<Vec<bool>> = Vec::new();
    let mut next: Vec<Vec<usize>> = Vec::new();

    for block in function.blocks() {
        let mut read = vec![false; locals];
        let mut written = vec![false; locals];
        // A function rather than a closure, since an `Index` operand is a
        // place with projections of its own, `a[b[x]]`, and is read however
        // deep. Found by review.
        fn reads_of_place(place: &Place, read: &mut Vec<bool>, written: &[bool]) {
            if !written[place.local.index()] {
                read[place.local.index()] = true;
            }
            for step in &place.projection {
                match step {
                    Projection::Deref => {}
                    Projection::Index(Operand::Copy(index)) => reads_of_place(index, read, written),
                    Projection::Index(Operand::Constant(_)) => {}
                }
            }
        }
        let reads_of_operand =
            |operand: &Operand, read: &mut Vec<bool>, written: &[bool]| match operand {
                Operand::Copy(place) => reads_of_place(place, read, written),
                Operand::Constant(_) => {}
            };
        // A destination that is the whole local writes it; one through a
        // projection reads the local it goes through.
        let writes_to = |place: &Place, read: &mut Vec<bool>, written: &mut Vec<bool>| {
            if place.projection.is_empty() {
                written[place.local.index()] = true;
            } else {
                reads_of_operand(&Operand::Copy(place.clone()), read, written);
            }
        };

        for element in &block.elements {
            match element {
                Element::Assign(operation) => {
                    match &operation.value {
                        Rvalue::Use(operand) | Rvalue::Unary { operand, .. } => {
                            reads_of_operand(operand, &mut read, &written);
                        }
                        Rvalue::Binary { lhs, rhs, .. } => {
                            reads_of_operand(lhs, &mut read, &written);
                            reads_of_operand(rhs, &mut read, &written);
                        }
                        Rvalue::Address(place) => {
                            always[place.local.index()] = true;
                            reads_of_operand(&Operand::Copy(place.clone()), &mut read, &written);
                        }
                    }
                    writes_to(&operation.place, &mut read, &mut written);
                }
                Element::Evaluate { place, origin: _ } => {
                    reads_of_operand(&Operand::Copy(place.clone()), &mut read, &written);
                }
                Element::StorageLive {
                    local: _,
                    origin: _,
                }
                | Element::StorageDead {
                    local: _,
                    origin: _,
                }
                | Element::Sequenced { origin: _ }
                | Element::ArgumentsEvaluated { origin: _ }
                | Element::UnsequencedAccess {
                    local: _,
                    written: _,
                    accessed: _,
                    both_write: _,
                    between: _,
                    origin: _,
                } => {}
            }
        }
        match &block.terminator {
            Terminator::Goto(_) | Terminator::Abnormal { to: _ } => {}
            Terminator::Branch { condition, .. } => {
                reads_of_operand(condition, &mut read, &written);
            }
            Terminator::Call {
                arguments,
                destination,
                ..
            } => {
                for argument in arguments {
                    reads_of_operand(argument, &mut read, &written);
                }
                if let Some(place) = destination {
                    writes_to(place, &mut read, &mut written);
                }
            }
            Terminator::Return => {
                let place = function.return_place();
                if !written[place.index()] {
                    read[place.index()] = true;
                }
            }
        }
        reads_first.push(read);
        writes.push(written);
        let mut successors = Vec::new();
        block.terminator.successors(&mut successors);
        next.push(successors.iter().map(|block| block.index()).collect());
    }

    let mut live = vec![vec![false; locals]; reads_first.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for block in (0..live.len()).rev() {
            let mut out = vec![false; locals];
            for &successor in &next[block] {
                for (out, &live) in out.iter_mut().zip(&live[successor]) {
                    *out |= live;
                }
            }
            let entry: Vec<bool> = (0..locals)
                .map(|local| {
                    always[local]
                        || reads_first[block][local]
                        || (out[local] && !writes[block][local])
                })
                .collect();
            if entry != live[block] {
                live[block] = entry;
                changed = true;
            }
        }
    }
    live
}

#[cfg(test)]
mod tests {
    //! [`live_in`], one kind of read at a time. Each test builds a function
    //! whose only read of `x` after it is written is of one kind, and asserts
    //! `x` is live at the block that reads it and not at the block that writes
    //! it first. A read `live_in` misses is a local cleared at an edge while
    //! something still reads it, which is silence (ADR-0048), so each test's
    //! mutation is dropping its own kind of read, and it fails on the first
    //! assertion.

    use super::live_in;
    use crate::ir::{
        Block, BlockId, Element, Function, LocalId, Operand, Operation, Origin, Place, Projection,
        Rvalue, Terminator, TranslationUnit, Ty,
    };
    use crate::source::{SourceMap, Span};
    use crate::target::Target;

    /// `x = 0;` in the first block, then `read` in the second, then a return.
    fn liveness(
        read: impl FnOnce(&mut TranslationUnit, LocalId, LocalId, Span, BlockId) -> Block,
    ) -> (bool, bool) {
        let mut sources = SourceMap::new();
        let file = sources.add_virtual(
            "t.c",
            "int f(void) { return 0; }
",
        );
        let at = Span::new(file, 0, 3);
        let mut unit = TranslationUnit::new(
            Target::from_triple("x86_64-pc-windows-msvc").expect("a known triple"),
        );
        let int = unit.push_type(Ty::Int);
        let pointer = unit.push_type(Ty::Pointer(int));
        let mut function = Function::new(at, int, []);
        let x = function.push_local(pointer);
        let y = function.push_local(int);
        let writes = function.reserve_block();
        let reads = function.reserve_block();
        let exit = function.reserve_block();
        function.fill_block(
            writes,
            Block {
                elements: vec![Element::Assign(Operation {
                    place: Place::local(x),
                    value: Rvalue::Use(Operand::Constant(0)),
                    origin: Origin::Written(at),
                })],
                terminator: Terminator::Goto(reads),
            },
        );
        let block = read(&mut unit, x, y, at, exit);
        function.fill_block(reads, block);
        function.fill_block(
            exit,
            Block {
                elements: vec![],
                terminator: Terminator::Return,
            },
        );
        let live = live_in(&function);
        (
            live[reads.index()][x.index()],
            live[writes.index()][x.index()],
        )
    }

    fn assign(place: Place, value: Rvalue, at: Span, then: BlockId) -> Block {
        Block {
            elements: vec![Element::Assign(Operation {
                place,
                value,
                origin: Origin::Written(at),
            })],
            terminator: Terminator::Goto(then),
        }
    }

    fn deref(local: LocalId) -> Place {
        Place {
            local,
            projection: vec![Projection::Deref],
        }
    }

    #[test]
    fn a_local_copied_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            assign(
                Place::local(y),
                Rvalue::Use(Operand::Copy(Place::local(x))),
                at,
                then,
            )
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_dereferenced_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            assign(
                Place::local(y),
                Rvalue::Use(Operand::Copy(deref(x))),
                at,
                then,
            )
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_used_as_an_index_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            let place = Place {
                local: y,
                projection: vec![Projection::Index(Operand::Copy(Place::local(x)))],
            };
            assign(Place::local(y), Rvalue::Use(Operand::Copy(place)), at, then)
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_written_through_is_read() {
        let (at_read, at_write) = liveness(|_, x, _, at, then| {
            assign(deref(x), Rvalue::Use(Operand::Constant(1)), at, then)
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_in_arithmetic_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            assign(
                Place::local(y),
                Rvalue::Binary {
                    op: crate::ir::BinOp::Add,
                    lhs: Operand::Copy(Place::local(x)),
                    rhs: Operand::Constant(1),
                },
                at,
                then,
            )
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_evaluated_is_read() {
        let (at_read, at_write) = liveness(|_, x, _, at, then| Block {
            elements: vec![Element::Evaluate {
                place: deref(x),
                origin: Origin::Written(at),
            }],
            terminator: Terminator::Goto(then),
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_handed_to_a_call_is_read() {
        let (at_read, at_write) = liveness(|unit, x, _, at, then| {
            let void = unit.push_type(Ty::Void);
            let int = unit.push_type(Ty::Int);
            let pointer = unit.push_type(Ty::Pointer(int));
            let callee = unit.push_function(Function::declaration(at, void, [pointer]));
            Block {
                elements: vec![],
                terminator: Terminator::Call {
                    callee,
                    arguments: vec![Operand::Copy(Place::local(x))],
                    destination: None,
                    then: Some(then),
                    origin: Origin::Written(at),
                },
            }
        });
        assert!(at_read);
        assert!(!at_write);
    }

    #[test]
    fn a_local_a_branch_tests_is_read() {
        let (at_read, at_write) = liveness(|_, x, _, at, then| Block {
            elements: vec![],
            terminator: Terminator::Branch {
                condition: Operand::Copy(Place::local(x)),
                then,
                otherwise: then,
                origin: Origin::Written(at),
            },
        });
        assert!(at_read);
        assert!(!at_write);
    }

    /// Mutation: count only the local an `Index` operand names, not the
    /// indices inside it; this fails.
    #[test]
    fn a_local_used_as_an_index_inside_an_index_is_read() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            let inner = Place {
                local: y,
                projection: vec![Projection::Index(Operand::Copy(Place::local(x)))],
            };
            let place = Place {
                local: y,
                projection: vec![Projection::Index(Operand::Copy(inner))],
            };
            assign(Place::local(y), Rvalue::Use(Operand::Copy(place)), at, then)
        });
        assert!(at_read);
        assert!(!at_write);
    }

    /// And a local whose address is taken is live everywhere, the block that
    /// writes it included, since a pointer may read it anywhere. Mutation:
    /// leave out `always`; the second assertion fails.
    #[test]
    fn a_local_whose_address_is_taken_is_live_everywhere() {
        let (at_read, at_write) = liveness(|_, x, y, at, then| {
            assign(Place::local(y), Rvalue::Address(Place::local(x)), at, then)
        });
        assert!(at_read);
        assert!(at_write);
    }
}
