struct S {
    int a, b;
};

struct L {
    struct S s;
    int n;
};

union U {
    int i;
    long long l;
};

struct N {
    char s[4];
    int k;
};

struct Anonymous {
    int first;
    struct {
        int inner;
    };
    int last;
};

struct Gap {
    int a;
    int : 4;
    int b;
};

struct T3 {
    struct S s;
};

union Same {
    long long n;
    long long m;
};

struct Holds {
    union Same v;
    long long after;
};

union Pointer {
    int *p;
    long long i;
};

enum { K = 3 };

static int big[1 << 20] = {[(1 << 20) - 1] = 7, [5] = 1};
struct S gs = {.b = 2, .a = 1};
int ga[] = {[4] = 1, 2};
union U gu = {.l = 9};
struct L gl = {.s.b = 4, 5};
struct N gn = {.s = "abc", .s[1] = 88};
struct Anonymous gan = {.inner = 6, 7};
struct Holds gholds = {.v.n = 1, .v.m = 2, .after = 3};
struct T3 gbraced = {.s = {.a = 1}, .s = {.b = 2}};
struct S grows[8] = {[5].a = 1, [1].a = 2, [5].b = 3};

static int calls;
static int order[4];

static int counted(int value) {
    calls++;
    return value;
}

static int logged(int value) {
    order[calls] = value;
    calls++;
    return value;
}

static int locals(void) {
    struct S s = {.b = 2, .a = 1};
    int a[5] = {[2] = 3, 4, [0] = 1};
    int u[] = {[K] = 1};
    struct L l = {.s = {7, 8}, .s.a = 9};
    union U lu = {.l = 6};
    int m[2][2] = {[1] = {5, 6}, [0][1] = 2};
    struct Anonymous an = {.inner = 6, 7};
    int over[2] = {[0] = counted(1), [0] = 2};
    struct L prefix = {.s.a = counted(1), .s = s};
    union U both = {.i = 1, .l = 2};
    struct L back = {.s.b = 4, .n = 5};
    int down[] = {[5] = 1, [0] = 2};
    struct Gap gap = {.b = 5};
    struct T3 braced = {.s = {.a = 1}, .s = {.b = 2}};
    union Pointer held = {.i = 5};
    if (s.a != 1 || s.b != 2 || a[0] != 1 || a[1] != 0 || a[2] != 3 || a[3] != 4 || sizeof u != 16) {
        return 1;
    }
    if (l.s.a != 9 || l.s.b != 8 || lu.l != 6 || m[0][1] != 2 || m[1][0] != 5 || m[1][1] != 6) {
        return 2;
    }
    if (an.inner != 6 || an.last != 7 || an.first != 0 || over[0] != 2 || calls != 0) {
        return 3;
    }
    if (prefix.s.a != 1 || both.l != 2 || back.s.b != 4 || back.n != 5 || sizeof down != 24 || down[5] != 1) {
        return 7;
    }
    if (gap.b != 5 || gap.a != 0 || braced.s.a != 0 || braced.s.b != 2 || held.i != 5) {
        return 8;
    }
    // In the object's order, as `clang` evaluates them, and an earlier
    // member of a union not at all.
    calls = 0;
    struct S ordered = {.b = logged(1), .a = logged(2)};
    union U last = {.i = logged(3), .l = logged(4)};
    if (calls != 3 || order[0] != 2 || order[1] != 1 || order[2] != 4 || ordered.a != 2 || last.l != 4) {
        return 11;
    }
    return 0;
}

static int statics(void) {
    if (big[5] != 1 || big[(1 << 20) - 1] != 7 || big[6] != 0 || gs.a != 1 || gs.b != 2) {
        return 4;
    }
    if (sizeof ga != 24 || ga[4] != 1 || ga[5] != 2 || gu.l != 9 || gl.s.b != 4 || gl.n != 5) {
        return 5;
    }
    if (gn.s[0] != 'a' || gn.s[1] != 'X' || gn.s[2] != 'c' || gan.inner != 6 || gan.last != 7) {
        return 6;
    }
    if (gholds.v.m != 2 || gholds.after != 3 || gbraced.s.a != 0 || gbraced.s.b != 2) {
        return 9;
    }
    if (grows[5].a != 1 || grows[5].b != 3 || grows[1].a != 2) {
        return 10;
    }
    return 0;
}

int main(void) {
    int failed = locals();
    if (failed != 0) {
        return failed;
    }
    return statics();
}
