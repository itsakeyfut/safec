//! A settled record's reasoning is what it was when it was settled.
//!
//! `docs/adr/README.md` says a record's reasoning is not rewritten: a decision
//! reversed or extended is a new record that supersedes it. Nothing held that,
//! and the memory records grew a paragraph per issue instead, until a reader
//! could no longer tell which decision held or which routes were still unnamed.
//!
//! So each record whose status is anything but `proposed` has a line in
//! `docs/adr/frozen.txt`: its number and a hash of its reasoning. This test
//! recomputes the hash and fails where the two differ. The lock can be edited,
//! and that is deliberate: a changed line there is the act of reversing the
//! rule, visible in the diff, which is what a reviewer is asked to look for.
//!
//! What is hashed is everything except what the README says still changes once
//! a record is accepted: its status line, its Confirmation, and its pointers to
//! code and to other documents. See [`reasoning`].

use std::fs;
use std::path::Path;

/// The record's text with everything that may still change taken out.
///
/// In this order:
///
/// 1. CRLF becomes LF, since a Windows checkout under `core.autocrlf` and a
///    Linux runner must hash the same record the same way.
/// 2. The front matter's `status:` line goes. `date:` and `decision-makers:`
///    stay.
/// 3. The `### Confirmation` subsection goes, from its heading to the next
///    `##` or `###` heading: what guards a decision is a fact about the tree
///    now, which the README obliges the record to keep current.
/// 4. Every inline code span becomes an empty one, since the README obliges a
///    pointer to code to follow the code when it moves. A fenced block is left
///    as it is: a C program in a record is evidence, not a pointer.
/// 5. Every link target becomes empty, for the same reason. A link's text
///    stays.
/// 6. Every run of whitespace becomes one space, so rewrapping a paragraph is
///    not a change of reasoning.
fn reasoning(text: &str) -> String {
    let text = text.replace("\r\n", "\n");

    let mut kept = Vec::new();
    let mut front = 0;
    let mut in_confirmation = false;
    let mut in_fence = false;
    for line in text.lines() {
        if line == "---" && front < 2 {
            front += 1;
            kept.push(line.to_owned());
            continue;
        }
        if front == 1 && line.starts_with("status:") {
            continue;
        }
        if line.starts_with("```") {
            in_fence = !in_fence;
        }
        if !in_fence && (line.starts_with("## ") || line.starts_with("### ")) {
            in_confirmation = line.trim_end() == "### Confirmation";
        }
        if in_confirmation {
            continue;
        }
        if in_fence || line.starts_with("```") {
            kept.push(line.to_owned());
        } else {
            kept.push(without_link_targets(&without_code_spans(line)));
        }
    }

    kept.join("\n")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The line with every inline code span emptied, however many backticks
/// delimit it. A run of backticks with no closing run of the same length is
/// text, as Markdown reads it.
fn without_code_spans(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let open = chars[i..].iter().take_while(|&&c| c == '`').count();
        let after = i + open;
        let close = (after..chars.len()).find(|&j| {
            chars[j..].iter().take_while(|&&c| c == '`').count() == open
                && (j == 0 || chars[j - 1] != '`')
        });
        match close {
            Some(j) => {
                out.push_str(&"`".repeat(open * 2));
                i = j + open;
            }
            None => {
                out.extend(&chars[i..after]);
                i = after;
            }
        }
    }
    out
}

/// The line with every link target, `](...)`, emptied.
fn without_link_targets(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find("](") {
        out.push_str(&rest[..at + 2]);
        let after = &rest[at + 2..];
        match after.find(')') {
            Some(end) => rest = &after[end..],
            None => {
                rest = after;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// FNV-1a over 64 bits. Written out because the hash has to be the same on
/// every platform and every toolchain, and `std`'s hasher promises neither.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The hash a record's lock line carries.
fn hash_of(text: &str) -> String {
    format!("{:016x}", fnv1a64(reasoning(text).as_bytes()))
}

/// The status in the record's front matter, without its quotes. Only the
/// front matter is read: a record's prose may well start a line with
/// "status".
fn status(text: &str) -> Option<String> {
    let text = text.replace("\r\n", "\n");
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return None;
    }
    lines
        .take_while(|line| *line != "---")
        .find_map(|line| line.strip_prefix("status:"))
        .map(|value| value.trim().trim_matches('"').to_owned())
}

/// Mutations, each failing this test alone:
///
/// - the hash comparison dropped, so a changed word in ADR-0060's Decision
///   Outcome passes;
/// - a settled record with no line in `frozen.txt` passing;
/// - a line naming a record that does not exist passing.
#[test]
fn a_settled_record_s_reasoning_is_what_it_was_when_it_was_settled() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let adr = root.join("docs/adr");
    let lock = fs::read_to_string(adr.join("frozen.txt")).expect("docs/adr/frozen.txt reads");

    let mut locked = std::collections::BTreeMap::new();
    for line in lock.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (number, hash) = line
            .split_once(' ')
            .unwrap_or_else(|| panic!("a line of frozen.txt is not `NNNN <hash>`: {line:?}"));
        locked.insert(number.to_owned(), hash.trim().to_owned());
    }

    let mut problems = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut names: Vec<_> = fs::read_dir(&adr)
        .expect("docs/adr lists")
        .map(|entry| entry.expect("an entry of docs/adr").file_name())
        .filter_map(|name| name.into_string().ok())
        .filter(|name| name.len() > 5 && name[..4].bytes().all(|b| b.is_ascii_digit()))
        .collect();
    names.sort();

    for name in names {
        let number = name[..4].to_owned();
        seen.insert(number.clone());
        let text = fs::read_to_string(adr.join(&name)).expect("a record reads");
        let Some(status) = status(&text) else {
            problems.push(format!("{name} has no status line"));
            continue;
        };
        if status == "proposed" {
            continue;
        }
        let hash = hash_of(&text);
        match locked.get(&number) {
            None => problems.push(format!(
                "ADR-{number} is {status} and has no line in docs/adr/frozen.txt. \
                 A record settled now adds its own line: `{number} {hash}`"
            )),
            Some(want) if *want != hash => problems.push(format!(
                "ADR-{number}'s reasoning has changed since it was settled. A decision \
                 reversed or extended is a new record that supersedes this one, and a \
                 decision applied to one more route is a row in its route table \
                 (docs/adr/README.md). Its status, its Confirmation, a code span and a \
                 link target may change and are not what moved"
            )),
            Some(_) => {}
        }
    }
    for number in locked.keys() {
        if !seen.contains(number) {
            problems.push(format!(
                "docs/adr/frozen.txt has a line for ADR-{number}, and there is no such record"
            ));
        }
    }

    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

/// A small record in the shape the template gives, for the tests below.
const RECORD: &str = "---\n\
status: \"accepted\"\n\
date: 2026-11-01\n\
decision-makers: someone\n\
---\n\
\n\
# A title\n\
\n\
## Considered Options\n\
\n\
* keep `old_name` where it is\n\
\n\
## Decision Outcome\n\
\n\
Chosen because the walk in `memory::report` asks it once,\n\
see [the record](./0017-a.md).\n\
\n\
```c\n\
free(p);\n\
```\n\
\n\
### Confirmation\n\
\n\
`a_test_name` fails.\n\
\n\
### Consequences\n\
\n\
* Good, because it is one place.\n";

/// Mutation: step 3 left out, so Confirmation is hashed.
#[test]
fn an_edit_inside_confirmation_leaves_the_reasoning_as_it_was() {
    let edited = RECORD.replace(
        "`a_test_name` fails.",
        "`a_test_name` fails, and so does `another_test_name`.",
    );
    assert_eq!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: step 2 left out, so the status line is hashed.
#[test]
fn a_status_change_leaves_the_reasoning_as_it_was() {
    let edited = RECORD.replace("status: \"accepted\"", "status: \"superseded by ADR-0099\"");
    assert_eq!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: step 4 left out, so code spans are hashed.
#[test]
fn a_renamed_code_span_leaves_the_reasoning_as_it_was() {
    let edited = RECORD.replace("`memory::report`", "`reads`");
    assert_eq!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: step 5 left out, so link targets are hashed.
#[test]
fn a_changed_link_target_leaves_the_reasoning_as_it_was() {
    let edited = RECORD.replace("./0017-a.md", "./0017-renamed.md");
    assert_eq!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: step 6 left out, so line breaks are hashed.
#[test]
fn a_rewrapped_paragraph_leaves_the_reasoning_as_it_was() {
    let edited = RECORD.replace("asks it once,\nsee", "asks it\nonce, see");
    assert_eq!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: step 1 left out, so a CRLF checkout hashes differently.
#[test]
fn a_record_checked_out_with_crlf_has_the_reasoning_it_has_with_lf() {
    assert_eq!(reasoning(&RECORD.replace('\n', "\r\n")), reasoning(RECORD));
}

/// Mutation: `reasoning` answers the empty string, or step 3 drops every
/// section after the first `###` rather than Confirmation alone.
#[test]
fn a_changed_word_in_the_decision_outcome_changes_the_reasoning() {
    let edited = RECORD.replace("asks it once", "asks it twice");
    assert_ne!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: step 3 drops from `## Decision Outcome` to the end rather than
/// Confirmation alone, so Consequences goes with it.
#[test]
fn a_changed_word_in_the_consequences_changes_the_reasoning() {
    let edited = RECORD.replace("it is one place", "it is two places");
    assert_ne!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: the text of the considered options is dropped along with its
/// code span, rather than the span alone.
#[test]
fn a_changed_word_in_the_considered_options_changes_the_reasoning() {
    let edited = RECORD.replace("keep `old_name` where it is", "move `old_name` elsewhere");
    assert_ne!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: a fenced block's lines go through step 4 and 5 like prose, or are
/// dropped.
#[test]
fn a_changed_line_in_a_fenced_block_changes_the_reasoning() {
    let edited = RECORD.replace("free(p);", "free(q);");
    assert_ne!(reasoning(&edited), reasoning(RECORD));
}
