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
