//! A struct passed and returned by value meets `clang`'s C ABI on the host,
//! both ways round: a caller this compiler built calls a callee `clang`
//! built, and the other way, for each size class the ABIs tell apart (#1201,
//! ADR-0124). On Linux and macOS that is x86-64 System V, and on Windows
//! Win64, so each CI runner holds the convention of its own target.
//!
//! **Linked and run, not compared as text**: the convention is what the two
//! objects agree on at run time, and a signature spelled alike can still put
//! a piece in another register. Skipped with a note where there is no
//! `clang`, and a failure where `SAFEC_REQUIRE_LLVM` is set, as in
//! `tests/run.rs`.

use std::fs;
use std::process::{Command, Stdio};

/// One struct per size class: its members, and the bytes it takes.
const SHAPES: &[(&str, u64)] = &[
    ("char a;", 1),
    ("char a; char b; char c;", 3),
    ("int a;", 4),
    ("int a; int b;", 8),
    ("int a; int b; int c;", 12),
    ("long long a; long long b;", 16),
    ("long long a; long long b; long long c;", 24),
];

/// The member names of a shape, in order.
fn members(shape: &str) -> Vec<&str> {
    shape
        .split(';')
        .filter_map(|member| member.split_whitespace().last())
        .collect()
}

/// The callee: for each shape, `make_K(seed)` fills a struct from `seed`
/// and returns it, and `sum_K(s)` adds its members.
fn callee() -> String {
    let mut source = String::new();
    for (k, (shape, _)) in SHAPES.iter().enumerate() {
        source.push_str(&format!("struct S{k} {{ {shape} }};\n"));
        source.push_str(&format!(
            "struct S{k} make_{k}(int seed) {{\n    struct S{k} s;\n"
        ));
        for (i, member) in members(shape).iter().enumerate() {
            source.push_str(&format!("    s.{member} = seed + {i};\n"));
        }
        source.push_str("    return s;\n}\n");
        source.push_str(&format!("int sum_{k}(struct S{k} s) {{\n    return 0"));
        for member in members(shape) {
            source.push_str(&format!(" + (int)s.{member}"));
        }
        source.push_str(";\n}\n");
    }
    source
}

/// The caller: each shape made with seed 10, summed, and checked, through a
/// struct the callee returned and one passed back to it. Exits with the
/// shape that went wrong, counted from one, and zero where none did.
fn caller() -> String {
    let mut source = String::new();
    for (k, (shape, _)) in SHAPES.iter().enumerate() {
        source.push_str(&format!("struct S{k} {{ {shape} }};\n"));
        source.push_str(&format!("struct S{k} make_{k}(int seed);\n"));
        source.push_str(&format!("int sum_{k}(struct S{k} s);\n"));
    }
    source.push_str("int main(void) {\n");
    for (k, (shape, _)) in SHAPES.iter().enumerate() {
        let n = members(shape).len();
        let expected: usize = (0..n).map(|i| 10 + i).sum();
        let first = members(shape)[0];
        source.push_str(&format!(
            "    {{\n        struct S{k} s = make_{k}(10);\n        if (s.{first} != 10 || sum_{k}(s) != {expected}) {{\n            return {};\n        }}\n    }}\n",
            k + 1
        ));
    }
    source.push_str("    return 0;\n}\n");
    source
}

/// Build `source` into an object at `out` with this compiler or with
/// `clang`, and say what failed where it did.
fn object(by_safec: bool, source: &std::path::Path, out: &std::path::Path) {
    let built = if by_safec {
        Command::new(env!("CARGO_BIN_EXE_safec"))
            .args([
                "--color", "never", "--safety", "off", "--emit", "object", "-o",
            ])
            .arg(out)
            .arg(source)
            .output()
    } else {
        Command::new("clang")
            .args(["-std=c17", "-w", "-c", "-o"])
            .arg(out)
            .arg(source)
            .output()
    }
    .expect("the compiler runs");
    assert!(
        built.status.success(),
        "{} did not build {}:\n{}",
        if by_safec { "safec" } else { "clang" },
        source.display(),
        String::from_utf8_lossy(&built.stderr)
    );
}

/// Mutation: classify a Win64 struct of 1, 2, 4 or 8 bytes as memory in
/// `abi::classify`; this fails on Windows, where `clang` passes it in a
/// register. The System V half is held by the Linux and macOS runners.
#[test]
fn a_struct_by_value_meets_clangs_abi() {
    let clang_is_here = Command::new("clang")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    if !clang_is_here {
        assert!(
            std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
            "SAFEC_REQUIRE_LLVM is set and there is no `clang` to link against"
        );
        eprintln!("no `clang` on this machine: no ABI was compared");
        return;
    }
    let dir = std::env::temp_dir().join("safec_abi");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the temporary directory is writable");
    let (callee_c, caller_c) = (dir.join("callee.c"), dir.join("caller.c"));
    fs::write(&callee_c, callee()).expect("the temporary directory is writable");
    fs::write(&caller_c, caller()).expect("the temporary directory is writable");
    let exe = dir.join(if cfg!(windows) { "pair.exe" } else { "pair" });
    for (caller_by_safec, callee_by_safec) in [(true, false), (false, true)] {
        let (caller_o, callee_o) = (dir.join("caller.o"), dir.join("callee.o"));
        object(caller_by_safec, &caller_c, &caller_o);
        object(callee_by_safec, &callee_c, &callee_o);
        let linked = Command::new("clang")
            .arg("-o")
            .arg(&exe)
            .arg(&caller_o)
            .arg(&callee_o)
            .output()
            .expect("clang links");
        assert!(
            linked.status.success(),
            "the two objects did not link:\n{}",
            String::from_utf8_lossy(&linked.stderr)
        );
        let ran = Command::new(&exe)
            .status()
            .expect("the linked program runs");
        let built_by = if caller_by_safec {
            "safec calling clang"
        } else {
            "clang calling safec"
        };
        assert_eq!(
            ran.code(),
            Some(0),
            "{built_by}: the shape that went wrong, counted from one"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}
