//! Each belief the memory check keeps about memory, asked once per route a
//! pointer can take into memory, as a matrix generated at test time.
//!
//! A row is a belief, a route, a depth and a path shape. Its test writes two
//! programs, a hazard the belief has to report and a control it must not, and
//! asks the belief's rule of both. **The rule comes from the belief and not
//! from the checker**, so a route the checker answers wrongly is a named test
//! failing rather than a review finding; and **a control is asked as well**,
//! since a rule asked only of the hazard passes a checker that reports every
//! program. Beside the rows, the whole matrix's answers are blessed into one
//! summary, so that a change silencing a route and its plain store together,
//! which every row would still agree with, is a diff somebody reads.
//!
//! The rows are a literal table with a name each, as the corpus is (ADR-0007),
//! and the table is checked against its axes. See ADR-0062.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What memory is believed to hold, and what reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Belief {
    /// How wide a pointer stored there points (ADR-0060, ADR-0061): a
    /// `char`'s address read back as an `int *` and written through.
    Width,
    /// Whether it may be indeterminate (ADR-0057): memory nothing wrote read
    /// back as a pointer.
    Fresh,
    /// Whether a pointer there is one this check stopped following
    /// (ADR-0045): a loop's earlier allocation, put there, freed by the name
    /// it was put there under, read back and used. Held in memory, a lost
    /// pointer is doubted only once something may have freed it, so the free
    /// is what makes it a hazard; and not a free of the loop's later
    /// allocation, which is another object (ADR-0045).
    Lost,
    /// Whether a pointer there may be stale (ADR-0045): stored on a loop's
    /// first turn, freed, its site made again, read back and used.
    Stale,
}

/// How a pointer gets where it is read back from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    /// `slot = v;`
    Store,
    /// `memcpy(&slot, &v_, 8);`
    Memcpy,
    /// Stored, then what holds it grown by `realloc`.
    Realloc,
    /// A call this check cannot read writes it through an out-parameter.
    Call,
    /// Handed through `void *pass(void *)`, which this check cannot read.
    Ret,
}

/// Where the slot is: a local, or one or two levels below one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Depth {
    D0,
    D1,
    D2,
}

/// Whether the route runs on every path, or on one arm with a null stored on
/// the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Both,
    One,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    belief: Belief,
    route: Route,
    depth: Depth,
    shape: Shape,
}

const BELIEFS: [Belief; 4] = [Belief::Width, Belief::Fresh, Belief::Lost, Belief::Stale];
const ROUTES: [Route; 5] = [
    Route::Store,
    Route::Memcpy,
    Route::Realloc,
    Route::Call,
    Route::Ret,
];
const DEPTHS: [Depth; 3] = [Depth::D0, Depth::D1, Depth::D2];
const SHAPES: [Shape; 2] = [Shape::Both, Shape::One];

/// One `#[test]` per row, and the roster the axis guard reads, from one
/// literal table, as `cases!` builds the corpus (ADR-0007).
macro_rules! routes {
    ($($name:ident : $belief:ident $route:ident $depth:ident $shape:ident;)*) => {
        const ROWS: &[(&str, Row)] = &[$((stringify!($name), Row {
            belief: Belief::$belief,
            route: Route::$route,
            depth: Depth::$depth,
            shape: Shape::$shape,
        })),*];

        $(
            #[test]
            fn $name() {
                check(stringify!($name), Row {
                    belief: Belief::$belief,
                    route: Route::$route,
                    depth: Depth::$depth,
                    shape: Shape::$shape,
                });
            }
        )*
    };
}

routes! {
    width_store_d0_both: Width Store D0 Both;
    width_store_d0_one: Width Store D0 One;
    width_store_d1_both: Width Store D1 Both;
    width_store_d1_one: Width Store D1 One;
    width_store_d2_both: Width Store D2 Both;
    width_store_d2_one: Width Store D2 One;
    width_memcpy_d0_both: Width Memcpy D0 Both;
    width_memcpy_d0_one: Width Memcpy D0 One;
    width_memcpy_d1_both: Width Memcpy D1 Both;
    width_memcpy_d1_one: Width Memcpy D1 One;
    width_memcpy_d2_both: Width Memcpy D2 Both;
    width_memcpy_d2_one: Width Memcpy D2 One;
    width_realloc_d1_both: Width Realloc D1 Both;
    width_realloc_d1_one: Width Realloc D1 One;
    width_realloc_d2_both: Width Realloc D2 Both;
    width_realloc_d2_one: Width Realloc D2 One;
    width_call_d0_both: Width Call D0 Both;
    width_call_d0_one: Width Call D0 One;
    width_call_d1_both: Width Call D1 Both;
    width_call_d1_one: Width Call D1 One;
    width_call_d2_both: Width Call D2 Both;
    width_call_d2_one: Width Call D2 One;
    width_ret_d0_both: Width Ret D0 Both;
    width_ret_d0_one: Width Ret D0 One;
    width_ret_d1_both: Width Ret D1 Both;
    width_ret_d1_one: Width Ret D1 One;
    width_ret_d2_both: Width Ret D2 Both;
    width_ret_d2_one: Width Ret D2 One;
    fresh_store_d0_both: Fresh Store D0 Both;
    fresh_store_d0_one: Fresh Store D0 One;
    fresh_store_d1_both: Fresh Store D1 Both;
    fresh_store_d1_one: Fresh Store D1 One;
    fresh_store_d2_both: Fresh Store D2 Both;
    fresh_store_d2_one: Fresh Store D2 One;
    fresh_memcpy_d0_both: Fresh Memcpy D0 Both;
    fresh_memcpy_d0_one: Fresh Memcpy D0 One;
    fresh_memcpy_d1_both: Fresh Memcpy D1 Both;
    fresh_memcpy_d1_one: Fresh Memcpy D1 One;
    fresh_memcpy_d2_both: Fresh Memcpy D2 Both;
    fresh_memcpy_d2_one: Fresh Memcpy D2 One;
    fresh_realloc_d0_both: Fresh Realloc D0 Both;
    fresh_realloc_d0_one: Fresh Realloc D0 One;
    fresh_realloc_d1_both: Fresh Realloc D1 Both;
    fresh_realloc_d1_one: Fresh Realloc D1 One;
    fresh_realloc_d2_both: Fresh Realloc D2 Both;
    fresh_realloc_d2_one: Fresh Realloc D2 One;
    lost_store_d0_both: Lost Store D0 Both;
    lost_store_d0_one: Lost Store D0 One;
    lost_store_d1_both: Lost Store D1 Both;
    lost_store_d1_one: Lost Store D1 One;
    lost_store_d2_both: Lost Store D2 Both;
    lost_store_d2_one: Lost Store D2 One;
    lost_memcpy_d0_both: Lost Memcpy D0 Both;
    lost_memcpy_d0_one: Lost Memcpy D0 One;
    lost_memcpy_d1_both: Lost Memcpy D1 Both;
    lost_memcpy_d1_one: Lost Memcpy D1 One;
    lost_memcpy_d2_both: Lost Memcpy D2 Both;
    lost_memcpy_d2_one: Lost Memcpy D2 One;
    lost_realloc_d1_both: Lost Realloc D1 Both;
    lost_realloc_d1_one: Lost Realloc D1 One;
    lost_realloc_d2_both: Lost Realloc D2 Both;
    lost_realloc_d2_one: Lost Realloc D2 One;
    lost_ret_d0_both: Lost Ret D0 Both;
    lost_ret_d0_one: Lost Ret D0 One;
    lost_ret_d1_both: Lost Ret D1 Both;
    lost_ret_d1_one: Lost Ret D1 One;
    lost_ret_d2_both: Lost Ret D2 Both;
    lost_ret_d2_one: Lost Ret D2 One;
    stale_store_d0_both: Stale Store D0 Both;
    stale_store_d1_both: Stale Store D1 Both;
    stale_store_d2_both: Stale Store D2 Both;
    stale_memcpy_d0_both: Stale Memcpy D0 Both;
    stale_memcpy_d1_both: Stale Memcpy D1 Both;
    stale_memcpy_d2_both: Stale Memcpy D2 Both;
    stale_realloc_d1_both: Stale Realloc D1 Both;
    stale_realloc_d2_both: Stale Realloc D2 Both;
    stale_ret_d0_both: Stale Ret D0 Both;
    stale_ret_d1_both: Stale Ret D1 Both;
    stale_ret_d2_both: Stale Ret D2 Both;
}

/// The combinations no program is written for, each with why: a row that is
/// neither here nor in the table fails
/// `the_matrix_is_every_combination_of_its_axes`.
/// A combination of the axes no program is written for, and why.
struct Unwritten {
    route: Route,
    depth: Option<Depth>,
    belief: Option<Belief>,
    shape: Option<Shape>,
    why: &'static str,
}

const NOT_WRITTEN: &[Unwritten] = &[
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Width),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Lost),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Realloc,
        depth: Some(Depth::D0),
        belief: Some(Belief::Stale),
        shape: None,
        why: "a local is not grown by realloc",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a call this check cannot read may write the memory, so it is not fresh",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Lost),
        shape: None,
        why: "a call writes a value of its own, not the loop's lost one",
    },
    Unwritten {
        route: Route::Call,
        depth: None,
        belief: Some(Belief::Stale),
        shape: None,
        why: "a call writes a value of its own, not the loop's freed one",
    },
    Unwritten {
        route: Route::Ret,
        depth: None,
        belief: Some(Belief::Fresh),
        shape: None,
        why: "a value handed through a call is a pointer written into the slot, not memory nothing wrote",
    },
    Unwritten {
        route: Route::Store,
        depth: None,
        belief: Some(Belief::Stale),
        shape: Some(Shape::One),
        why: "a stale pointer is made across a loop's turns, whose first-turn branch is already the one-arm shape",
    },
    Unwritten {
        route: Route::Memcpy,
        depth: None,
        belief: Some(Belief::Stale),
        shape: Some(Shape::One),
        why: "a stale pointer is made across a loop's turns, whose first-turn branch is already the one-arm shape",
    },
    Unwritten {
        route: Route::Realloc,
        depth: None,
        belief: Some(Belief::Stale),
        shape: Some(Shape::One),
        why: "a stale pointer is made across a loop's turns, whose first-turn branch is already the one-arm shape",
    },
    Unwritten {
        route: Route::Ret,
        depth: None,
        belief: Some(Belief::Stale),
        shape: Some(Shape::One),
        why: "a stale pointer is made across a loop's turns, whose first-turn branch is already the one-arm shape",
    },
];

/// Which side of a row disagrees with its belief's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// The hazard is not reported: a silence.
    Hazard,
    /// The control is reported too: a doubt about a program that has none.
    Control,
    /// Both.
    Both,
}

/// Why a row is let disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// A false report on a control that is defined C, or a silence, that an
    /// open issue or a known cost answers for: the row agreeing is a fix,
    /// and removes its entry.
    Cost(&'static str),
    /// A report a record takes on purpose, because the control is not
    /// provable from the unit by this check: it hands the pointer to a call
    /// this check cannot read, or reads one the check stopped following.
    /// **The row agreeing is a regression**, the check saying safe about
    /// what it did not prove, unless that record is superseded.
    Belief(&'static str),
}

/// The rows whose answer is not their belief's rule, each with the issue or
/// the record that says why. **Exact in both directions**: a row listed here
/// that now agrees fails, and what that means is the entry's [`Why`].
///
/// **A row listed on [`Side::Control`] no longer asks its hazard**: the
/// control already carries the belief's code, so the hazard's report cannot
/// be told from it. Such a row holds only that the route is not silent
/// altogether, and the blessed summary is what notices more. See ADR-0062.
const ACCEPTED: &[(&str, Side, Why)] = &[
    (
        "width_ret_d0_both",
        Side::Control,
        Why::Belief(
            "a `void *` a call this check cannot read returns believes no width (ADR-0060)",
        ),
    ),
    (
        "width_ret_d0_one",
        Side::Control,
        Why::Belief(
            "a `void *` a call this check cannot read returns believes no width (ADR-0060)",
        ),
    ),
    (
        "width_ret_d1_both",
        Side::Control,
        Why::Belief(
            "a `void *` a call this check cannot read returns believes no width (ADR-0060)",
        ),
    ),
    (
        "width_ret_d1_one",
        Side::Control,
        Why::Belief(
            "a `void *` a call this check cannot read returns believes no width (ADR-0060)",
        ),
    ),
    (
        "width_ret_d2_both",
        Side::Control,
        Why::Belief(
            "a `void *` a call this check cannot read returns believes no width (ADR-0060)",
        ),
    ),
    (
        "width_ret_d2_one",
        Side::Control,
        Why::Belief(
            "a `void *` a call this check cannot read returns believes no width (ADR-0060)",
        ),
    ),
    (
        "width_call_d0_both",
        Side::Control,
        Why::Belief(
            "a call this check cannot read writes the local through its address, and what it wrote is doubted where read through, the output-parameter cost (ADR-0060)",
        ),
    ),
    (
        "width_call_d0_one",
        Side::Control,
        Why::Belief(
            "a call this check cannot read writes the local through its address, and what it wrote is doubted where read through, the output-parameter cost (ADR-0060)",
        ),
    ),
    ("width_memcpy_d0_both", Side::Control, Why::Cost("#769")),
    ("width_memcpy_d0_one", Side::Control, Why::Cost("#769")),
    ("fresh_memcpy_d2_both", Side::Hazard, Why::Cost("#768")),
    ("fresh_memcpy_d2_one", Side::Hazard, Why::Cost("#768")),
    (
        "lost_ret_d2_both",
        Side::Control,
        Why::Belief(
            "a pointer handed to a call this check cannot read may be freed by it (ADR-0039)",
        ),
    ),
    (
        "lost_ret_d2_one",
        Side::Control,
        Why::Belief(
            "a pointer handed to a call this check cannot read may be freed by it (ADR-0039)",
        ),
    ),
    (
        "lost_ret_d0_both",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "lost_ret_d0_one",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "stale_ret_d0_both",
        Side::Control,
        Why::Belief(
            "a pointer handed to a call this check cannot read may be freed by it (ADR-0039)",
        ),
    ),
    (
        "stale_ret_d1_both",
        Side::Control,
        Why::Belief(
            "a pointer handed to a call this check cannot read may be freed by it (ADR-0039)",
        ),
    ),
    (
        "stale_ret_d2_both",
        Side::Control,
        Why::Belief(
            "a pointer handed to a call this check cannot read may be freed by it (ADR-0039)",
        ),
    ),
    (
        "stale_memcpy_d0_both",
        Side::Control,
        Why::Cost("a pointer copied by memcpy is a doubt about whether it was freed (#652)"),
    ),
    (
        "stale_memcpy_d1_both",
        Side::Control,
        Why::Cost("a pointer copied by memcpy is a doubt about whether it was freed (#652)"),
    ),
    (
        "stale_memcpy_d2_both",
        Side::Control,
        Why::Cost("a pointer copied by memcpy is a doubt about whether it was freed (#652)"),
    ),
    (
        "stale_realloc_d2_both",
        Side::Control,
        Why::Cost(
            "the old block a store replaced in memory is still taken to be there after realloc moves it (#656)",
        ),
    ),
    (
        "lost_realloc_d2_both",
        Side::Control,
        Why::Cost(
            "the old block a store replaced in memory is still taken to be there after realloc moves it (#656)",
        ),
    ),
    (
        "lost_realloc_d2_one",
        Side::Control,
        Why::Cost(
            "the old block a store replaced in memory is still taken to be there after realloc moves it (#656)",
        ),
    ),
    (
        "lost_store_d0_both",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "lost_store_d0_one",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "lost_memcpy_d0_both",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "lost_memcpy_d0_one",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "lost_memcpy_d1_both",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "lost_memcpy_d1_one",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "lost_memcpy_d2_both",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
    (
        "lost_memcpy_d2_one",
        Side::Control,
        Why::Belief(
            "a lost pointer read out of a local, copied by memcpy or handed through a call, proves nothing where it is used (ADR-0045)",
        ),
    ),
];

/// What a belief's hazard is reported as: a code, and where one code stands
/// for two beliefs, a phrase of its message.
fn mark(belief: Belief) -> (&'static str, Option<&'static str>) {
    match belief {
        Belief::Width => ("SC0412", None),
        Belief::Fresh => ("SC0411", None),
        // **Either phrase**: a free of a lost pointer is reported in whichever
        // words the route leaves it, "this check stopped following" at a
        // local and "may have been freed" through a `realloc` two levels
        // down, and both are the doubt the belief exists to give. The
        // control, which differs in the free alone, is what keeps this from
        // passing on a doubt the free did not cause.
        Belief::Lost => ("SC0402", None),
        // **The freeing phrase alone**: the control is a loop that makes the
        // site again with nothing freed, which this check rightly calls a
        // pointer it stopped following (ADR-0045), so `SC0402` alone would
        // count the control's lost doubt as the hazard's.
        Belief::Stale => ("SC0402", Some("may have been freed")),
    }
}

const DECLARATIONS: &str = "void *malloc(int n);
void free(void *p);
void *realloc(void *p, int n);
void *memcpy(void *d, void *s, int n);
void *pass(void *p);
void get_narrow(char **p);
void get_wide(int **p);
";

/// The statements that make the slot, its spelling, and what it lives in.
fn container(depth: Depth) -> (Vec<String>, &'static str) {
    let made: &[&str] = match depth {
        Depth::D0 => &["int *s = 0;"],
        Depth::D1 => &["int **m = malloc(8);", "if (!m) return 0;"],
        Depth::D2 => &[
            "int ***m = malloc(8);",
            "if (!m) return 0;",
            "int **n = malloc(8);",
            "if (!n) return 0;",
            "*m = n;",
        ],
    };
    let slot = match depth {
        Depth::D0 => "s",
        Depth::D1 => "*m",
        Depth::D2 => "**m",
    };
    (made.iter().map(|line| (*line).to_owned()).collect(), slot)
}

/// What holds the slot, which `Route::Realloc` grows, and the statement that
/// puts the grown block back where it was.
fn holder(depth: Depth) -> (&'static str, &'static str) {
    match depth {
        Depth::D0 => unreachable!("a local is not grown; see `NOT_WRITTEN`"),
        Depth::D1 => ("m", "m = g_;"),
        Depth::D2 => ("n", "*m = g_;"),
    }
}

/// The statements that put `value`, an `int *`, into the slot by `route`.
fn put(route: Route, depth: Depth, slot: &str, value: &str) -> Vec<String> {
    match route {
        Route::Store => vec![format!("{slot} = {value};")],
        Route::Memcpy => vec![
            format!("int *v_ = {value};"),
            format!("memcpy(&{slot}, &v_, 8);"),
        ],
        Route::Ret => vec![format!("{slot} = pass({value});")],
        Route::Realloc => {
            let (grown, back) = holder(depth);
            vec![
                format!("{slot} = {value};"),
                format!("int **g_ = realloc({grown}, 16);"),
                "if (!g_) return 0;".to_owned(),
                back.to_owned(),
            ]
        }
        Route::Call => unreachable!("only `Belief::Width` writes through a call; see `width`"),
    }
}

/// The route on every path, or on one arm with a null on the other.
fn shaped(shape: Shape, slot: &str, body: Vec<String>) -> Vec<String> {
    match shape {
        Shape::Both => body,
        Shape::One => {
            let mut lines = vec!["if (k) {".to_owned()];
            lines.extend(body.into_iter().map(|line| format!("    {line}")));
            lines.push("} else {".to_owned());
            lines.push(format!("    {slot} = 0;"));
            lines.push("}".to_owned());
            lines
        }
    }
}

/// A `char`'s address, converted through `void *`, for the hazard; an
/// `int`'s for the control. Through a call, one taking `char **` or `int **`
/// writes it.
fn width(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines = vec!["char c = 0;".to_owned(), "int i = 0;".to_owned()];
    lines.extend(made);
    let body = if row.route == Route::Call {
        let address = match row.depth {
            Depth::D0 => "&s",
            Depth::D1 => "m",
            Depth::D2 => "n",
        };
        if hazard {
            vec![
                format!("void *p_ = {address};"),
                "get_narrow(p_);".to_owned(),
            ]
        } else {
            vec![format!("get_wide({address});")]
        }
    } else {
        let mut value = vec![];
        if hazard {
            value.push("void *vc = &c;".to_owned());
            value.push("int *v = vc;".to_owned());
        } else {
            value.push("int *v = &i;".to_owned());
        }
        value.extend(put(row.route, row.depth, slot, "v"));
        value
    };
    lines.extend(shaped(row.shape, slot, body));
    lines
}

/// Memory nothing wrote copied into the slot for the hazard; the same memory
/// written first for the control.
fn fresh(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines = made;
    lines.push("int **b = malloc(8);".to_owned());
    lines.push("if (!b) return 0;".to_owned());
    if !hazard {
        lines.push("int x = 0;".to_owned());
        lines.push("*b = &x;".to_owned());
    }
    let body = match row.route {
        Route::Store => vec![format!("{slot} = *b;")],
        Route::Memcpy => vec![format!("memcpy(&{slot}, b, 8);")],
        Route::Realloc => vec![
            "int **g_ = realloc(b, 16);".to_owned(),
            "if (!g_) return 0;".to_owned(),
            format!("{slot} = *g_;"),
        ],
        Route::Call | Route::Ret => unreachable!("see `NOT_WRITTEN`"),
    };
    lines.extend(shaped(row.shape, slot, body));
    lines
}

/// A loop's earlier allocation, which the loop's next `malloc` makes this
/// check lose, put in the slot and freed by the name it was put there under,
/// for the hazard; **the same program without the free** for the control, so
/// that the two differ in the free alone and the control asks that a lost
/// pointer held in memory is not doubted before anything may have freed it.
fn lost(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines: Vec<String> = {
        [
            "int *u = 0;",
            "int *q = 0;",
            "int t = 0;",
            "while (t < 2) {",
            "    u = q;",
            "    q = malloc(4);",
            "    if (!q) return 0;",
            "    t = t + 1;",
            "}",
            "int *v = u;",
            "if (!v) return 0;",
        ]
        .iter()
        .map(|line| (*line).to_owned())
        .collect()
    };
    lines.extend(made);
    let body = put(row.route, row.depth, slot, "v");
    lines.extend(shaped(row.shape, slot, body));
    if hazard {
        lines.push("free(v);".to_owned());
    }
    lines
}

/// A pointer put in the slot on a loop's first turn and freed, the site made
/// again on the next, for the hazard; the same with no free for the control.
fn stale(row: Row, hazard: bool) -> Vec<String> {
    let (made, slot) = container(row.depth);
    let mut lines = made;
    lines.push(format!("{slot} = 0;"));
    lines.push("int t = 0;".to_owned());
    lines.push("while (t < 2) {".to_owned());
    lines.push("    int *p = malloc(4);".to_owned());
    lines.push("    if (!p) return 0;".to_owned());
    lines.push("    *p = 1;".to_owned());
    lines.push("    if (t == 0) {".to_owned());
    // **Grown after the loop, not inside it**: a holder `realloc` moves on
    // one turn is doubted on the next, whichever turn the check takes it to
    // be, which made the control doubted for a reason of its own.
    let inside = if row.route == Route::Realloc {
        Route::Store
    } else {
        row.route
    };
    for line in put(inside, row.depth, slot, "p") {
        lines.push(format!("        {line}"));
    }
    if hazard {
        lines.push("        free(p);".to_owned());
    }
    lines.push("    }".to_owned());
    lines.push("    t = t + 1;".to_owned());
    lines.push("}".to_owned());
    if row.route == Route::Realloc {
        let (grown, back) = holder(row.depth);
        lines.push(format!("int **g_ = realloc({grown}, 16);"));
        lines.push("if (!g_) return 0;".to_owned());
        lines.push(back.to_owned());
    }
    lines
}

/// The whole program for one side of a row: the declarations, the belief's
/// statements, and the read back that every belief shares, through an
/// `int *` that is tested and written through.
fn program(row: Row, hazard: bool) -> String {
    let body = match row.belief {
        Belief::Width => width(row, hazard),
        Belief::Fresh => fresh(row, hazard),
        Belief::Lost => lost(row, hazard),
        Belief::Stale => stale(row, hazard),
    };
    let (_, slot) = container(row.depth);
    let mut text = String::from(DECLARATIONS);
    text.push_str("int f(int k) {\n");
    for line in body.iter().map(String::as_str).chain([
        format!("int *r = {slot};").as_str(),
        "if (!r) return 0;",
        "*r = 1;",
        "return 0;",
    ]) {
        text.push_str("    ");
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("}\n");
    text
}

/// What the compiler said about one program: its exit code and the code of
/// every report, with the message beside it.
#[derive(Debug)]
struct Answer {
    exit: i32,
    reports: Vec<(String, String)>,
}

impl Answer {
    /// Whether a report carries what `belief` is reported as.
    fn reports(&self, belief: Belief) -> bool {
        let (code, phrase) = mark(belief);
        self.reports.iter().any(|(said, message)| {
            said == code && phrase.is_none_or(|phrase| message.contains(phrase))
        })
    }

    /// `exit:CODE,CODE`, the codes sorted and once each, for the summary.
    fn summary(&self) -> String {
        let mut codes: Vec<&str> = self.reports.iter().map(|(code, _)| code.as_str()).collect();
        codes.sort_unstable();
        codes.dedup();
        format!(
            "{}:{}",
            self.exit,
            if codes.is_empty() {
                "-".to_owned()
            } else {
                codes.join(",")
            }
        )
    }
}

/// Run the compiler on `source` at `safety`, in a directory of its own,
/// removed afterwards.
///
/// **Its own per call, and not per row and side**: a row's test and the
/// summary write the same program at the same time, and a directory named by
/// the row had one run read the other's half-written file, a failure that came
/// and went between runs. Numbered by this process, as the corpus's are,
/// rather than named, since a Windows working directory has to stay short.
fn ask(name: &str, source: &str, safety: &str) -> Answer {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("safec-route-{}-{call}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a row's working directory can be made");
    std::fs::write(dir.join("f.c"), source).expect("a generated program can be written");
    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .current_dir(&dir)
        .args([
            "--color",
            "never",
            "--safety",
            safety,
            "--emit",
            "safety-ir",
        ])
        .args(["--target", "x86_64-pc-windows-msvc", "-o", "out", "f.c"])
        .output()
        .expect("the compiler binary was built for this test");
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reports = stderr
        .lines()
        .filter_map(|line| {
            let rest = line
                .strip_prefix("error[")
                .or_else(|| line.strip_prefix("warning["))?;
            let (code, message) = rest.split_once(']')?;
            Some((code.to_owned(), message.to_owned()))
        })
        .collect();
    let exit = output
        .status
        .code()
        .unwrap_or_else(|| panic!("row `{name}`: the compiler was killed by a signal"));
    Answer { exit, reports }
}

/// Which side of `row` disagrees with its belief's rule, if either.
fn disagreement(name: &str, row: Row) -> Option<Side> {
    let hazard = ask(name, &program(row, true), "memory");
    let control = ask(name, &program(row, false), "memory");
    match (hazard.reports(row.belief), control.reports(row.belief)) {
        (true, false) => None,
        (false, false) => Some(Side::Hazard),
        (true, true) => Some(Side::Control),
        (false, true) => Some(Side::Both),
    }
}

/// Ask a row its belief's rule, against what `ACCEPTED` lists for it.
///
/// Mutation: the checker's `memcpy` pours no width into what it copies into;
/// `width_memcpy_d1_both` fails, among the four rows at one and two levels.
/// Mutation: the copy carries no unwritten mark into a block;
/// `fresh_memcpy_d1_both` fails, with its one-arm row. Mutation: ask the
/// control's program for the hazard's; 46 rows fail. Mutation: list in
/// `ACCEPTED` a row that agrees; that row fails.
fn check(name: &str, row: Row) {
    let entry = ACCEPTED.iter().find(|(listed, _, _)| *listed == name);
    let listed = entry.map(|&(_, side, _)| side);
    let found = disagreement(name, row);
    if found != listed {
        let mark = mark(row.belief);
        let meaning = match (entry, found) {
            (Some((_, _, Why::Belief(why))), None) => format!(
                "THIS IS A REGRESSION: the control is not provable from the unit ({why}), \
                 and the row agreeing is the check saying safe about it. Restore the doubt, \
                 or supersede the record first."
            ),
            (Some((_, _, Why::Cost(why))), None) => {
                format!("The row now agrees ({why}): remove its entry from `ACCEPTED`.")
            }
            _ => "A row that newly disagrees is a defect, or a belief a record took, and is \
                  listed with the issue or record that says so."
                .to_owned(),
        };
        panic!(
            "row `{name}` disagrees on {found:?} where `ACCEPTED` lists {listed:?}.\n\
             The hazard must report {mark:?} and the control must not. {meaning}\n\n\
             hazard:\n{}\ncontrol:\n{}",
            program(row, true),
            program(row, false),
        );
    }
}

/// The table is every combination of its axes, less what `NOT_WRITTEN` says
/// cannot be written, once each, and each row's name is spelled from its axes,
/// so that a row left out is a failure rather than a gap and a name cannot
/// claim one row while testing another.
///
/// Mutation: delete a row from `routes!`. This fails, naming it.
#[test]
fn the_matrix_is_every_combination_of_its_axes() {
    let excused = |row: Row| {
        NOT_WRITTEN.iter().any(|unwritten| {
            !unwritten.why.is_empty()
                && unwritten.route == row.route
                && unwritten.depth.is_none_or(|depth| depth == row.depth)
                && unwritten.belief.is_none_or(|belief| belief == row.belief)
                && unwritten.shape.is_none_or(|shape| shape == row.shape)
        })
    };
    let mut missing = vec![];
    for belief in BELIEFS {
        for route in ROUTES {
            for depth in DEPTHS {
                for shape in SHAPES {
                    let row = Row {
                        belief,
                        route,
                        depth,
                        shape,
                    };
                    let listed = ROWS.iter().filter(|(_, listed)| *listed == row).count();
                    match (excused(row), listed) {
                        (false, 1) | (true, 0) => {}
                        _ => missing.push(format!("{} (listed {listed} times)", name_of(row))),
                    }
                }
            }
        }
    }
    let misnamed: Vec<&str> = ROWS
        .iter()
        .filter(|(name, row)| *name != name_of(*row))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        missing.is_empty(),
        "rows the axes call for that the table does not hold once: {missing:?}"
    );
    assert!(
        misnamed.is_empty(),
        "rows whose name is not spelled from their axes: {misnamed:?}"
    );
}

/// A row's name as its axes spell it.
fn name_of(row: Row) -> String {
    let belief = match row.belief {
        Belief::Width => "width",
        Belief::Fresh => "fresh",
        Belief::Lost => "lost",
        Belief::Stale => "stale",
    };
    let route = match row.route {
        Route::Store => "store",
        Route::Memcpy => "memcpy",
        Route::Realloc => "realloc",
        Route::Call => "call",
        Route::Ret => "ret",
    };
    let depth = match row.depth {
        Depth::D0 => "d0",
        Depth::D1 => "d1",
        Depth::D2 => "d2",
    };
    let shape = match row.shape {
        Shape::Both => "both",
        Shape::One => "one",
    };
    format!("{belief}_{route}_{depth}_{shape}")
}

/// `ACCEPTED` names only rows of the table, once each, and gives a reason.
#[test]
fn every_accepted_row_is_in_the_table_once_with_a_reason() {
    for (place, (name, _, why)) in ACCEPTED.iter().enumerate() {
        assert!(
            ROWS.iter().any(|(row, _)| row == name),
            "`ACCEPTED` names `{name}`, which is no row"
        );
        let (Why::Cost(why) | Why::Belief(why)) = why;
        assert!(!why.is_empty(), "`ACCEPTED` gives `{name}` no reason");
        assert!(
            ACCEPTED[place + 1..]
                .iter()
                .all(|(other, _, _)| other != name),
            "`ACCEPTED` names `{name}` twice"
        );
    }
}

/// Every program the matrix writes is C this frontend reads, built at
/// `--safety off` with nothing said, so that a template that is wrong about
/// C fails as itself rather than as a row that disagrees.
///
/// Mutation: have `width`'s control assign `&c`, a `char *`, to its `int *`
/// rather than `&i`; this fails, beside the rows that template writes.
#[test]
fn every_generated_program_is_c_this_frontend_reads() {
    let mut refused = vec![];
    for &(name, row) in ROWS {
        for (side, hazard) in [("hazard", true), ("control", false)] {
            let answer = ask(name, &program(row, hazard), "off");
            if answer.exit != 0 || !answer.reports.is_empty() {
                refused.push(format!("{name} {side}: {answer:?}"));
            }
        }
    }
    assert!(
        refused.is_empty(),
        "generated programs this frontend refuses:\n{}",
        refused.join("\n")
    );
}

/// The whole matrix's answers are what was blessed: one line per row, the
/// hazard's and the control's exit and codes.
///
/// **What finds a regression between two builds**, where the rows find a
/// defect in one: a change that silences a route and its plain store together
/// keeps every row agreeing with itself and moves this. See ADR-0062.
///
/// Mutation: change one line of `tests/routes.expected`. This fails.
#[test]
fn the_matrix_answers_what_was_blessed() {
    let mut lines: Vec<String> = ROWS
        .iter()
        .map(|&(name, row)| {
            let hazard = ask(name, &program(row, true), "memory");
            let control = ask(name, &program(row, false), "memory");
            format!(
                "{name} hazard={} control={}",
                hazard.summary(),
                control.summary()
            )
        })
        .collect();
    lines.sort();
    let actual = lines.join("\n") + "\n";
    let path = expected_path();
    if blessing() {
        std::fs::write(&path, &actual)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| {
            panic!(
                "{}: {error}; run with SAFEC_BLESS=1 to write it",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    if expected != actual {
        let moved: Vec<String> = expected
            .lines()
            .filter(|old| !actual.lines().any(|new| new == *old))
            .map(|old| format!("- {old}"))
            .chain(
                actual
                    .lines()
                    .filter(|new| !expected.lines().any(|old| old == *new))
                    .map(|new| format!("+ {new}")),
            )
            .collect();
        panic!(
            "the matrix answers differently from {}:\n{}\n\nRead the difference, then run \
             with SAFEC_BLESS=1 to write what it answers now.",
            path.display(),
            moved.join("\n")
        );
    }
}

fn expected_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("routes.expected")
}

/// Whether to rewrite the summary, refused under CI, as the corpus's own
/// blessing is in `tests/cases.rs`: a run that rewrites its own expectations
/// proves nothing.
fn blessing() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    let asked = set("SAFEC_BLESS");
    assert!(
        !(asked && set("CI")),
        "SAFEC_BLESS is set under CI. Expected output is written by a person who then reads the diff."
    );
    asked
}
