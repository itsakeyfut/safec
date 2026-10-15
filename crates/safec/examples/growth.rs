//! How the memory check's time grows with the function it reads.
//!
//! `cargo run --release --example growth` writes three shapes of function at
//! 50, 100 and 200 allocating locals, checks each with `--emit safety-ir`,
//! and prints how long the check took. Every shape gives each local
//! `int *pK = malloc(4);` and a pointer to the next one, `int **qK = &p(K+1);`,
//! and writes through those pointers inside a loop:
//!
//! - **one loop**, every write in its body;
//! - **loops in sequence**, one loop per write;
//! - **branches in a loop**, every write behind its own `if`.
//!
//! Each shape stands for a way the walk can grow. The first visits each block
//! about once, so what it measures is the cost of one visit, which grows with
//! the value. The other two are the shapes whose visits grew faster than the
//! blocks did while the solver could walk a later loop before an earlier one
//! had settled. See ADR-0016.
//!
//! **It asserts nothing.** A time is a property of the machine it was taken
//! on, so this is a measurement to repeat rather than a test, and the guard on
//! the walk count is a test of its own in `crates/safec-ir/tests/written.rs`.
//! It is an example rather than a script so that every build of the examples
//! compiles it against the driver it measures.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use clap::Parser as _;

use safec::cli::Cli;
use safec::driver;

/// The sizes measured, in allocating locals.
const SIZES: [usize; 3] = [50, 100, 200];

#[derive(Clone, Copy)]
enum Shape {
    OneLoop,
    Sequence,
    Branches,
}

impl Shape {
    const ALL: [Shape; 3] = [Shape::OneLoop, Shape::Sequence, Shape::Branches];

    fn name(self) -> &'static str {
        match self {
            Shape::OneLoop => "one loop",
            Shape::Sequence => "loops in sequence",
            Shape::Branches => "branches in a loop",
        }
    }

    /// The C text of this shape with `n` allocating locals.
    fn source(self, n: usize) -> String {
        let mut c = String::from("void *malloc(int n);\nint f(int c) {\n");
        for k in 0..n {
            writeln!(c, "    int *p{k} = malloc(4);").expect("writing to a string");
        }
        for k in 0..n {
            writeln!(c, "    int **q{k} = &p{};", (k + 1) % n).expect("writing to a string");
        }
        match self {
            Shape::OneLoop => {
                c.push_str("    while (c) {\n");
                for k in 0..n {
                    writeln!(c, "        *q{k} = p{k};").expect("writing to a string");
                }
                c.push_str("        c = c - 1;\n    }\n");
            }
            Shape::Sequence => {
                for k in 0..n {
                    writeln!(c, "    while (c) {{ *q{k} = p{k}; c = c - 1; }}")
                        .expect("writing to a string");
                }
            }
            Shape::Branches => {
                c.push_str("    while (c) {\n");
                for k in 0..n {
                    writeln!(c, "        if (c) {{ *q{k} = p{k}; }}").expect("writing to a string");
                }
                c.push_str("        c = c - 1;\n    }\n");
            }
        }
        c.push_str("    return 0;\n}\n");
        c
    }
}

/// How long checking `source` took, and whether the run reported an error.
///
/// Through the command line's own parser, so that what is measured is what
/// `safec --emit safety-ir` does with the file.
fn measure(source: &str, at: &std::path::Path) -> (Duration, bool) {
    std::fs::write(at, source).expect("the temporary directory is writable");
    let arguments: [OsString; 4] = [
        "safec".into(),
        "--emit".into(),
        "safety-ir".into(),
        at.as_os_str().to_owned(),
    ];
    let options = Cli::parse_from(arguments)
        .into_options()
        .expect("the host is a machine this compiler knows");

    let started = Instant::now();
    let compiled = driver::compile(&options);
    (started.elapsed(), compiled.diagnostics.has_errors())
}

fn main() {
    let at = std::env::temp_dir().join(format!("safec-growth-{}.c", std::process::id()));

    println!(
        "| shape | {} |",
        SIZES.map(|n| format!("{n} locals")).join(" | ")
    );
    println!("|---|{}", "---|".repeat(SIZES.len()));
    for shape in Shape::ALL {
        let mut row = format!("| {} |", shape.name());
        for n in SIZES {
            let (took, failed) = measure(&shape.source(n), &at);
            write!(
                row,
                " {} ms{} |",
                took.as_millis(),
                if failed { " (failed)" } else { "" }
            )
            .expect("writing to a string");
        }
        println!("{row}");
    }

    let _ = std::fs::remove_file(&at);
}
