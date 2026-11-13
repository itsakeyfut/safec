//! Comparing what two builds of `safec` report on one program, for the corpus
//! and the route matrix alike.
//!
//! **Why it exists.** Each harness compares a build against its own expected
//! output, so a change that makes a reported program quiet and is then blessed
//! has made the silence the expected answer. Three such silences on #918's
//! branch were found by review rather than by a test: a block written at one
//! element read at another (d91d86fc), a copy into a block of two believed
//! written whole (#925), and six cases blessed quiet (c4cc026d). Each was a
//! program the older build reported and the newer one did not, which is a
//! question a machine can ask.
//!
//! **How it is asked.** With `SAFEC_COMPARE_WITH` naming an older `safec`, each
//! program a harness runs is run on that build too, and the harness fails,
//! naming the program, where the older build said something the newer one does
//! not. A diagnostic is keyed by its level, its code, where it points and
//! whether it was proved, so a report that moved to another line, or a proof
//! weakened to a doubt, counts as one lost. What it says is not keyed, so a
//! change of words is not.
//!
//! `SAFEC_COMPARE_ALLOWED` may name a file of programs that may lose what they
//! said, one per line, the name and then the reason. `SAFEC_COMPARE_REPORT`
//! may name a file the comparison appends to: every program whose name says
//! its quiet is deliberate (`_is_not_doubted`, `_is_not_reported`) and which
//! the newer build reports, so that how many beliefs a change gave up is a
//! count rather than an impression.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// One diagnostic, as the comparison keys it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Diagnostic {
    /// `error` or `warning`, with its code where it has one: `error[SC0411]`.
    pub(crate) opens: String,
    /// Where it points, `file:line:col`, or nothing where it points nowhere.
    pub(crate) at: String,
    /// Whether it was proved: an unproven conclusion carries a note that says
    /// it could not be, which the renderer gives every one and no proof.
    pub(crate) proved: bool,
}

/// The diagnostics in a run's `stderr`, in the order they were printed.
pub(crate) fn diagnostics(stderr: &str) -> Vec<Diagnostic> {
    let mut found: Vec<Diagnostic> = Vec::new();
    for line in stderr.lines() {
        if line.starts_with("error") || line.starts_with("warning") {
            found.push(Diagnostic {
                opens: line.split(':').next().unwrap_or(line).to_owned(),
                at: String::new(),
                proved: true,
            });
        } else if let Some(last) = found.last_mut() {
            if last.at.is_empty() {
                if let Some((_, rest)) = line.split_once("─[ ") {
                    last.at = rest.trim_end().trim_end_matches(']').trim_end().to_owned();
                }
            }
            if line.contains("could not be proven") {
                last.proved = false;
            }
        }
    }
    found
}

/// What `older` said that `newer` does not, counted as a multiset: two of one
/// diagnostic before and one after is one lost.
pub(crate) fn lost(older: &[Diagnostic], newer: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut left: BTreeMap<&Diagnostic, usize> = BTreeMap::new();
    for diagnostic in newer {
        *left.entry(diagnostic).or_insert(0) += 1;
    }
    let mut lost = Vec::new();
    for diagnostic in older {
        // **A doubt is answered by a proof at the same place**: the newer
        // build saying more is not a silence.
        let proved = Diagnostic {
            proved: true,
            ..diagnostic.clone()
        };
        let answered = [diagnostic, &proved]
            .into_iter()
            .any(|key| match left.get_mut(key) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    true
                }
                _ => false,
            });
        if !answered {
            lost.push(diagnostic.clone());
        }
    }
    lost
}

/// The older build to compare with, where one is named.
pub(crate) fn older_build() -> Option<PathBuf> {
    std::env::var_os("SAFEC_COMPARE_WITH")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Why `name` may lose what it said, where the allowed file lists it.
fn allowed(name: &str) -> Option<String> {
    let path = std::env::var_os("SAFEC_COMPARE_ALLOWED").filter(|path| !path.is_empty())?;
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", PathBuf::from(&path).display()));
    allowed_in(&text, name)
}

/// Why `name` may lose what it said, where `text`, an allowed file's
/// contents, lists it: one program per line, the name and then the reason.
pub(crate) fn allowed_in(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (case, reason) = line.trim().split_once(char::is_whitespace)?;
        (case == name).then(|| reason.trim().to_owned())
    })
}

/// Fail where the newer build lost something the older one said about
/// `name`, unless it is allowed; and record a deliberate quiet given up.
///
/// Measured: with the build before a library copy wrote only the slots it
/// covers as the older one, and the copy writing the block whole in this one,
/// `SAFEC_BLESS=1` stops at
/// `a_copy_of_one_pointer_into_a_block_of_two_is_reported_where_the_other_is_read`
/// and two more before anything of theirs is written.
pub(crate) fn compare(name: &str, older_stderr: &str, newer_stderr: &str) {
    let older = diagnostics(older_stderr);
    let newer = diagnostics(newer_stderr);
    let quiet_by_name = name.contains("_is_not_doubted") || name.contains("_is_not_reported");
    if quiet_by_name && older.is_empty() && !newer.is_empty() {
        if let Some(path) = std::env::var_os("SAFEC_COMPARE_REPORT").filter(|path| !path.is_empty())
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", PathBuf::from(&path).display()));
            writeln!(file, "{name}").expect("the comparison report can be written");
        }
    }
    if let Some(refusal) = refusal(name, &older, &newer, allowed(name)) {
        panic!("{refusal}");
    }
}

/// What [`compare`] fails with for `name`, where the newer build lost
/// something the older one said and `allowed` gives no reason it may: the
/// whole decision apart from the files it reads, so that a test can ask it.
pub(crate) fn refusal(
    name: &str,
    older: &[Diagnostic],
    newer: &[Diagnostic],
    allowed: Option<String>,
) -> Option<String> {
    let lost = lost(older, newer);
    if lost.is_empty() || allowed.is_some() {
        return None;
    }
    let lines: Vec<String> = lost
        .iter()
        .map(|d| {
            let proved = if d.proved { "proved" } else { "unproven" };
            format!("{} at {}, {proved}", d.opens, d.at)
        })
        .collect();
    Some(format!(
        "`{name}`: the older build said what this one does not:\n  {}\n\
         Read why; if it is right, list `{name}` with the reason in the file \
         SAFEC_COMPARE_ALLOWED names.",
        lines.join("\n  ")
    ))
}
