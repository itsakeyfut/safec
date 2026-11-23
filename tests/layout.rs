//! Every struct and union shape is laid out by the IR as `clang` lays it out, on every
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

use safec_ir::ir::{Rank, Sign, StructId, TranslationUnit, Ty, TyId};
use safec_ir::target::Target;

/// One member's type, as both the C below and the IR spell it.
#[derive(Clone, Copy)]
enum Member {
    Integer(Rank, Sign),
    Pointer,
    Array(Rank, u64),
    /// The shape at this index in [`shapes`], which comes before it, by
    /// value, or an array of this many of it (#1202).
    Shape(usize, Option<u64>),
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
            Member::Shape(index, _) => format!("{} S{index}", keyword(index)),
        }
    }

    fn suffix(self) -> String {
        match self {
            Member::Array(_, length) | Member::Shape(_, Some(length)) => format!("[{length}]"),
            Member::Integer(..) | Member::Pointer | Member::Shape(_, None) => String::new(),
        }
    }

    fn ir(self, unit: &mut TranslationUnit, shapes: &[Vec<Member>]) -> TyId {
        match self {
            Member::Shape(index, length) => {
                let members: Vec<TyId> = shapes[index]
                    .iter()
                    .map(|member| member.ir(unit, shapes))
                    .collect();
                let (_, element) = lay(unit, index, &members);
                match length {
                    Some(length) => unit.push_type(Ty::Array { element, length }),
                    None => element,
                }
            }
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

/// Where the unions begin in [`shapes`]: after every struct, counted from
/// the list itself, so a struct shape added later moves it with them.
fn first_union() -> usize {
    struct_shapes().len()
}

/// `union` or `struct`, for the shape at `index`.
fn keyword(index: usize) -> &'static str {
    if index >= first_union() {
        "union"
    } else {
        "struct"
    }
}

/// Lay out the shape at `index` of these members as the IR does, and say
/// which it is and its type.
fn lay(unit: &mut TranslationUnit, index: usize, members: &[TyId]) -> (StructId, TyId) {
    if index >= first_union() {
        let id = unit.push_union(members).expect("every shape is laid out");
        (id, unit.push_type(Ty::Struct(id)))
    } else {
        let id = unit.push_struct(members).expect("every shape is laid out");
        (id, unit.push_type(Ty::Struct(id)))
    }
}

/// The shapes, chosen for where padding falls: after a `char` before each
/// wider member, at the end to the strictest alignment, around an array, and
/// a `long long`, which `i686` aligns to four in a struct; and a struct as a
/// member and an array of them, aligned as their strictest member (#1202);
/// and unions after them, sized as their largest member rounded to the
/// strictest alignment, which `i686`'s `long long` and a `char` array longer
/// than the widest member each move (#1203).
fn shapes() -> Vec<Vec<Member>> {
    let mut shapes = struct_shapes();
    shapes.extend(union_shapes(shapes.len()));
    shapes
}

/// The struct shapes, which [`shapes`] gives first.
fn struct_shapes() -> Vec<Vec<Member>> {
    use Member::{Array, Integer, Pointer, Shape};
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
        vec![
            Integer(Char, Signed),
            Shape(2, None),
            Integer(Short, Signed),
        ],
        vec![
            Integer(Char, Signed),
            Shape(0, Some(3)),
            Integer(Char, Signed),
        ],
        vec![Shape(7, None), Integer(Char, Signed), Shape(9, Some(2))],
    ]
}

/// The union shapes, which [`shapes`] gives after the structs, from index
/// `first`: one of them is named by its index there.
fn union_shapes(first: usize) -> Vec<Vec<Member>> {
    use Member::{Array, Integer, Pointer, Shape};
    use Rank::{Char, Int, LongLong, Short};
    use Sign::Signed;
    vec![
        vec![Integer(Char, Signed), Integer(Int, Signed)],
        vec![Integer(LongLong, Signed), Array(Char, 13), Pointer],
        vec![
            Integer(Short, Signed),
            Shape(2, None),
            Integer(Char, Signed),
        ],
        vec![Integer(Char, Signed), Shape(first, Some(3))],
        vec![
            Integer(Char, Signed),
            Shape(first + 2, None),
            Integer(Int, Signed),
        ],
    ]
}

/// What `clang` says each shape's offsets, size and alignment are on
/// `triple`, in the order [`shapes`] gives, or `None` where it did not run.
fn clang_layouts(triple: &str, shapes: &[Vec<Member>]) -> Option<Vec<(Vec<u64>, u64, u64)>> {
    let mut source = String::new();
    for (n, members) in shapes.iter().enumerate() {
        source.push_str(&format!("{} S{n} {{", keyword(n)));
        for (k, member) in members.iter().enumerate() {
            source.push_str(&format!(" {} m{k}{};", member.c(), member.suffix()));
        }
        source.push_str(" };\n");
        for k in 0..members.len() {
            source.push_str(&format!(
                "long long o_{n}_{k} = __builtin_offsetof({} S{n}, m{k});\n",
                keyword(n)
            ));
        }
        source.push_str(&format!("long long s_{n} = sizeof({} S{n});\n", keyword(n)));
        source.push_str(&format!(
            "long long a_{n} = _Alignof({} S{n});\n",
            keyword(n)
        ));
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
/// `safec_ir::ir::lay_out`; this fails at the first shape with padding. Size
/// a union as its first member in `lay_out_union`; this fails at the second
/// union.
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
        for (index, (members, (offsets, size, align))) in shapes.iter().zip(expected).enumerate() {
            let mut unit = TranslationUnit::new(target);
            let types: Vec<TyId> = members
                .iter()
                .map(|member| member.ir(&mut unit, &shapes))
                .collect();
            let (id, _) = lay(&mut unit, index, &types);
            let layout = unit.struct_layout(id);
            let laid: Vec<u64> = layout.members.iter().map(|member| member.offset).collect();
            let spelled: Vec<String> = members.iter().map(|member| member.c()).collect();
            assert_eq!(laid, offsets, "{triple}: offsets of {spelled:?}");
            assert_eq!(layout.size, size, "{triple}: size of {spelled:?}");
            assert_eq!(layout.align, align, "{triple}: alignment of {spelled:?}");
        }
    }
}
