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

use safec_ir::ir::{Declared, Length, Rank, Sign, StructId, TranslationUnit, Ty, TyId};
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
    /// An anonymous member (C17 6.7.2.1 p13, #1207): a union where the flag
    /// says so and a struct where not, of these members, written inline with
    /// no tag and no name. The IR lays it out as any member of its type, and
    /// `clang` places it where `__builtin_offsetof` finds its first member.
    Anonymous(bool, &'static [Member]),
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
            // Its spelling in a failure's message; `declared` writes the C.
            Member::Anonymous(true, _) => "union { .. }".to_owned(),
            Member::Anonymous(false, _) => "struct { .. }".to_owned(),
        }
    }

    /// The member declaration of this member at position `k`, `;` included:
    /// named `m{k}`, or for an anonymous member its members named `a{k}_{j}`
    /// inside it.
    fn declared(self, k: usize) -> String {
        match self {
            Member::Anonymous(union, members) => {
                let keyword = if union { "union" } else { "struct" };
                let inner: String = members
                    .iter()
                    .enumerate()
                    .map(|(j, member)| format!(" {} a{k}_{j}{};", member.c(), member.suffix()))
                    .collect();
                format!(" {keyword} {{{inner} }};")
            }
            _ => format!(" {} m{k}{};", self.c(), self.suffix()),
        }
    }

    /// What `__builtin_offsetof` names to find this member at position `k`:
    /// its own name, or an anonymous member's first member, which is where
    /// the anonymous member starts.
    fn offset_name(self, k: usize) -> String {
        match self {
            Member::Anonymous(..) => format!("a{k}_0"),
            _ => format!("m{k}"),
        }
    }

    fn suffix(self) -> String {
        match self {
            Member::Array(_, length) | Member::Shape(_, Some(length)) => format!("[{length}]"),
            Member::Integer(..)
            | Member::Pointer
            | Member::Shape(_, None)
            | Member::Anonymous(..) => String::new(),
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
                    Some(length) => unit.push_type(Ty::Array {
                        element,
                        length: Length::Constant(length),
                    }),
                    None => element,
                }
            }
            Member::Anonymous(union, members) => {
                let members: Vec<TyId> = members
                    .iter()
                    .map(|member| member.ir(unit, shapes))
                    .collect();
                let id = if union {
                    unit.push_union(&members)
                } else {
                    unit.push_struct(&members)
                }
                .expect("every shape is laid out");
                unit.push_type(Ty::Struct(id))
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
                unit.push_type(Ty::Array {
                    element,
                    length: Length::Constant(length),
                })
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
    use Member::{Anonymous, Array, Integer, Pointer, Shape};
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
        vec![
            Integer(Char, Signed),
            Anonymous(true, &[Integer(Short, Signed), Integer(LongLong, Signed)]),
            Integer(Char, Signed),
        ],
    ]
}

/// The union shapes, which [`shapes`] gives after the structs, from index
/// `first`: one of them is named by its index there.
fn union_shapes(first: usize) -> Vec<Vec<Member>> {
    use Member::{Anonymous, Array, Integer, Pointer, Shape};
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
        vec![
            Integer(Short, Signed),
            Anonymous(
                false,
                &[
                    Integer(Char, Signed),
                    Integer(Int, Signed),
                    Integer(Char, Signed),
                ],
            ),
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
            source.push_str(&member.declared(k));
        }
        source.push_str(" };\n");
        for (k, member) in members.iter().enumerate() {
            source.push_str(&format!(
                "long long o_{n}_{k} = __builtin_offsetof({} S{n}, {});\n",
                keyword(n),
                member.offset_name(k)
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
/// `safec_ir::record::lay_out_itanium`; this fails at the first shape with padding. Size
/// a union as its first member in `record::lay_out_union`; this fails at the second
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

/// One member of a bit-field shape: its integer type's rank and sign, its
/// width where it is a bit-field, and whether it is named.
type BitMember = (Rank, Sign, Option<u64>, bool);

/// The bit-field shapes, each a struct or a union, chosen for where the
/// three packing conventions part (#1206): a bit-field that would straddle
/// its type's unit, one of another type's size after it, a zero-width one
/// with and without a unit open, an unnamed one, an ordinary member between
/// bit-fields, `i686`'s `long long` aligned to 4 and taking 8, and unions of
/// bit-fields, named and not.
fn bit_shapes() -> Vec<(bool, Vec<BitMember>)> {
    use Rank::{Char, Int, Long, LongLong, Short};
    use Sign::{Signed, Unsigned};
    let s = |members: Vec<BitMember>| (false, members);
    let u = |members: Vec<BitMember>| (true, members);
    vec![
        s(vec![
            (Int, Signed, Some(3), true),
            (Int, Unsigned, Some(5), true),
        ]),
        s(vec![
            (Char, Signed, None, true),
            (Int, Signed, Some(3), true),
            (Char, Signed, None, true),
        ]),
        s(vec![
            (Int, Signed, Some(30), true),
            (Int, Signed, Some(5), true),
        ]),
        s(vec![
            (Char, Signed, Some(3), true),
            (Int, Signed, Some(7), true),
            (Char, Signed, Some(2), true),
        ]),
        s(vec![
            (Int, Signed, Some(3), true),
            (Int, Signed, Some(0), false),
            (Int, Signed, Some(4), true),
        ]),
        s(vec![
            (Char, Signed, Some(3), true),
            (Int, Signed, Some(0), false),
            (Char, Signed, Some(4), true),
        ]),
        s(vec![
            (Char, Signed, Some(1), true),
            (Int, Signed, Some(3), false),
            (Char, Signed, None, true),
        ]),
        s(vec![
            (Short, Signed, Some(9), true),
            (Short, Signed, Some(9), true),
        ]),
        s(vec![
            (LongLong, Signed, Some(40), true),
            (Int, Signed, Some(20), true),
        ]),
        s(vec![
            (Char, Signed, None, true),
            (LongLong, Signed, Some(3), true),
        ]),
        s(vec![
            (Int, Signed, Some(0), false),
            (Char, Signed, Some(1), true),
        ]),
        s(vec![
            (Char, Signed, Some(7), true),
            (Short, Signed, Some(7), true),
            (Int, Signed, Some(7), true),
        ]),
        s(vec![
            (LongLong, Signed, Some(3), true),
            (Char, Signed, None, true),
        ]),
        s(vec![
            (Char, Signed, Some(2), true),
            (Char, Signed, Some(0), false),
            (Char, Signed, Some(3), true),
            (Int, Signed, Some(0), false),
            (Short, Signed, Some(1), true),
        ]),
        s(vec![
            (Char, Signed, None, true),
            (Short, Signed, Some(4), false),
            (Char, Signed, None, true),
        ]),
        s(vec![
            (Int, Signed, Some(31), true),
            (Char, Signed, Some(2), true),
        ]),
        s(vec![
            (LongLong, Signed, Some(33), true),
            (LongLong, Signed, Some(33), true),
            (Char, Signed, None, true),
        ]),
        s(vec![
            (Int, Unsigned, Some(1), true),
            (LongLong, Signed, Some(1), true),
            (Short, Signed, Some(1), true),
        ]),
        s(vec![
            (Int, Signed, Some(5), true),
            (Char, Signed, None, true),
            (Int, Signed, Some(5), true),
        ]),
        s(vec![
            (Long, Signed, Some(3), true),
            (Int, Signed, Some(5), true),
        ]),
        s(vec![
            (Char, Unsigned, Some(8), true),
            (Char, Unsigned, Some(8), true),
            (Char, Unsigned, Some(1), true),
        ]),
        s(vec![
            (Char, Signed, None, true),
            (LongLong, Signed, Some(60), true),
            (LongLong, Signed, Some(10), true),
        ]),
        s(vec![
            (Short, Signed, None, true),
            (LongLong, Signed, Some(48), true),
            (Short, Signed, Some(15), true),
        ]),
        u(vec![
            (Int, Signed, Some(3), true),
            (Char, Signed, None, true),
        ]),
        u(vec![(Char, Signed, Some(3), true)]),
        u(vec![
            (LongLong, Signed, Some(3), true),
            (Char, Signed, None, true),
        ]),
        u(vec![
            (Char, Signed, None, true),
            (Int, Signed, Some(3), false),
        ]),
        u(vec![
            (Char, Signed, None, true),
            (Int, Signed, Some(0), false),
        ]),
        u(vec![
            (Short, Signed, Some(12), true),
            (Char, Signed, Some(2), true),
        ]),
        // Found by review: a struct ending in a zero-width bit-field, which
        // no unit fits; unions whose zero-width bit-field takes its type's
        // size on Microsoft after another bit-field and not before; and an
        // unnamed bit-field wider than its union.
        s(vec![
            (Int, Signed, None, true),
            (Int, Signed, Some(0), false),
        ]),
        s(vec![
            (Char, Signed, None, true),
            (Int, Signed, Some(0), false),
        ]),
        s(vec![
            (Int, Signed, Some(3), true),
            (Int, Signed, Some(0), false),
        ]),
        u(vec![
            (Char, Signed, Some(3), true),
            (LongLong, Signed, Some(0), false),
        ]),
        u(vec![
            (Char, Signed, None, true),
            (LongLong, Signed, Some(0), false),
        ]),
        u(vec![
            (LongLong, Signed, Some(0), false),
            (Char, Signed, Some(3), true),
        ]),
        u(vec![
            (Char, Signed, Some(3), true),
            (Int, Signed, Some(0), false),
            (Char, Signed, None, true),
        ]),
        u(vec![
            (Char, Unsigned, Some(5), true),
            (LongLong, Unsigned, Some(11), false),
        ]),
        s(vec![
            (Char, Signed, Some(1), true),
            (Char, Signed, Some(0), false),
        ]),
        s(vec![
            (Char, Signed, None, true),
            (Int, Signed, Some(17), false),
        ]),
    ]
}

/// The C spelling of an integer type.
fn integer(rank: Rank, sign: Sign) -> String {
    let name = match rank {
        Rank::Char => "char",
        Rank::Short => "short",
        Rank::Int => "int",
        Rank::Long => "long",
        Rank::LongLong => "long long",
    };
    match sign {
        Sign::Unsigned => format!("unsigned {name}"),
        _ => name.to_owned(),
    }
}

/// One shape as `clang` lays it out: each named member's first bit and
/// its width where it is a bit-field, and the whole's size and alignment.
type BitLayout = (Vec<(u64, Option<u64>)>, u64, u64);

/// What `clang`'s record layout dump says each bit-field shape is on
/// `triple`, in the order given.
fn clang_bit_layouts(triple: &str, shapes: &[(bool, Vec<BitMember>)]) -> Vec<BitLayout> {
    let mut source = String::new();
    for (n, (union, members)) in shapes.iter().enumerate() {
        let keyword = if *union { "union" } else { "struct" };
        source.push_str(&format!("{keyword} B{n} {{"));
        for (k, &(rank, sign, width, named)) in members.iter().enumerate() {
            let name = if named {
                format!(" m{k}")
            } else {
                String::new()
            };
            let width = width.map_or(String::new(), |width| format!(" : {width}"));
            source.push_str(&format!(" {}{name}{width};", integer(rank, sign)));
        }
        source.push_str(&format!(" }};\nint z{n} = sizeof({keyword} B{n});\n"));
    }
    let dir = std::env::temp_dir().join(format!("safec_bits_{triple}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the temporary directory is writable");
    let file = dir.join("bits.c");
    std::fs::write(&file, source).expect("the temporary directory is writable");
    let output = Command::new("clang")
        .args([
            "-std=c17",
            "-w",
            "-fsyntax-only",
            "-Xclang",
            "-fdump-record-layouts-complete",
        ])
        .arg(format!("--target={triple}"))
        .arg(&file)
        .output()
        .expect("clang runs");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "clang read the bit-field shapes for {triple}"
    );
    let dump = String::from_utf8_lossy(&output.stdout);
    let mut found: Vec<Option<BitLayout>> = vec![None; shapes.len()];
    for block in dump.split("*** Dumping AST Record Layout").skip(1) {
        let mut lines = block.trim().lines();
        let header = lines.next().unwrap_or_default();
        let named_shape = header.contains("struct B") || header.contains("union B");
        let Some(n) = header
            .rsplit(" B")
            .next()
            .filter(|_| named_shape)
            .and_then(|n| n.trim().parse::<usize>().ok())
        else {
            continue;
        };
        let mut members = Vec::new();
        let mut whole = (0, 0);
        for line in lines {
            let Some((at, rest)) = line.split_once('|') else {
                continue;
            };
            let rest = rest.trim();
            if let Some(sizes) = rest.strip_prefix("[sizeof=") {
                let numbers: Vec<u64> = sizes
                    .split(|c: char| !c.is_ascii_digit())
                    .filter_map(|n| n.parse().ok())
                    .collect();
                whole = (numbers[0], *numbers.last().expect("an alignment"));
                continue;
            }
            let named = rest
                .rsplit(' ')
                .next()
                .is_some_and(|name| name.starts_with('m') && name[1..].parse::<u64>().is_ok());
            if !named {
                continue;
            }
            let at = at.trim();
            members.push(match at.split_once(':') {
                Some((byte, range)) => {
                    let (low, high) = range.split_once('-').expect("a bit range");
                    let low: u64 = low.parse().expect("a bit");
                    let high: u64 = high.parse().expect("a bit");
                    let byte: u64 = byte.parse().expect("a byte");
                    (byte * 8 + low, Some(high - low + 1))
                }
                None => (at.parse::<u64>().expect("a byte") * 8, None),
            });
        }
        found[n] = Some((members, whole.0, whole.1));
    }
    found
        .into_iter()
        .map(|shape| shape.expect("clang dumped every shape"))
        .collect()
}

/// This compiler's own `sizeof` of each bit-field shape on `triple`, which
/// the type check works out apart from the IR's layout: the `@zN = global
/// i32 ...` it writes for the globals `clang_bit_layouts` declares.
fn our_sizes(triple: &str, shapes: &[(bool, Vec<BitMember>)]) -> Vec<u64> {
    let mut source = String::new();
    for (n, (union, members)) in shapes.iter().enumerate() {
        let keyword = if *union { "union" } else { "struct" };
        source.push_str(&format!("{keyword} B{n} {{"));
        for (k, &(rank, sign, width, named)) in members.iter().enumerate() {
            let name = if named {
                format!(" m{k}")
            } else {
                String::new()
            };
            let width = width.map_or(String::new(), |width| format!(" : {width}"));
            source.push_str(&format!(" {}{name}{width};", integer(rank, sign)));
        }
        source.push_str(&format!(" }};\nint z{n} = sizeof({keyword} B{n});\n"));
    }
    let dir = std::env::temp_dir().join(format!("safec_sizes_{triple}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the temporary directory is writable");
    let (file, module) = (dir.join("bits.c"), dir.join("bits.ll"));
    std::fs::write(&file, source).expect("the temporary directory is writable");
    let built = Command::new(env!("CARGO_BIN_EXE_safec"))
        .args([
            "--color", "never", "--safety", "off", "--emit", "llvm-ir", "--target", triple, "-o",
        ])
        .arg(&module)
        .arg(&file)
        .output()
        .expect("safec runs");
    assert!(
        built.status.success(),
        "safec read the bit-field shapes for {triple}:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let written = std::fs::read_to_string(&module).expect("safec wrote the module");
    let _ = std::fs::remove_dir_all(&dir);
    (0..shapes.len())
        .map(|n| {
            let prefix = format!("@z{n} = global i32 ");
            written
                .lines()
                .find_map(|line| line.strip_prefix(&prefix))
                .and_then(|size| size.trim().parse().ok())
                .unwrap_or_else(|| panic!("safec wrote `@z{n}` for {triple}"))
        })
        .collect()
}

/// Every bit-field shape is laid out by the IR as `clang` lays it out, on
/// every target (#1206): each named bit-field's first bit and width, each
/// ordinary member's offset, and the whole's size and alignment; and this
/// compiler's own `sizeof` of each, which the type check works out apart
/// from the IR, is the same size. Mutation: count every member named in
/// `Checker::struct_layout`; this fails at `B6` off MSVC. Mutation:
/// let a bit-field straddle its unit in `record::lay_out_itanium`; this
/// fails at `B2`. Mutation: let a Microsoft unit hold bit-fields of another
/// size; this fails at `B3`.
#[test]
fn the_layout_of_every_bit_field_shape_is_clangs() {
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
    let shapes = bit_shapes();
    for &target in Target::ALL {
        let triple = target.triple();
        let expected = clang_bit_layouts(triple, &shapes);
        let sizes = our_sizes(triple, &shapes);
        for (n, ((union, members), (want, size, align))) in shapes.iter().zip(expected).enumerate()
        {
            let mut unit = TranslationUnit::new(target);
            let declared: Vec<Declared> = members
                .iter()
                .map(|&(rank, sign, width, named)| Declared {
                    ty: unit.push_type(Ty::Integer { rank, sign }),
                    width,
                    named,
                })
                .collect();
            let id = if *union {
                unit.declare_union()
            } else {
                unit.declare_struct()
            };
            unit.complete_record(id, &declared)
                .expect("every bit-field shape is laid out");
            let layout = unit.struct_layout(id);
            let laid: Vec<(u64, Option<u64>)> = members
                .iter()
                .zip(&layout.members)
                .filter(|((_, _, _, named), _)| *named)
                .map(|(_, member)| match member.bits {
                    Some(bits) => (
                        member.offset * 8 + u64::from(bits.at),
                        Some(u64::from(bits.width)),
                    ),
                    None => (member.offset * 8, None),
                })
                .collect();
            assert_eq!(laid, want, "{triple}: the members of B{n}");
            assert_eq!(layout.size, size, "{triple}: the size of B{n}");
            assert_eq!(layout.align, align, "{triple}: the alignment of B{n}");
            assert_eq!(sizes[n], size, "{triple}: this compiler's `sizeof` of B{n}");
        }
    }
}
