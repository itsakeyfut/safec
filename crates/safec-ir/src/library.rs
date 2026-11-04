//! What this crate reads in a library function's name: one row per function,
//! [`LIBRARY`], and the kinds a row is made of.
//!
//! At crate level rather than inside the memory check, its only reader so
//! far, so that an analysis besides it can read these rows rather than keep
//! a copy of them to drift. See ADR-0063.

/// What the memory check can read in a callee's name.
///
/// By name because nothing else is available: no annotation says what a
/// function does to what it is passed. The nullability specifiers say only
/// whether a pointer a function takes or returns may be null, which is
/// ADR-0037 and ADR-0050.
/// Besides `malloc` and `free`, the names read are the library functions
/// ADR-0039 lists, each for what its clause says it does to what it is handed.
/// C17 7.1.3 reserves the identifiers the library declares, so a program
/// that defines its own `free` has no behaviour C defines. `clang -std=c17
/// -pedantic-errors` does not diagnose one, measured, so a program that does it
/// anyway is read wrongly here and there is no way to tell from inside.
/// **No `PartialEq`**, so that every reader is a `match` and a new kind is
/// `error[E0004]` wherever it has to be answered. A comparison lets it through
/// in silence: the rule for what an opaque call returns was once gated by one.
/// `matches!` and a `_` arm are that comparison spelled differently, so
/// neither is used on it. Measured by adding a variant: every reader in the
/// memory check fails to compile.
#[derive(Clone, Copy)]
pub(crate) enum Callee {
    /// C17 7.22.3.3's `free`.
    Frees,
    /// C17 7.22.3.4's `malloc`, and 7.22.3.1's `aligned_alloc` and 7.22.3.2's
    /// `calloc`, which 7.22.3 p1 holds to the same thing: a pointer to the
    /// start of an object disjoint from any other. Read so that its arguments
    /// are left alone, and so that what it returns is a fresh allocation nobody
    /// else has, rather than one an opaque call may hand back. An allocation
    /// is otherwise named by where it landed rather than by which function made
    /// it. See ADR-0039.
    Allocates,
    /// C17 7.22.3.5's `realloc`.
    ///
    /// Its first argument may be freed, and is not when the call fails with a
    /// nonzero size (p3), so what it names becomes unproven rather than freed.
    /// What it returns is a fresh allocation, as `malloc`'s is, holding what
    /// the old one held (p2). See ADR-0039.
    Reallocates,
    /// A C library function that frees nothing and returns its first argument:
    /// `memset`, `strcpy`, `strncpy`, `strcat` and `strncat`, whose clauses in
    /// C17 7.24.2 to 7.24.6 each say so.
    ///
    /// **What it is handed is exposed all the same**, because it copies bytes
    /// and a pointer is bytes, and a local whose address it is handed may be
    /// written through it. See ADR-0039.
    ReturnsFirst,
    /// `memcpy` and `memmove`: what [`Callee::ReturnsFirst`] is, and a copy of
    /// an object besides, so that what the destination may contain gains what
    /// the source contains (C17 7.24.2.1 and 7.24.2.2 copy the object's bytes).
    ///
    /// **Not the string functions**, whose copies end at a null byte (7.24.2.3
    /// and on), which a pointer's representation may hold, so a stored pointer
    /// is not copied whole by them. A variant of its own rather than a name
    /// read inside the transfer, so that every reader of the callee answers
    /// for it. See ADR-0039.
    Copies,
    /// One of [`DOES_NOT_RETURN`](crate::ir::DOES_NOT_RETURN): `abort`,
    /// `exit`, `_Exit` and `quick_exit`, believed by name as the lowering and
    /// the reserved-name check believe them (ADR-0051).
    ///
    /// **It touches nothing.** What it is handed is asked as an argument, so
    /// `exit(*p)` reads `*p`, but it exposes, frees and returns nothing. The
    /// lowering keeps its edge inside a larger full expression, and read as
    /// [`Callee::Opaque`] that edge carried a call that may have freed
    /// everything it could reach to code after it: `n > 0 || (abort(), 0);
    /// use(p);` doubted `p` (#586). No execution continues past it, so the
    /// value before it is the one that claims least beyond it.
    Ends,
    /// Anything else. It may free what it was passed and this cannot tell.
    Opaque,
}

/// How far a library call reaches through one pointer it is handed.
///
/// See [`LIBRARY`], which is the only place one is written.
#[derive(Clone, Copy)]
pub(crate) enum Reach {
    /// As many bytes as the argument at this index says, read as the
    /// parameter receives it.
    Length(usize),
    /// As far as a string runs, which the memory check does not read, so it is
    /// never shown to fit.
    String,
}

/// How many bytes an allocation function makes, out of its arguments.
#[derive(Clone, Copy)]
pub(crate) enum Makes {
    /// The argument at this index.
    Argument(usize),
    /// The product of the two arguments at these indices, as `calloc`'s.
    Product(usize, usize),
}

/// What an allocation function leaves in the object it makes.
#[derive(Clone, Copy)]
pub(crate) enum Leaves {
    /// Indeterminate bytes: `malloc` (C17 7.22.3.4 p2) and `aligned_alloc`
    /// (7.22.3.1 p2). The allocation is unwritten, `Known::init`. See ADR-0057.
    Indeterminate,
    /// Zero bits, which is a null pointer on every target here: `calloc`
    /// (7.22.3.2 p2).
    ZeroBits,
    /// What the object it was handed held, up to the lesser of the two
    /// sizes (7.22.3.5 p2), and indeterminate bytes where it was handed a
    /// null pointer, since it then behaves as `malloc` for the size (p3):
    /// `realloc`. Null is what the memory check's value can say certainly, a
    /// constant `0` or a local that names nothing (`Known::names_nothing`);
    /// a pointer null on one path only is read as the old object, because
    /// the value has no "may be null" to tell it by (#689).
    TheOld,
}

/// A library function the memory check reads by name.
///
/// **One row says everything that is read about the name**, and a row that
/// leaves a field out is `error[E0063]`: what it does to what it is handed,
/// for the memory transfer; how far it reaches through which argument, for
/// whether an access fits; what it makes, where it allocates; and what it
/// leaves in what it makes. A fact about a name read outside the table is a
/// second reader to drift: `realloc(NULL, n)` was read as copying an old
/// object while the name `malloc` alone was read as leaving indeterminate
/// bytes (#689). Kept as
/// three readers matching names of their own, adding a function that takes
/// a length to the first and forgetting the second left `memmove(p, q, 64)`
/// on four bytes silent with every test green. Found by review. See
/// ADR-0060.
pub(crate) struct Library {
    /// What the source calls it.
    pub(crate) name: &'static str,
    /// What it does to what it is handed.
    pub(crate) callee: Callee,
    /// How far it reaches through each argument it reads or writes through.
    pub(crate) reaches: &'static [(usize, Reach)],
    /// How many bytes it makes, where it makes an allocation.
    pub(crate) makes: Option<Makes>,
    /// What it leaves in what it makes: `Some` exactly where `makes` is.
    pub(crate) leaves: Option<Leaves>,
}

/// Every library function the memory check reads by name, with what each is
/// read as. A name not here is [`Callee::Opaque`] and reaches nothing it
/// asks about: it is believed, as a parameter is.
///
/// The clauses: `free` 7.22.3.3, `malloc` 7.22.3.4, `calloc` 7.22.3.2,
/// `aligned_alloc` 7.22.3.1, `realloc` 7.22.3.5; `memcpy` 7.24.2.1,
/// `memmove` 7.24.2.2, `strcpy` 7.24.2.3, `strncpy` 7.24.2.4, which writes
/// exactly `n` characters (p2, p3), `strcat` 7.24.3.1, `strncat` 7.24.3.2,
/// which appends after a string whose length nothing here reads; `memcmp`
/// 7.24.4.1, `strcmp` 7.24.4.2, `strncmp` 7.24.4.4, which stops at a null
/// character before `n`; `memchr` 7.24.5.1, `strchr` 7.24.5.2, `strrchr`
/// 7.24.5.5, `strstr` 7.24.5.7; `memset` 7.24.6.1, `strlen` 7.24.6.3.
pub(crate) const LIBRARY: &[Library] = &[
    Library {
        name: "free",
        callee: Callee::Frees,
        reaches: &[],
        makes: None,
        leaves: None,
    },
    Library {
        name: "malloc",
        callee: Callee::Allocates,
        reaches: &[],
        makes: Some(Makes::Argument(0)),
        leaves: Some(Leaves::Indeterminate),
    },
    Library {
        name: "calloc",
        callee: Callee::Allocates,
        reaches: &[],
        makes: Some(Makes::Product(0, 1)),
        leaves: Some(Leaves::ZeroBits),
    },
    Library {
        name: "aligned_alloc",
        callee: Callee::Allocates,
        reaches: &[],
        makes: Some(Makes::Argument(1)),
        leaves: Some(Leaves::Indeterminate),
    },
    Library {
        name: "realloc",
        callee: Callee::Reallocates,
        reaches: &[],
        makes: Some(Makes::Argument(1)),
        leaves: Some(Leaves::TheOld),
    },
    Library {
        name: "memcpy",
        callee: Callee::Copies,
        reaches: &[(0, Reach::Length(2)), (1, Reach::Length(2))],
        makes: None,
        leaves: None,
    },
    Library {
        name: "memmove",
        callee: Callee::Copies,
        reaches: &[(0, Reach::Length(2)), (1, Reach::Length(2))],
        makes: None,
        leaves: None,
    },
    Library {
        name: "memset",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::Length(2))],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strcpy",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strncpy",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::Length(2)), (1, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strcat",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strncat",
        callee: Callee::ReturnsFirst,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "memcmp",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::Length(2)), (1, Reach::Length(2))],
        makes: None,
        leaves: None,
    },
    Library {
        name: "memchr",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::Length(2))],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strlen",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strcmp",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strncmp",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strchr",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strrchr",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String)],
        makes: None,
        leaves: None,
    },
    Library {
        name: "strstr",
        callee: Callee::Opaque,
        reaches: &[(0, Reach::String), (1, Reach::String)],
        makes: None,
        leaves: None,
    },
];
