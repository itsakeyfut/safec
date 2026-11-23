//! Every struct shape is laid out by the IR as `clang` lays it out, on every
//! target this compiler knows (ADR-0124: the IR lays an aggregate out, and
//! nothing else does, so this is the one answer to compare).
//!
//! **Compared with `clang` as it runs, and not with numbers written here**: a
//! table of offsets typed in by hand is the shape a test of the target table
//! already has (`Target::ALL`), and a layout rule is exactly what a person
//! gets wrong in the same way twice. Each shape is compiled by `clang` with
//! `__builtin_offsetof`, `sizeof` and `_Alignof` into globals, read back out
//! of the IR it writes, and set beside `TranslationUnit::push_struct`'s answer.
//! Skipped with a note where there is no `clang`, and a failure where
//! `SAFEC_REQUIRE_LLVM` is set, as in `tests/run.rs`.

use std::process::{Command, Stdio};

use safec_ir::ir::{Rank, Sign, TranslationUnit, Ty, TyId};
use safec_ir::target::Target;

/// One member's type, as both the C below and the IR spell it.
#[derive(Clone, Copy)]
enum Member {
    Integer(Rank, Sign),
    Pointer,
    Array(Rank, u64),
}

impl Member {
    fn c(self) -> String {
        let integer = |rank: Rank| match rank {
            Rank::Char => "char",
            Rank::Short => "short",
            Rank::Int => "int",
            Rank::Long => "long",
            Rank::LongLong => "long long",
        };
        match self {
            Member::Integer(rank, Sign::Unsigned) => format!("unsigned {}", integer(rank)),
            Member::Integer(rank, _) => integer(rank).to_owned(),
            Member::Pointer => "int *".to_owned(),
            Member::Array(rank, _) => integer(rank).to_owned(),
        }
    }

    fn suffix(self) -> String {
        match self {
            Member::Array(_, length) => format!("[{length}]"),
            Member::Integer(..) | Member::Pointer => String::new(),
        }
    }

    fn ir(self, unit: &mut TranslationUnit) -> TyId {
        match self {
            Member::Integer(rank, sign) => unit.push_type(Ty::Integer { rank, sign }),
            Member::Pointer => {
                let int = unit.push_type(Ty::Integer {
                    rank: Rank::Int,
                    sign: Sign::Signed,
                });
                unit.push_type(Ty::Pointer(int))
            }
            Member::Array(rank, length) => {
                let element = unit.push_type(Ty::Integer {
                    rank,
                    sign: Sign::Signed,
                });
                unit.push_type(Ty::Array { element, length })
            }
        }
    }
}

/// The shapes, chosen for where padding falls: after a `char` before each
/// wider member, at the end to the strictest alignment, around an array, and
/// a `long long`, which `i686` aligns to four in a struct.
fn shapes() -> Vec<Vec<Member>> {
    use Member::{Array, Integer, Pointer};
    use Rank::{Char, Int, Long, LongLong, Short};
    use Sign::Signed;
    vec![
        vec![Integer(Char, Signed), Integer(Int, Signed)],
        vec![Integer(Int, Signed), Integer(Char, Signed)],
        vec![
            Integer(Char, Signed),
            Integer(LongLong, Signed),
            Integer(Short, Signed),
            Pointer,
        ],
        vec![Integer(Char, Signed), Integer(Char, Sign::Unsigned)],
        vec![Integer(Short, Signed), Integer(Char, Signed)],
        vec![Array(Int, 3), Integer(Char, Signed)],
        vec![
            Integer(Char, Signed),
            Array(Short, 3),
            Integer(Long, Signed),
        ],
        vec![Integer(LongLong, Signed), Integer(Char, Signed)],
        vec![Integer(Char, Signed), Pointer, Integer(Char, Signed)],
    ]
}

/// What `clang` says each shape's offsets, size and alignment are on
/// `triple`, in the order [`shapes`] gives, or `None` where it did not run.
fn clang_layouts(triple: &str, shapes: &[Vec<Member>]) -> Option<Vec<(Vec<u64>, u64, u64)>> {
    let mut source = String::new();
    for (n, members) in shapes.iter().enumerate() {
        source.push_str(&format!("struct S{n} {{"));
        for (k, member) in members.iter().enumerate() {
            source.push_str(&format!(" {} m{k}{};", member.c(), member.suffix()));
        }
        source.push_str(" };\n");
        for k in 0..members.len() {
            source.push_str(&format!(
                "long long o_{n}_{k} = __builtin_offsetof(struct S{n}, m{k});\n"
            ));
        }
        source.push_str(&format!("long long s_{n} = sizeof(struct S{n});\n"));
        source.push_str(&format!("long long a_{n} = _Alignof(struct S{n});\n"));
    }
    let dir = std::env::temp_dir().join(format!("safec_layout_{triple}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the temporary directory is writable");
    let file = dir.join("shapes.c");
    std::fs::write(&file, source).expect("the temporary directory is writable");
    let output = Command::new("clang")
        .args(["-std=c17", "-S", "-emit-llvm", "-o", "-"])
        .arg(format!("--target={triple}"))
        .arg(&file)
        .stderr(Stdio::inherit())
        .output()
        .ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "clang compiled the shapes for {triple}"
    );
    let ir = String::from_utf8_lossy(&output.stdout);
    let value = |name: &str| -> u64 {
        let line = ir
            .lines()
            .find(|line| line.starts_with(&format!("@{name} = ")))
            .unwrap_or_else(|| panic!("clang wrote `@{name}` for {triple}"));
        line.split("i64 ")
            .nth(1)
            .and_then(|rest| rest.split(',').next())
            .and_then(|number| number.trim().parse().ok())
            .unwrap_or_else(|| panic!("`@{name}` holds a number for {triple}: {line}"))
    };
    Some(
        shapes
            .iter()
            .enumerate()
            .map(|(n, members)| {
                (
                    (0..members.len())
                        .map(|k| value(&format!("o_{n}_{k}")))
                        .collect(),
                    value(&format!("s_{n}")),
                    value(&format!("a_{n}")),
                )
            })
            .collect(),
    )
}

/// Mutation: place each member at the end of the one before it in
/// `safec_ir::ir::lay_out`; this fails at the first shape with padding.
#[test]
fn the_layout_of_every_struct_shape_is_clangs() {
    let clang_is_here = Command::new("clang")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    if !clang_is_here {
        assert!(
            std::env::var_os("SAFEC_REQUIRE_LLVM").is_none(),
            "SAFEC_REQUIRE_LLVM is set and there is no `clang` to compare a layout with"
        );
        eprintln!("no `clang` on this machine: no layout was compared");
        return;
    }
    let shapes = shapes();
    for &target in Target::ALL {
        let triple = target.triple();
        let Some(expected) = clang_layouts(triple, &shapes) else {
            panic!("clang ran for {triple}");
        };
        for (members, (offsets, size, align)) in shapes.iter().zip(expected) {
            let mut unit = TranslationUnit::new(target);
            let types: Vec<TyId> = members.iter().map(|member| member.ir(&mut unit)).collect();
            let id = unit
                .push_struct(&types)
                .expect("every shape here is laid out");
            let layout = unit.struct_layout(id);
            let laid: Vec<u64> = layout.members.iter().map(|member| member.offset).collect();
            let spelled: Vec<String> = members.iter().map(|member| member.c()).collect();
            assert_eq!(laid, offsets, "{triple}: offsets of {spelled:?}");
            assert_eq!(layout.size, size, "{triple}: size of {spelled:?}");
            assert_eq!(layout.align, align, "{triple}: alignment of {spelled:?}");
        }
    }
}
