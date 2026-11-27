void g(void *);

union Writable {
    char *a;
    const char *b;
};

union AllConst {
    const char *a;
    const char *b;
};

union BesideAnInt {
    const char *b;
    long long n;
};

struct Anonymous {
    int tag;
    union {
        char *a;
        const char *b;
    };
};

static union Writable writable;
static union AllConst all_const;
static union BesideAnInt beside_an_int;
static struct Anonymous anonymous;

void literal_read_as_writable(void) {
    writable.b = "abc";
    g(writable.a);
}

void every_reader_const(void) {
    all_const.b = "abc";
}

void an_int_reads_no_level(void) {
    beside_an_int.b = "abc";
}

void through_an_anonymous_union(void) {
    anonymous.b = "abc";
}

union Inner {
    const char *b;
    int n;
};

union Outer {
    union Inner in;
    char *a;
};

union HoldsAStruct {
    struct {
        const char *b;
        int x;
    } s;
    char *a;
};

struct Holder {
    int k;
    union Writable u;
};

static union Outer outer;
static union HoldsAStruct holds_a_struct;

void a_union_of_a_union(void) {
    outer.in.b = "abc";
    g(outer.a);
}

void a_struct_in_a_union(void) {
    holds_a_struct.s.b = "abc";
    g(holds_a_struct.a);
}

void through_a_pointer(struct Holder *h) {
    if (h) {
        h->u.b = "abc";
    }
}

struct ConstFirst {
    const char *q;
    int k;
};

union ConstStruct {
    const char *s;
    struct ConstFirst t;
};

union ConstArray {
    const char *s;
    const char *arr[2];
};

union ConstUnion {
    const char *s;
    union {
        const char *t;
        int n;
    } in;
};

union BesideCode {
    const char *s;
    void (*f)(void);
};

static union ConstStruct const_struct;
static union ConstArray const_array;
static union ConstUnion const_union;
static union BesideCode beside_code;

void a_const_struct_reader(void) {
    const_struct.s = "abc";
}

void a_const_array_reader(void) {
    const_array.s = "abc";
}

void a_const_union_reader(void) {
    const_union.s = "abc";
}

void a_function_pointer_reads_no_level(void) {
    beside_code.s = "abc";
}

struct NotAUnion {
    char *a;
    const char *b;
};

union ArrayReader {
    const char *b;
    char *arr[2];
};

union StructReaderFirst {
    const char *b;
    struct {
        char *p;
        int k;
    } first;
};

union StructReaderSecond {
    const char *b;
    struct {
        int k;
        char *p;
    } second;
};

union ReaderAfter {
    char *a;
    const char *b;
};

union TwoDeep {
    char **a;
    const char **b;
};

struct TwoAnonymous {
    int tag;
    struct {
        int k;
        union {
            char *a;
            const char *b;
        };
    };
};

static struct NotAUnion not_a_union;
static union ArrayReader array_reader;
static union StructReaderFirst struct_reader_first;
static union StructReaderSecond struct_reader_second;
static union ReaderAfter reader_after;
static union TwoDeep two_deep;
static struct TwoAnonymous two_anonymous;
static const char *literal = "abc";

void a_struct_is_no_union(void) {
    not_a_union.b = "abc";
}

void an_array_of_writable_pointers_reads_it(void) {
    array_reader.b = "abc";
}

void a_struct_whose_first_pointer_is_writable_reads_it(void) {
    struct_reader_first.b = "abc";
}

void a_struct_whose_pointer_is_elsewhere_does_not(void) {
    struct_reader_second.b = "abc";
}

void a_reader_declared_after_the_writer(void) {
    reader_after.b = "abc";
}

void two_levels_down(void) {
    two_deep.b = &literal;
}

void through_two_anonymous_members(void) {
    two_anonymous.b = "abc";
}
