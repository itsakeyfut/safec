//! Directive lines read as `clang` reads them, compared spelling by spelling.
//!
//! Under #1359 eight holes where safec read a directive line differently
//! from `clang` were opened, and each was found by a review or a hunt, none
//! by the corpus: each review answer added a case for its own spelling, and
//! the next spelling was found by a person. So this generates the spellings.
//! Each of the directives in [`DIRECTIVES`] is written with one of the
//! characters in [`CHARACTERS`] at one place: before its `#`, between the `#`
//! and the name, touching the name's end, or at the end of the line, which
//! for `#ifdef X` touches the name it asks about. Each line is placed in a
//! group that is compiled and in one that is skipped, and its plain spelling
//! also in a conditional nested in a skipped group, beside declarations that
//! say which groups were taken, and `clang` is asked which it compiled. One
//! character at one place per line: two together, a comment between, a
//! splice and `%:` are not generated. Asked as it compiles, not as
//! `clang -E` preprocesses: the two disagree on white space outside ASCII
//! after a name, which `-E` refuses and a compile reads with a warning, and
//! what a program is, is what is compiled.
//!
//! safec agrees where it takes the same groups, or where it refuses the
//! program, which ADR-0134 allows. It disagrees where it builds a program
//! `clang` takes other groups in (row 6), or builds one `clang` refuses.
//! Today's disagreements are listed exactly in [`KNOWN`], each with the
//! issue that closes it, and a row that no longer disagrees fails too, so
//! the list only shrinks. Since a refusal agrees, two more things are held:
//! every plain spelling `clang` builds is built, but for those
//! [`PLAIN_REFUSED`] lists, and [`BUILT_BY_BOTH`] counts the programs both
//! built alike, so refusing more is a change somebody states.
//!
//! `clang` is found as the LLVM tests find it: where there is none the suite
//! is skipped, and `SAFEC_REQUIRE_LLVM`, which CI sets, makes that a failure.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// A directive: its name, what follows the name, and how it is placed so
/// that each reading of it is a program whose groups say which reading it
/// was.
struct Directive {
    name: &'static str,
    rest: &'static str,
    shape: Shape,
}

/// How a directive line is set among the conditionals around it.
#[derive(Clone, Copy)]
enum Shape {
    /// Opens a level: `#if`, `#ifdef`, `#ifndef`.
    Opens,
    /// Chooses the next group: `#elif`, `#else`, `#elifdef`.
    Chooses,
    /// Closes a level: `#endif`.
    Closes,
    /// Any other directive, which chooses no group.
    Other,
}

const DIRECTIVES: &[Directive] = &[
    Directive {
        name: "if",
        rest: " 1",
        shape: Shape::Opens,
    },
    Directive {
        name: "ifdef",
        rest: " X",
        shape: Shape::Opens,
    },
    Directive {
        name: "ifndef",
        rest: " Z",
        shape: Shape::Opens,
    },
    Directive {
        name: "elif",
        rest: " 1",
        shape: Shape::Chooses,
    },
    Directive {
        name: "else",
        rest: "",
        shape: Shape::Chooses,
    },
    Directive {
        name: "elifdef",
        rest: " X",
        shape: Shape::Chooses,
    },
    Directive {
        name: "elifndef",
        rest: " Z",
        shape: Shape::Chooses,
    },
    Directive {
        name: "endif",
        rest: "",
        shape: Shape::Closes,
    },
    Directive {
        name: "define",
        rest: " Y 1",
        shape: Shape::Other,
    },
    Directive {
        name: "undef",
        rest: " Y",
        shape: Shape::Other,
    },
    Directive {
        name: "include",
        rest: " \"a.h\"",
        shape: Shape::Other,
    },
    Directive {
        name: "pragma",
        rest: " once",
        shape: Shape::Other,
    },
    Directive {
        name: "line",
        rest: " 40",
        shape: Shape::Other,
    },
    // The null directive and a line marker, which ADR-0134 calls proved.
    Directive {
        name: "",
        rest: "",
        shape: Shape::Other,
    },
    Directive {
        name: "4",
        rest: " \"a.c\"",
        shape: Shape::Other,
    },
];

/// The target both compilers are asked about, the same on every host: how a
/// line is read depends on it, a Ctrl-Z ending the file only where
/// Microsoft's extensions are on, and run on each host's default target the
/// count of programs both built alike moved by the 52 Ctrl-Z rows on Linux.
const TARGET: &str = "x86_64-pc-windows-msvc";

/// What can stand beside a directive's name, by a label for reports.
const CHARACTERS: &[(&str, &str)] = &[
    ("nothing", ""),
    ("at", "@"),
    ("backtick", "`"),
    ("dollar", "$"),
    ("backslash", "\\"),
    ("backslash q", "\\q"),
    ("\\u alone", "\\u"),
    ("\\u short", "\\u00e"),
    ("\\u letter", "\\u00e9"),
    ("\\u space", "\\u00a0"),
    ("\\U letter", "\\U000000e9"),
    ("\\U past the range", "\\U00110000"),
    ("\\u{} letter", "\\u{e9}"),
    ("\\u{} space", "\\u{a0}"),
    ("\\u{} empty", "\\u{}"),
    ("\\N letter", "\\N{LATIN SMALL LETTER E WITH ACUTE}"),
    ("\\N space", "\\N{NO-BREAK SPACE}"),
    ("no-break space", "\u{a0}"),
    ("ideographic space", "\u{3000}"),
    ("U+180E", "\u{180e}"),
    ("line separator", "\u{2028}"),
    ("letter", "\u{e9}"),
    ("zero-width space", "\u{200b}"),
    ("NUL", "\u{0}"),
    ("control", "\u{1}"),
    ("substitute", "\u{1a}"),
    ("vertical tab", "\u{b}"),
    ("form feed", "\u{c}"),
];

/// Where the character stands on the line: the last is after what follows
/// the name, which for `#ifdef X` touches the name it asks about.
const PLACES: &[&str] = &[
    "before the #",
    "after the #",
    "after the name",
    "at the end",
];

/// Which kind of group the line is in: one compiled, one skipped, and one
/// nested in a conditional opened inside a skipped group, where only the
/// depth counts.
const GROUPS: &[&str] = &["compiled", "skipped", "nested"];

/// Today's disagreements, each as `[group, place, directive, character]`,
/// where `*` stands for any, with the issue that closes it. Deleting a row
/// the code now agrees on is the change that closes it; a row that matches
/// no disagreement fails.
/// How many generated programs `clang` and safec both built, taking the
/// same groups. A refusal agrees with `clang`, as ADR-0134 allows, so this
/// is what says how much is read rather than refused: a change that refuses
/// more, or reads more, moves it, and is stated by updating it.
const BUILT_BY_BOTH: usize = 1067;

/// The plain spellings safec refuses though `clang` builds them, each with
/// the issue that reads it.
const PLAIN_REFUSED: &[([&str; 4], &str)] = &[
    // C23's `#elifdef` and `#elifndef` are not read yet.
    (["*", "*", "#elifdef", "nothing"], "#1273"),
    (["*", "*", "#elifndef", "nothing"], "#1273"),
    // A null directive is refused in a group that is compiled.
    (["compiled", "*", "#(null)", "nothing"], "#1384"),
];

const KNOWN: &[([&str; 4], &str)] = &[
    // A Ctrl-Z ends the file for `clang` targeting MSVC, and this reads on.
    (
        ["compiled", "after the name", "#elif", "substitute"],
        "#1383",
    ),
    (
        ["compiled", "after the name", "#else", "substitute"],
        "#1383",
    ),
    (
        ["compiled", "after the name", "#pragma", "substitute"],
        "#1383",
    ),
    (["compiled", "at the end", "#define", "substitute"], "#1383"),
    (["compiled", "at the end", "#elif", "substitute"], "#1383"),
    (["compiled", "at the end", "#ifdef", "substitute"], "#1383"),
    (["compiled", "at the end", "#ifndef", "substitute"], "#1383"),
    (["compiled", "at the end", "#pragma", "substitute"], "#1383"),
    (["skipped", "after the name", "#4", "substitute"], "#1383"),
    (
        ["skipped", "after the name", "#define", "substitute"],
        "#1383",
    ),
    (
        ["skipped", "after the name", "#else", "substitute"],
        "#1383",
    ),
    (["skipped", "after the name", "#if", "substitute"], "#1383"),
    (
        ["skipped", "after the name", "#ifdef", "substitute"],
        "#1383",
    ),
    (
        ["skipped", "after the name", "#ifndef", "substitute"],
        "#1383",
    ),
    (
        ["skipped", "after the name", "#include", "substitute"],
        "#1383",
    ),
    (
        ["skipped", "after the name", "#line", "substitute"],
        "#1383",
    ),
    (
        ["skipped", "after the name", "#pragma", "substitute"],
        "#1383",
    ),
    (
        ["skipped", "after the name", "#undef", "substitute"],
        "#1383",
    ),
    (["skipped", "at the end", "#4", "substitute"], "#1383"),
    (["skipped", "at the end", "#define", "substitute"], "#1383"),
    (["skipped", "at the end", "#if", "substitute"], "#1383"),
    (["skipped", "at the end", "#ifdef", "substitute"], "#1383"),
    (["skipped", "at the end", "#ifndef", "substitute"], "#1383"),
    (["skipped", "at the end", "#include", "substitute"], "#1383"),
    (["skipped", "at the end", "#line", "substitute"], "#1383"),
    (["skipped", "at the end", "#pragma", "substitute"], "#1383"),
    (["skipped", "at the end", "#undef", "substitute"], "#1383"),
];

/// Whether `key`, a disagreement's `group / place / directive / character`,
/// is the one `known` lists.
fn listed(key: &str, known: &[&str; 4]) -> bool {
    let fields: Vec<&str> = key.split(" / ").collect();
    fields.len() == 4
        && fields
            .iter()
            .zip(known)
            .all(|(field, wanted)| *wanted == "*" || field == wanted)
}

/// The directive's line, with `character` at `place`.
fn line(directive: &Directive, character: &str, place: &str) -> String {
    let (before, between, after, end) = match place {
        "before the #" => (character, "", "", ""),
        "after the #" => ("", character, "", ""),
        "after the name" => ("", "", character, ""),
        _ => ("", "", "", character),
    };
    format!(
        "{before}#{between}{}{after}{}{end}",
        directive.name, directive.rest
    )
}

/// A program holding `line` in a `group`, declaring `in_a`, `in_b`, `in_c`
/// in the groups each reading of it would take.
fn program(directive: &Directive, line: &str, group: &str) -> String {
    if group == "nested" {
        return format!(
            "#define X 1\n#if 0\n#if 1\nint in_z;\n{line}\nint in_a;\n#endif\nint in_b;\n#endif\n\
             int in_c;\nint main(void) {{ return 0; }}\n"
        );
    }
    let compiled = group == "compiled";
    let body = match (directive.shape, compiled) {
        (Shape::Opens, true) => format!("{line}\nint in_a;\n#endif\nint in_b;\n"),
        (Shape::Opens, false) => {
            format!("#if 0\n{line}\nint in_a;\n#endif\nint in_b;\n#endif\nint in_c;\n")
        }
        (Shape::Chooses, true) => format!("#if 1\nint in_a;\n{line}\nint in_b;\n#endif\n"),
        (Shape::Chooses, false) => format!("#if 0\nint in_a;\n{line}\nint in_b;\n#endif\n"),
        (Shape::Closes, true) => {
            format!("#if 1\nint in_a;\n{line}\nint in_b;\n#endif\nint in_c;\n")
        }
        (Shape::Closes, false) => {
            format!("#if 0\nint in_a;\n{line}\nint in_b;\n#endif\nint in_c;\n")
        }
        (Shape::Other, true) => format!("{line}\nint in_a;\n"),
        (Shape::Other, false) => format!("#if 0\n{line}\n#endif\nint in_a;\n"),
    };
    format!("#define X 1\n{body}int main(void) {{ return 0; }}\n")
}

/// The marker declarations named in `text`.
fn markers(text: &str) -> BTreeSet<String> {
    ["in_a", "in_b", "in_c", "in_z"]
        .into_iter()
        .filter(|marker| text.contains(marker))
        .map(str::to_owned)
        .collect()
}

/// What one of the two compilers made of a program.
#[derive(Debug, PartialEq)]
enum Reading {
    /// Built it, compiling the groups declaring these.
    Built(BTreeSet<String>),
    /// Refused it.
    Refused,
}

/// What `clang` compiles of `file`: the declarations its syntax tree
/// holds, or a refusal.
fn clang(file: &Path) -> Reading {
    let output = Command::new("clang")
        .arg("-std=c17")
        .arg(format!("--target={TARGET}"))
        .arg("-fsyntax-only")
        .args(["-Xclang", "-ast-dump"])
        .arg(file)
        .stderr(Stdio::null())
        .output()
        .expect("clang runs");
    if output.status.success() {
        let dump = String::from_utf8_lossy(&output.stdout);
        let declared: String = dump
            .lines()
            .filter(|line| line.contains("VarDecl"))
            .collect::<Vec<_>>()
            .join("\n");
        Reading::Built(markers(&declared))
    } else {
        Reading::Refused
    }
}

fn safec(file: &Path) -> Reading {
    let output = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args(["--color", "never", "--emit", "ast", "--target", TARGET])
        .arg(file)
        .output()
        .expect("safec runs");
    match output.status.code() {
        Some(0) => Reading::Built(markers(&String::from_utf8_lossy(&output.stdout))),
        Some(1) => Reading::Refused,
        other => panic!(
            "safec exited {other:?} on {}:\n{}",
            file.display(),
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

/// Whether there is a `clang` on this machine; where there is none, say so,
/// and fail where `SAFEC_REQUIRE_LLVM` asks for one.
fn clang_or_skip() -> bool {
    let here = Command::new("clang")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    if !here {
        assert!(
            std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
            "SAFEC_REQUIRE_LLVM is set and there is no `clang` to compare directive lines with"
        );
        eprintln!("no `clang` on this machine: directive lines were not compared");
    }
    here
}

/// A directory of its own, removed however the test ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Each generated directive spelling is read as `clang` reads it, refused,
/// or listed in [`KNOWN`]; each plain one `clang` builds is built; and the
/// number both built alike is [`BUILT_BY_BOTH`].
///
/// Mutations: take a stray character touching a name as going on with it
/// (#1360's review), read white space outside ASCII after `#` as a space
/// (#1370), or read a universal character name past U+10FFFF as ending the
/// name (#1369's review); rows not in `KNOWN` disagree. Refuse every line in
/// a compiled group; `BUILT_BY_BOTH` and the plain spellings fail. Drop
/// `#elifndef` from the names that choose a group; its rows disagree. Act on
/// an `#else` nested in a skipped group; the nested rows disagree.
#[test]
fn generated_directive_spellings_are_read_as_clang_reads_them_or_refused() {
    if !clang_or_skip() {
        return;
    }
    let scratch =
        Scratch(std::env::temp_dir().join(format!("safec_directives_{}", std::process::id())));
    let _ = fs::remove_dir_all(&scratch.0);
    fs::create_dir_all(&scratch.0).expect("the temporary directory is writable");
    fs::write(scratch.0.join("a.h"), "").expect("the temporary directory is writable");

    let mut jobs = Vec::new();
    for group in GROUPS {
        for place in PLACES {
            for directive in DIRECTIVES {
                // At the end of a directive with nothing after its name is
                // touching the name, a place already generated.
                if *place == "at the end" && directive.rest.is_empty() {
                    continue;
                }
                for (label, character) in CHARACTERS {
                    // A nested group asks how the depth is counted, which
                    // the plain spelling asks.
                    if *group == "nested" && *label != "nothing" {
                        continue;
                    }
                    let key = format!(
                        "{group} / {place} / #{} / {label}",
                        if directive.name.is_empty() {
                            "(null)"
                        } else {
                            directive.name
                        }
                    );
                    let text = program(directive, &line(directive, character, place), group);
                    jobs.push((key, text));
                }
            }
        }
    }

    let disagreements = Mutex::new(Vec::new());
    // How many programs both built alike, and the plain spellings safec
    // refused though `clang` built them: a refusal agrees, so without these
    // a safec that refused everything would pass. Found by review.
    let built_by_both = Mutex::new(0usize);
    let plain_refused = Mutex::new(Vec::new());
    let next = Mutex::new(0usize);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let at = {
                        let mut next = next.lock().unwrap();
                        let at = *next;
                        *next += 1;
                        at
                    };
                    let Some((key, text)) = jobs.get(at) else {
                        break;
                    };
                    let file = scratch.0.join(format!("p{at}.c"));
                    fs::write(&file, text).expect("the temporary directory is writable");
                    let theirs = clang(&file);
                    let ours = safec(&file);
                    let agrees = match (&theirs, &ours) {
                        (_, Reading::Refused) => true,
                        (Reading::Built(a), Reading::Built(b)) => a == b,
                        (Reading::Refused, Reading::Built(_)) => false,
                    };
                    if let (Reading::Built(a), Reading::Built(b)) = (&theirs, &ours) {
                        if a == b {
                            *built_by_both.lock().unwrap() += 1;
                        }
                    }
                    if key.ends_with(" / nothing")
                        && matches!(theirs, Reading::Built(_))
                        && ours == Reading::Refused
                        && !PLAIN_REFUSED.iter().any(|(known, _)| listed(key, known))
                    {
                        plain_refused.lock().unwrap().push(key.clone());
                    }
                    if !agrees {
                        disagreements
                            .lock()
                            .unwrap()
                            .push((key.clone(), format!("clang {theirs:?}, safec {ours:?}")));
                    }
                }
            });
        }
    });

    let plain_refused = plain_refused.into_inner().unwrap();
    assert!(
        plain_refused.is_empty(),
        "safec refuses plain spellings clang builds:\n{}",
        plain_refused.join("\n")
    );
    let built_by_both = built_by_both.into_inner().unwrap();
    assert_eq!(
        built_by_both, BUILT_BY_BOTH,
        "programs both compilers built alike moved from {BUILT_BY_BOTH} to {built_by_both}: \
         a refusal added or lifted is a change to say, by updating `BUILT_BY_BOTH`"
    );
    let mut disagreements: Vec<(String, String)> = disagreements.into_inner().unwrap();
    disagreements.sort();
    let unexpected: Vec<String> = disagreements
        .iter()
        .filter(|(key, _)| !KNOWN.iter().any(|(known, _)| listed(key, known)))
        .map(|(key, how)| format!("{key}: {how}"))
        .collect();
    let stale: Vec<String> = KNOWN
        .iter()
        .filter(|(known, _)| !disagreements.iter().any(|(key, _)| listed(key, known)))
        .map(|(known, issue)| format!("{} ({issue})", known.join(" / ")))
        .collect();
    assert!(
        unexpected.is_empty() && stale.is_empty(),
        "{} disagreements not in KNOWN:\n{}\n\n{} KNOWN rows that now agree:\n{}",
        unexpected.len(),
        unexpected.join("\n"),
        stale.len(),
        stale.join("\n"),
    );
}
