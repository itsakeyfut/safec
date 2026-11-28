void g(void *);

union Writable {
    char *a;
    const char *b;
};
static union Writable handed = {.b = "abc"};

int handed_on(void) {
    g(handed.a);
    return 0;
}

union Writable returned = {.b = "abc"};

void *returned_through_the_other(void) {
    return returned.a;
}

static const char *p = "a";
union Deep {
    const char *const *cp;
    char **pp;
};
union Deep two_levels_down = {.cp = &p};

struct Around {
    int n;
    union {
        const char *const *cp;
        char **pp;
    } v;
};
struct Around inside_a_struct = {1, {&p}};

union BesideAnInt {
    const char *b;
    long long n;
};
union BesideAnInt beside_an_int = {.b = "abc"};

union AllConst {
    const char *a;
    const char *b;
};
union AllConst every_reader_const = {.b = "abc"};

union OverAStruct {
    struct {
        const char *const *cp;
        int n;
    } s;
    char **pp;
};
union OverAStruct a_struct_in_the_union = {.s = {&p, 1}};
