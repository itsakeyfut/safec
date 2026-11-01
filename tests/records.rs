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

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// The record's text with everything that may still change taken out.
///
/// In this order:
///
/// 1. The text is read line by line with `str::lines`, which ends a line at
///    CRLF as at LF, since a Windows checkout under `core.autocrlf` and a Linux
///    runner must hash the same record the same way.
/// 2. The front matter's `status:` line goes. `date:` and `decision-makers:`
///    stay.
/// 3. The `### Confirmation` subsection goes, from that exact heading to the
///    next `##` or `###` heading outside a fenced block: what guards a decision
///    is a fact about the tree now, which the README obliges the record to keep
///    current.
/// 4. Every inline code span becomes an empty one, since the README obliges a
///    pointer to code to follow the code when it moves. Spans are read over a
///    whole paragraph, the lines between two blank lines, headings or fences,
///    joined by spaces as Markdown joins them: a span may wrap a line, and read
///    a line at a time the closing backtick of one span pairs with the opening
///    backtick of the next and the prose between them goes unhashed. A fenced
///    block is left as it is: a C program in a record is evidence, not a
///    pointer.
/// 5. Every link target becomes empty, for the same reason. A link's text
///    stays.
/// 6. Every run of whitespace becomes one space, so rewrapping a paragraph is
///    not a change of reasoning.
///
/// So prose written inside a code span, a link target or Confirmation is not
/// frozen. That is the price of letting pointers and guards move, and it is
/// in the README, where a reviewer reads what may still change.
fn reasoning(text: &str) -> String {
    let mut kept = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut front = 0;
    let mut in_confirmation = false;
    let mut in_fence = false;
    for line in text.lines() {
        if line == "---" && front < 2 {
            front += 1;
            flush(&mut paragraph, &mut kept);
            kept.push(line.to_owned());
            continue;
        }
        if front == 1 && line.starts_with("status:") {
            continue;
        }
        let fence = line.starts_with("```");
        if !in_fence && !fence && (line.starts_with("## ") || line.starts_with("### ")) {
            in_confirmation = line.trim_end() == "### Confirmation";
        }
        if in_confirmation {
            // Still followed through Confirmation, so a heading inside a
            // fence there does not end it.
            if fence {
                in_fence = !in_fence;
            }
            continue;
        }
        if fence || in_fence {
            flush(&mut paragraph, &mut kept);
            kept.push(line.to_owned());
            if fence {
                in_fence = !in_fence;
            }
        } else if line.trim().is_empty() || line.starts_with('#') {
            flush(&mut paragraph, &mut kept);
            kept.push(line.to_owned());
        } else {
            paragraph.push(line);
        }
    }
    flush(&mut paragraph, &mut kept);

    kept.join("\n")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The paragraph so far, its spans and link targets emptied, onto `kept`.
fn flush(paragraph: &mut Vec<&str>, kept: &mut Vec<String>) {
    if !paragraph.is_empty() {
        let joined = paragraph.join(" ");
        kept.push(without_link_targets(&without_code_spans(&joined)));
        paragraph.clear();
    }
}

/// The text with every inline code span emptied, however many backticks
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

/// The text with every link target, `](...)`, emptied.
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

/// The status in the record's front matter, without its quotes, single or
/// double as YAML allows either. Only the
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
        .map(|value| value.trim().trim_matches(['"', '\'']).to_owned())
}

/// Every problem with the records against the lock: a settled record with no
/// line, a line whose hash differs, a line for a record that does not exist,
/// a record with no status, and a line of the lock that is not exactly
/// `NNNN <16 lowercase hex digits>` or repeats a number. `records` is each
/// record's file name and text.
///
/// The lock is read strictly because a line it misreads is reported as a
/// change of reasoning, which sends a reader to a record that did not move.
fn check(records: &[(String, String)], lock: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut locked = BTreeMap::new();
    for (index, line) in lock.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let parsed = line.split_once(' ').filter(|(number, hash)| {
            number.len() == 4
                && number.bytes().all(|b| b.is_ascii_digit())
                && hash.len() == 16
                && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
        let Some((number, hash)) = parsed else {
            problems.push(format!(
                "line {} of docs/adr/frozen.txt is not `NNNN <16 lowercase hex digits>`: {line:?}",
                index + 1
            ));
            continue;
        };
        if locked.insert(number.to_owned(), hash.to_owned()).is_some() {
            problems.push(format!(
                "docs/adr/frozen.txt has two lines for ADR-{number}"
            ));
        }
    }

    let mut seen = BTreeSet::new();
    for (name, text) in records {
        let number = name[..4].to_owned();
        seen.insert(number.clone());
        let Some(status) = status(text) else {
            problems.push(format!("{name} has no status line"));
            continue;
        };
        // A record with a line is held to it whatever its status now says:
        // setting a settled record back to `proposed` does not unfreeze it.
        let hash = hash_of(text);
        match locked.get(&number) {
            None if status == "proposed" => {}
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
    problems
}

/// The tree's own records against the tree's own lock. What `check` decides
/// is held by the tests after it, over records written in the test; this one
/// fails when a record in `docs/adr` is edited.
#[test]
fn a_settled_record_s_reasoning_is_what_it_was_when_it_was_settled() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let adr = root.join("docs/adr");
    let lock = fs::read_to_string(adr.join("frozen.txt")).expect("docs/adr/frozen.txt reads");

    let mut names: Vec<_> = fs::read_dir(&adr)
        .expect("docs/adr lists")
        .map(|entry| entry.expect("an entry of docs/adr").file_name())
        .filter_map(|name| name.into_string().ok())
        .filter(|name| is_record(name) && adr.join(name).is_file())
        .collect();
    names.sort();
    let records: Vec<_> = names
        .into_iter()
        .map(|name| {
            let text = fs::read_to_string(adr.join(&name)).expect("a record reads");
            (name, text)
        })
        .collect();

    let problems = check(&records, &lock);
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

/// Whether a file in `docs/adr` is a record: four digits, a dash, and `.md`.
/// An editor's backup beside a record, `0003-x.md.orig`, is not one.
fn is_record(name: &str) -> bool {
    name.len() > 8
        && name[..4].bytes().all(|b| b.is_ascii_digit())
        && name.as_bytes()[4] == b'-'
        && name.ends_with(".md")
}

/// Mutation: the `.md` test dropped from `is_record`, so a backup beside a
/// record is hashed against that record's line.
#[test]
fn only_a_numbered_markdown_file_is_a_record() {
    assert!(is_record("0060-an-access-is-proved.md"));
    assert!(!is_record("0060-an-access-is-proved.md.orig"));
    assert!(!is_record("0060-an-access-is-proved.md~"));
    assert!(!is_record("adr-template.md"));
    assert!(!is_record("frozen.txt"));
}

/// Mutation: the length and the digits of the hash both left unchecked, so a
/// note after it is taken as part of the hash and reported as a change of
/// reasoning. Either check alone refuses this line.
#[test]
fn a_lock_line_with_anything_after_its_hash_is_named_as_malformed() {
    let lock = format!("{} # a note\n", line_for("0001", RECORD).trim_end());
    let problems = check(&[("0001-a.md".to_owned(), RECORD.to_owned())], &lock);
    // The malformed line, then the record it was meant for, which has none.
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[1].contains("has no line"), "{problems:?}");
    assert!(
        problems[0].starts_with("line 1 of docs/adr/frozen.txt is not"),
        "{problems:?}"
    );
}

/// Mutation: a repeated number let through, so the later line silently
/// replaces the earlier.
#[test]
fn a_number_with_two_lock_lines_is_named() {
    let lock = format!("{}{}", line_for("0001", RECORD), line_for("0001", RECORD));
    assert_eq!(
        check(&[("0001-a.md".to_owned(), RECORD.to_owned())], &lock),
        ["docs/adr/frozen.txt has two lines for ADR-0001"]
    );
}

/// The lock line `check` wants for a record.
fn line_for(number: &str, text: &str) -> String {
    format!("{number} {}\n", hash_of(text))
}

/// Mutation: the hash comparison dropped, so a record whose reasoning moved
/// passes.
#[test]
fn a_settled_record_whose_reasoning_moved_is_named() {
    let lock = line_for("0001", RECORD);
    let edited = RECORD.replace("asks it once", "asks it twice");
    let problems = check(&[("0001-a.md".to_owned(), edited)], &lock);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].starts_with("ADR-0001's reasoning has changed"),
        "{problems:?}"
    );
}

/// Mutation: a settled record with no line taken to have the line it wants,
/// `locked.get(&number).or(Some(&hash))`.
#[test]
fn a_settled_record_with_no_line_is_named_with_the_line_to_add() {
    let problems = check(&[("0001-a.md".to_owned(), RECORD.to_owned())], "");
    let want = format!("`{}`", line_for("0001", RECORD).trim_end());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].ends_with(&want), "{problems:?}");
}

/// Mutation: the walk over the lock's own lines dropped, so a line for a
/// record that is gone passes.
#[test]
fn a_line_for_a_record_that_does_not_exist_is_named() {
    let problems = check(&[], &line_for("0999", RECORD));
    assert_eq!(
        problems,
        ["docs/adr/frozen.txt has a line for ADR-0999, and there is no such record"]
    );
}

/// Mutation: `proposed` checked like any other status, so a record still
/// being written needs a line.
#[test]
fn a_proposed_record_needs_no_line() {
    let proposed = RECORD.replace("status: \"accepted\"", "status: \"proposed\"");
    assert_eq!(
        check(&[("0001-a.md".to_owned(), proposed)], ""),
        Vec::<String>::new()
    );
}

/// Mutation: a record with a line skipped when its status is `proposed`, as
/// it was before review, so setting a settled record back to `proposed` and
/// then editing it passes.
#[test]
fn a_settled_record_set_back_to_proposed_is_still_held_to_its_line() {
    let lock = line_for("0001", RECORD);
    let edited = RECORD
        .replace("status: \"accepted\"", "status: \"proposed\"")
        .replace("asks it once", "asks it twice");
    let problems = check(&[("0001-a.md".to_owned(), edited)], &lock);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].starts_with("ADR-0001's reasoning has changed"),
        "{problems:?}"
    );
}

/// Mutation: a record with no status skipped in silence, so damaged front
/// matter unfreezes it.
#[test]
fn a_record_with_no_status_is_named() {
    let broken = RECORD.replace("status: \"accepted\"\n", "");
    assert_eq!(
        check(
            &[("0001-a.md".to_owned(), broken)],
            &line_for("0001", RECORD)
        ),
        ["0001-a.md has no status line"]
    );
}

/// Mutation: only double quotes taken off, so `'proposed'` is a status of its
/// own and is asked for a line.
#[test]
fn a_status_in_single_quotes_is_read_as_one_in_double_quotes() {
    let single = RECORD.replace("status: \"accepted\"", "status: 'proposed'");
    assert_eq!(status(&single).as_deref(), Some("proposed"));
}

/// Mutation: the status read from anywhere in the record rather than from its
/// front matter, so a line of prose starting with `status:` decides it.
#[test]
fn a_status_is_read_from_the_front_matter_alone() {
    let proposed = RECORD
        .replace("status: \"accepted\"", "")
        .replace("# A title\n", "# A title\n\nstatus: accepted, as prose\n");
    assert_eq!(status(&proposed), None);
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

/// Mutation: step 2 drops a `status:` line anywhere rather than in the front
/// matter alone, so prose starting with it is no longer reasoning.
#[test]
fn a_status_line_in_the_body_is_reasoning() {
    let with = RECORD.replace("# A title\n", "# A title\n\nstatus: what was believed\n");
    let edited = with.replace("what was believed", "what is believed now");
    assert_ne!(reasoning(&edited), reasoning(&with));
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

/// Mutation: `str::lines` replaced by `split('\n')` and step 6 left out, so
/// the carriage return stays in what is hashed.
#[test]
fn a_record_checked_out_with_crlf_has_the_reasoning_it_has_with_lf() {
    assert_eq!(reasoning(&RECORD.replace('\n', "\r\n")), reasoning(RECORD));
}

/// Mutation: `reasoning` answers the empty string.
#[test]
fn a_changed_word_in_the_decision_outcome_changes_the_reasoning() {
    let edited = RECORD.replace("asks it once", "asks it twice");
    assert_ne!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: step 3 never ends, so everything after Confirmation goes with
/// it.
#[test]
fn a_changed_word_in_the_consequences_changes_the_reasoning() {
    let edited = RECORD.replace("it is one place", "it is two places");
    assert_ne!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: a line holding a code span is dropped whole, rather than the
/// span alone.
#[test]
fn a_changed_word_in_the_considered_options_changes_the_reasoning() {
    let edited = RECORD.replace("keep `old_name` where it is", "move `old_name` elsewhere");
    assert_ne!(reasoning(&edited), reasoning(RECORD));
}

/// Mutation: spans read a line at a time, as they were before review, so a
/// span that wraps a line keeps what is in it.
#[test]
fn a_renamed_code_span_that_wraps_a_line_leaves_the_reasoning_as_it_was() {
    let with = RECORD.replace(
        "the walk in `memory::report`",
        "the walk in `memory::\nreport`",
    );
    let edited = with.replace("`memory::\nreport`", "`reads::\nwalk`");
    assert_eq!(reasoning(&edited), reasoning(&with));
}

/// Mutation: spans read a line at a time, so the closing backtick of a span
/// that wraps pairs with the opening one of the next span on its line and the
/// words between them are not hashed.
#[test]
fn prose_between_a_wrapped_span_and_the_next_changes_the_reasoning() {
    let with = RECORD.replace(
        "the walk in `memory::report` asks it once,",
        "the walk in `memory::\nreport` and then `known` asks it once,",
    );
    let edited = with.replace("and then", "but never");
    assert_ne!(reasoning(&edited), reasoning(&with));
}

/// Mutation: a heading inside a fenced block read as a heading, so a `###`
/// line in a code sample ends Confirmation or starts one.
#[test]
fn a_heading_inside_a_fenced_block_is_not_one() {
    let with = RECORD.replace(
        "`a_test_name` fails.",
        "`a_test_name` fails.\n\n```text\n### Consequences\n```\n\nStill Confirmation.",
    );
    let edited = with.replace("Still Confirmation.", "Edited, and still Confirmation.");
    assert_eq!(reasoning(&edited), reasoning(&with));
}

/// Mutation: any heading starting `### Confirmation` taken as Confirmation,
/// so `### Confirmation of the premise` goes unhashed.
#[test]
fn only_the_confirmation_heading_itself_starts_confirmation() {
    let with = RECORD.replace(
        "### Consequences",
        "### Confirmation of the premise\n\nWhat was believed.\n\n### Consequences",
    );
    let edited = with.replace("What was believed.", "What is believed now.");
    assert_ne!(reasoning(&edited), reasoning(&with));
}

/// Mutation: an opening run matched by a longer closing run, so `a``b` closes
/// at the double backtick; or a span emptied to one pair of backticks whatever
/// delimited it.
#[test]
fn a_span_closes_only_at_a_run_as_long_as_the_one_that_opened_it() {
    assert_eq!(without_code_spans("`a``b`"), "``");
    assert_eq!(without_code_spans("``a`b``"), "````");
}

/// Mutation: only the first link target on a line emptied.
#[test]
fn every_link_target_on_a_line_is_emptied() {
    let with = RECORD.replace(
        "see [the record](./0017-a.md).",
        "see [the record](./0017-a.md) and [the other](./0018-b.md).",
    );
    let edited = with.replace("./0018-b.md", "./0018-renamed.md");
    assert_eq!(reasoning(&edited), reasoning(&with));
}

/// Mutation: a fenced block's lines dropped.
#[test]
fn a_changed_line_in_a_fenced_block_changes_the_reasoning() {
    let edited = RECORD.replace("free(p);", "free(q);");
    assert_ne!(reasoning(&edited), reasoning(RECORD));
}
