//! Every package in the workspace takes `[workspace.lints]`.
//!
//! The lints reach a package only if its own manifest opts in with `[lints]`
//! and `workspace = true`, and two deleted lines in one manifest turn all of
//! them off for that package, `unsafe_code = "forbid"` among them. Nothing the
//! compiler or clippy says notices, because the lint that would have fired is
//! the one that went. So the opt-in is read here, from the manifests.
//!
//! Read line by line rather than with a TOML parser, and strictly: the one
//! spelling every manifest uses is accepted, and any other is reported as no
//! opt-in at all. That can make this test fail on a manifest that is in fact
//! fine, and it cannot make it pass on one that is not.

use std::fs;
use std::path::{Path, PathBuf};

/// The manifests of every package: the root's own, and each member's.
fn manifests() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(root.join("Cargo.toml")).expect("the root manifest reads");

    // The root is a package as well as the workspace (ADR-0058). If it stops
    // being one, the layout this test was written against has changed.
    assert!(
        text.lines().any(|line| line.trim() == "[package]"),
        "the root manifest has no [package] table, so the root is not a package to check"
    );

    let mut found = vec![root.join("Cargo.toml")];
    for member in members(&text) {
        found.push(root.join(member).join("Cargo.toml"));
    }
    found
}

/// The paths in `[workspace] members`, from the root manifest's text.
///
/// Panics, naming the reason, on a glob or on a `members` it cannot read:
/// cargo would expand a glob and this cannot, and a test that checks no member
/// proves nothing about them.
fn members(root: &str) -> Vec<String> {
    let start = root
        .find("\nmembers")
        .expect("the root manifest lists its members");
    let open = start + root[start..].find('[').expect("members is an array");
    let close = open + root[open..].find(']').expect("the members array closes");

    let found: Vec<String> = root[open + 1..close]
        .split(',')
        .map(|entry| entry.trim().trim_matches('"').to_owned())
        .filter(|entry| !entry.is_empty())
        .collect();

    for member in &found {
        assert!(
            !member.contains('*'),
            "a glob in `members` is not read here; list the packages: {member}"
        );
    }
    assert!(!found.is_empty(), "`members` names no package");
    found
}

/// Whether a manifest's text has a `[lints]` table holding `workspace = true`.
///
/// A comment needs no rule of its own: `# [lints]` does not start with `[`, and
/// `# workspace = true` has `# workspace` as its key.
fn opts_in(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_lints = line == "[lints]";
            continue;
        }
        if !in_lints {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "workspace" && value.trim() == "true" {
                return true;
            }
        }
    }
    false
}

/// Every package in the workspace opts in to `[workspace.lints]`, so that
/// `unsafe_code = "forbid"` and the documentation lints reach all of them.
///
/// Mutation: delete `[lints]` and `workspace = true` from any one of
/// `Cargo.toml`, `crates/safec-c/Cargo.toml`, `crates/safec-ir/Cargo.toml` or
/// `crates/safec-llvm/Cargo.toml`. This fails, naming that manifest. So does
/// adding a member whose manifest has no `[lints]`, and changing `members` to
/// `["crates/*"]` panics in `members` rather than passing. Commenting out the
/// `workspace = true` under `[lints]` fails here too. Commenting out `[lints]`
/// alone is refused by cargo before this runs, since `workspace = true` then
/// lands in the table above it.
#[test]
fn every_package_in_the_workspace_takes_the_workspace_lints() {
    let missing: Vec<String> = manifests()
        .into_iter()
        .filter(|path| {
            let text = fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("{} reads: {error}", path.display()));
            !opts_in(&text)
        })
        .map(|path| path.display().to_string())
        .collect();

    assert!(
        missing.is_empty(),
        "these manifests do not opt in to the workspace lints with `[lints]` and \
         `workspace = true`: {missing:?}"
    );
}
