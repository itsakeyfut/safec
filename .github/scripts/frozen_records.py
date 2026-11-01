"""A settled record's reasoning is what it was when it was settled.

`docs/adr/README.md` says a record's reasoning is not rewritten: a decision
reversed or extended is a new record that supersedes it. Nothing held that,
and the memory records grew a paragraph per issue instead, until a reader
could no longer tell which decision held or which routes were still unnamed.

So each record whose status is anything but `proposed` has a line in
`docs/adr/frozen.txt`: its number and a hash of its reasoning. This script
recomputes the hash and fails where the two differ. The lock can be edited,
and that is deliberate: a changed line there is the act of reversing the
rule, visible in the diff, which is what a reviewer is asked to look for.

It lives here and runs as a CI job of its own, rather than in `cargo test`,
because it checks the repository's records and not the compiler: kept among
the compiler's tests it read as one of them.

Run from anywhere in the repository:

    python3 .github/scripts/frozen_records.py

It runs its own tests first, then checks the records, and exits non-zero
naming every problem. Only the standard library is used, and only
`python3 -I` is assumed.

What is hashed is everything except what the README says still changes once
a record is accepted: its status line, its Confirmation, and its pointers to
code and to other documents. See `reasoning`.
"""

import os
import sys
import unittest


def lines(text):
    """The text's lines, each ended at LF or CRLF, with no empty line after a
    final line break: the lines a Windows checkout under `core.autocrlf` and a
    Linux runner both give for one record."""
    out = text.split("\n")
    if out and out[-1] == "":
        out.pop()
    return [line[:-1] if line.endswith("\r") else line for line in out]


def reasoning(text):
    """The record's text with everything that may still change taken out.

    In this order:

    1. The text is read by `lines`, which ends a line at CRLF as at LF.
    2. The front matter's `status:` line goes. `date:` and
       `decision-makers:` stay.
    3. The `### Confirmation` subsection goes, from that exact heading to the
       next `##` or `###` heading outside a fenced block: what guards a
       decision is a fact about the tree now, which the README obliges the
       record to keep current.
    4. Every inline code span becomes an empty one, since the README obliges
       a pointer to code to follow the code when it moves. Spans are read
       over a whole paragraph, the lines between two blank lines, headings or
       fences, joined by spaces as Markdown joins them: a span may wrap a
       line, and read a line at a time the closing backtick of one span pairs
       with the opening backtick of the next and the prose between them goes
       unhashed. A fenced block is left as it is: a C program in a record is
       evidence, not a pointer.
    5. Every link target becomes empty, for the same reason. A link's text
       stays.
    6. Every run of whitespace becomes one space, so rewrapping a paragraph
       is not a change of reasoning.

    So prose written inside a code span, a link target or Confirmation is not
    frozen. That is the price of letting pointers and guards move, and it is
    in the README, where a reviewer reads what may still change.
    """
    kept = []
    paragraph = []
    front = 0
    in_confirmation = False
    in_fence = False

    def flush():
        if paragraph:
            kept.append(without_link_targets(without_code_spans(" ".join(paragraph))))
            paragraph.clear()

    for line in lines(text):
        if line == "---" and front < 2:
            front += 1
            flush()
            kept.append(line)
            continue
        if front == 1 and line.startswith("status:"):
            continue
        fence = line.startswith("```")
        if not in_fence and not fence and (line.startswith("## ") or line.startswith("### ")):
            in_confirmation = line.rstrip() == "### Confirmation"
        if in_confirmation:
            # Still followed through Confirmation, so a heading inside a
            # fence there does not end it.
            if fence:
                in_fence = not in_fence
            continue
        if fence or in_fence:
            flush()
            kept.append(line)
            if fence:
                in_fence = not in_fence
        elif line.strip() == "" or line.startswith("#"):
            flush()
            kept.append(line)
        else:
            paragraph.append(line)
    flush()

    return " ".join("\n".join(kept).split())


def without_code_spans(text):
    """The text with every inline code span emptied, however many backticks
    delimit it. A run of backticks with no closing run of the same length is
    text, as Markdown reads it."""
    out = []
    i = 0
    n = len(text)

    def run_at(j):
        k = j
        while k < n and text[k] == "`":
            k += 1
        return k - j

    while i < n:
        if text[i] != "`":
            out.append(text[i])
            i += 1
            continue
        opening = run_at(i)
        after = i + opening
        close = next(
            (j for j in range(after, n) if run_at(j) == opening and text[j - 1] != "`"),
            None,
        )
        if close is None:
            out.append(text[i:after])
            i = after
        else:
            out.append("`" * (opening * 2))
            i = close + opening
    return "".join(out)


def without_link_targets(text):
    """The text with every link target, `](...)`, emptied."""
    out = []
    rest = text
    while True:
        at = rest.find("](")
        if at < 0:
            break
        out.append(rest[: at + 2])
        after = rest[at + 2 :]
        end = after.find(")")
        if end < 0:
            rest = after
            break
        rest = after[end:]
    out.append(rest)
    return "".join(out)


def fnv1a64(data):
    """FNV-1a over 64 bits, written out so the hash is the same everywhere."""
    h = 0xCBF29CE484222325
    for byte in data:
        h ^= byte
        h = (h * 0x00000100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def hash_of(text):
    """The hash a record's lock line carries."""
    return format(fnv1a64(reasoning(text).encode("utf-8")), "016x")


def status(text):
    """The status in the record's front matter, without its quotes, single or
    double as YAML allows either. Only the front matter is read: a record's
    prose may well start a line with "status"."""
    it = iter(lines(text))
    if next(it, None) != "---":
        return None
    for line in it:
        if line == "---":
            return None
        if line.startswith("status:"):
            return line[len("status:") :].strip().strip("\"'")
    return None


def is_record(name):
    """Whether a file in `docs/adr` is a record: four digits, a dash, and
    `.md`. An editor's backup beside a record, `0003-x.md.orig`, is not one."""
    return len(name) > 8 and name[:4].isdigit() and name[:4].isascii() and name[4] == "-" and name.endswith(".md")


HEX = set("0123456789abcdef")


def check(records, lock):
    """Every problem with the records against the lock: a settled record with
    no line, a line whose hash differs, a line for a record that does not
    exist, a record with no status, and a line of the lock that is not exactly
    `NNNN <16 lowercase hex digits>` or repeats a number. `records` is each
    record's file name and text.

    The lock is read strictly because a line it misreads is reported as a
    change of reasoning, which sends a reader to a record that did not move.
    """
    problems = []
    locked = {}
    for index, line in enumerate(lines(lock)):
        if line.strip() == "" or line.startswith("#"):
            continue
        number, sep, digest = line.partition(" ")
        if not (
            sep
            and len(number) == 4
            and number.isascii()
            and number.isdigit()
            and len(digest) == 16
            and set(digest) <= HEX
        ):
            problems.append(
                f"line {index + 1} of docs/adr/frozen.txt is not "
                f"`NNNN <16 lowercase hex digits>`: {line!r}"
            )
            continue
        if number in locked:
            problems.append(f"docs/adr/frozen.txt has two lines for ADR-{number}")
        locked[number] = digest

    seen = set()
    for name, text in records:
        number = name[:4]
        seen.add(number)
        state = status(text)
        if state is None:
            problems.append(f"{name} has no status line")
            continue
        # A record with a line is held to it whatever its status now says:
        # setting a settled record back to `proposed` does not unfreeze it.
        digest = hash_of(text)
        want = locked.get(number)
        if want is None:
            if state != "proposed":
                problems.append(
                    f"ADR-{number} is {state} and has no line in docs/adr/frozen.txt. "
                    f"A record settled now adds its own line: `{number} {digest}`"
                )
        elif want != digest:
            problems.append(
                f"ADR-{number}'s reasoning has changed since it was settled. A decision "
                "reversed or extended is a new record that supersedes this one, and a "
                "decision applied to one more route is a row in its route table "
                "(docs/adr/README.md). Its status, its Confirmation, a code span and a "
                "link target may change and are not what moved"
            )
    for number in sorted(locked):
        if number not in seen:
            problems.append(
                f"docs/adr/frozen.txt has a line for ADR-{number}, and there is no such record"
            )
    return problems


def tree_records(adr):
    """Each record in `adr`, as its file name and text, in order."""
    names = sorted(n for n in os.listdir(adr) if is_record(n) and os.path.isfile(os.path.join(adr, n)))
    out = []
    for name in names:
        with open(os.path.join(adr, name), encoding="utf-8", newline="") as f:
            out.append((name, f.read()))
    return out


# A small record in the shape the template gives, for the tests below.
RECORD = """---
status: "accepted"
date: 2026-11-01
decision-makers: someone
---

# A title

## Considered Options

* keep `old_name` where it is

## Decision Outcome

Chosen because the walk in `memory::report` asks it once,
see [the record](./0017-a.md).

```c
free(p);
```

### Confirmation

`a_test_name` fails.

### Consequences

* Good, because it is one place.
"""


def line_for(number, text):
    """The lock line `check` wants for a record."""
    return f"{number} {hash_of(text)}\n"


class Check(unittest.TestCase):
    """What `check` decides, over records written here. Each test names the
    mutation that fails it."""

    def test_a_settled_record_whose_reasoning_moved_is_named(self):
        """Mutation: the hash comparison dropped."""
        edited = RECORD.replace("asks it once", "asks it twice")
        problems = check([("0001-a.md", edited)], line_for("0001", RECORD))
        self.assertEqual(len(problems), 1, problems)
        self.assertTrue(problems[0].startswith("ADR-0001's reasoning has changed"), problems)

    def test_a_settled_record_with_no_line_is_named_with_the_line_to_add(self):
        """Mutation: a record with no line taken to have the one it wants,
        `want = locked.get(number, digest)`."""
        problems = check([("0001-a.md", RECORD)], "")
        self.assertEqual(len(problems), 1, problems)
        self.assertTrue(problems[0].endswith(f"`{line_for('0001', RECORD).strip()}`"), problems)

    def test_a_line_for_a_record_that_does_not_exist_is_named(self):
        """Mutation: the walk over the lock's own lines dropped."""
        self.assertEqual(
            check([], line_for("0999", RECORD)),
            ["docs/adr/frozen.txt has a line for ADR-0999, and there is no such record"],
        )

    def test_a_proposed_record_needs_no_line(self):
        """Mutation: `proposed` checked like any other status."""
        proposed = RECORD.replace('status: "accepted"', 'status: "proposed"')
        self.assertEqual(check([("0001-a.md", proposed)], ""), [])

    def test_a_settled_record_set_back_to_proposed_is_still_held_to_its_line(self):
        """Mutation: a record skipped when its status is `proposed`, before
        its line is looked at."""
        edited = RECORD.replace('status: "accepted"', 'status: "proposed"').replace(
            "asks it once", "asks it twice"
        )
        problems = check([("0001-a.md", edited)], line_for("0001", RECORD))
        self.assertEqual(len(problems), 1, problems)
        self.assertTrue(problems[0].startswith("ADR-0001's reasoning has changed"), problems)

    def test_a_record_with_no_status_is_named(self):
        """Mutation: a record with no status skipped in silence."""
        broken = RECORD.replace('status: "accepted"\n', "")
        self.assertEqual(
            check([("0001-a.md", broken)], line_for("0001", RECORD)),
            ["0001-a.md has no status line"],
        )

    def test_a_lock_line_with_anything_after_its_hash_is_named_as_malformed(self):
        """Mutation: the length and the digits of the hash both left
        unchecked. Either check alone refuses this line."""
        lock = line_for("0001", RECORD).strip() + " # a note\n"
        problems = check([("0001-a.md", RECORD)], lock)
        # The malformed line, then the record it was meant for, which has none.
        self.assertEqual(len(problems), 2, problems)
        self.assertTrue(problems[0].startswith("line 1 of docs/adr/frozen.txt is not"), problems)
        self.assertIn("has no line", problems[1])

    def test_a_number_with_two_lock_lines_is_named(self):
        """Mutation: a repeated number let through."""
        lock = line_for("0001", RECORD) * 2
        self.assertEqual(
            check([("0001-a.md", RECORD)], lock),
            ["docs/adr/frozen.txt has two lines for ADR-0001"],
        )


class Reading(unittest.TestCase):
    """What `reasoning`, `status` and `is_record` read. Each test names the
    mutation that fails it."""

    def same(self, a, b):
        self.assertEqual(reasoning(a), reasoning(b))

    def differ(self, a, b):
        self.assertNotEqual(reasoning(a), reasoning(b))

    def test_an_edit_inside_confirmation_leaves_the_reasoning_as_it_was(self):
        """Mutation: step 3 left out."""
        self.same(RECORD.replace("`a_test_name` fails.", "`a_test_name` fails, and so does `another`."), RECORD)

    def test_a_status_change_leaves_the_reasoning_as_it_was(self):
        """Mutation: step 2 left out."""
        self.same(RECORD.replace('status: "accepted"', 'status: "superseded by ADR-0099"'), RECORD)

    def test_a_status_line_in_the_body_is_reasoning(self):
        """Mutation: step 2 drops a `status:` line anywhere, not only in the
        front matter."""
        with_ = RECORD.replace("# A title\n", "# A title\n\nstatus: what was believed\n")
        self.differ(with_.replace("what was believed", "what is believed now"), with_)

    def test_a_renamed_code_span_leaves_the_reasoning_as_it_was(self):
        """Mutation: step 4 left out."""
        self.same(RECORD.replace("`memory::report`", "`reads`"), RECORD)

    def test_a_renamed_code_span_that_wraps_a_line_leaves_the_reasoning_as_it_was(self):
        """Mutation: spans read a line at a time."""
        with_ = RECORD.replace("the walk in `memory::report`", "the walk in `memory::\nreport`")
        self.same(with_.replace("`memory::\nreport`", "`reads::\nwalk`"), with_)

    def test_prose_between_a_wrapped_span_and_the_next_changes_the_reasoning(self):
        """Mutation: spans read a line at a time, so the closing backtick of
        a wrapped span pairs with the next span's opening one."""
        with_ = RECORD.replace(
            "the walk in `memory::report` asks it once,",
            "the walk in `memory::\nreport` and then `known` asks it once,",
        )
        self.differ(with_.replace("and then", "but never"), with_)

    def test_a_span_closes_only_at_a_run_as_long_as_the_one_that_opened_it(self):
        """Mutation: an opening run matched by a longer closing run, or a span
        emptied to one pair of backticks whatever delimited it."""
        self.assertEqual(without_code_spans("`a``b`"), "``")
        self.assertEqual(without_code_spans("``a`b``"), "````")

    def test_a_changed_link_target_leaves_the_reasoning_as_it_was(self):
        """Mutation: step 5 left out."""
        self.same(RECORD.replace("./0017-a.md", "./0017-renamed.md"), RECORD)

    def test_every_link_target_on_a_line_is_emptied(self):
        """Mutation: only the first link target emptied."""
        with_ = RECORD.replace(
            "see [the record](./0017-a.md).", "see [the record](./0017-a.md) and [the other](./0018-b.md)."
        )
        self.same(with_.replace("./0018-b.md", "./0018-renamed.md"), with_)

    def test_a_rewrapped_paragraph_leaves_the_reasoning_as_it_was(self):
        """Mutation: step 6 left out. A paragraph's lines are joined by a
        space already, so what step 6 still takes away is the indentation of
        a list item's wrapped line."""
        self.same(RECORD.replace("asks it once,\nsee", "asks it\nonce, see"), RECORD)
        self.same(RECORD.replace("* Good, because it is", "* Good, because\n  it is"), RECORD)

    def test_a_record_checked_out_with_crlf_has_the_reasoning_it_has_with_lf(self):
        """Mutation: `lines` keeping the carriage return and step 6 left out,
        both at once, since step 6 alone takes a stray carriage return away
        as whitespace."""
        self.same(RECORD.replace("\n", "\r\n"), RECORD)

    def test_a_heading_inside_a_fenced_block_is_not_one(self):
        """Mutation: a heading inside a fence read as one, or fences not
        followed through Confirmation."""
        with_ = RECORD.replace(
            "`a_test_name` fails.",
            "`a_test_name` fails.\n\n```text\n### Consequences\n```\n\nStill Confirmation.",
        )
        self.same(with_.replace("Still Confirmation.", "Edited, and still Confirmation."), with_)

    def test_only_the_confirmation_heading_itself_starts_confirmation(self):
        """Mutation: any heading starting `### Confirmation` taken as it."""
        with_ = RECORD.replace(
            "### Consequences", "### Confirmation of the premise\n\nWhat was believed.\n\n### Consequences"
        )
        self.differ(with_.replace("What was believed.", "What is believed now."), with_)

    def test_a_changed_word_in_the_decision_outcome_changes_the_reasoning(self):
        """Mutation: `reasoning` answers the empty string."""
        self.differ(RECORD.replace("asks it once", "asks it twice"), RECORD)

    def test_a_changed_word_in_the_consequences_changes_the_reasoning(self):
        """Mutation: step 3 never ends."""
        self.differ(RECORD.replace("it is one place", "it is two places"), RECORD)

    def test_a_changed_word_in_the_considered_options_changes_the_reasoning(self):
        """Mutation: a paragraph holding a code span dropped whole."""
        self.differ(RECORD.replace("keep `old_name` where it is", "move `old_name` elsewhere"), RECORD)

    def test_a_changed_line_in_a_fenced_block_changes_the_reasoning(self):
        """Mutation: a fenced block's lines dropped."""
        self.differ(RECORD.replace("free(p);", "free(q);"), RECORD)

    def test_a_status_is_read_from_the_front_matter_alone(self):
        """Mutation: the status read from anywhere in the record."""
        broken = RECORD.replace('status: "accepted"', "").replace(
            "# A title\n", "# A title\n\nstatus: accepted, as prose\n"
        )
        self.assertIsNone(status(broken))

    def test_a_status_in_single_quotes_is_read_as_one_in_double_quotes(self):
        """Mutation: only double quotes taken off."""
        self.assertEqual(status(RECORD.replace('status: "accepted"', "status: 'proposed'")), "proposed")

    def test_only_a_numbered_markdown_file_is_a_record(self):
        """Mutation: the `.md` test dropped from `is_record`."""
        self.assertTrue(is_record("0060-an-access-is-proved.md"))
        self.assertFalse(is_record("0060-an-access-is-proved.md.orig"))
        self.assertFalse(is_record("0060-an-access-is-proved.md~"))
        self.assertFalse(is_record("adr-template.md"))
        self.assertFalse(is_record("frozen.txt"))


def main():
    suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
    if not unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful():
        return 1

    root = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
    adr = os.path.join(root, "docs", "adr")
    with open(os.path.join(adr, "frozen.txt"), encoding="utf-8", newline="") as f:
        lock = f.read()
    records = tree_records(adr)
    problems = check(records, lock)
    for problem in problems:
        print(problem)
    print(f"records: {len(records)}  problems: {len(problems)}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
