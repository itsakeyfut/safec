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

enum { K = 3 };

static int big[1 << 20] = {[(1 << 20) - 1] = 7, [5] = 1};
struct S gs = {.b = 2, .a = 1};
int ga[] = {[4] = 1, 2};
union U gu = {.l = 9};
struct L gl = {.s.b = 4, 5};
struct N gn = {.s = "abc", .s[1] = 88};
struct Anonymous gan = {.inner = 6, 7};

static int calls;

static int counted(int value) {
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
    if (s.a != 1 || s.b != 2 || a[0] != 1 || a[1] != 0 || a[2] != 3 || a[3] != 4 || sizeof u != 16) {
        return 1;
    }
    if (l.s.a != 9 || l.s.b != 8 || lu.l != 6 || m[0][1] != 2 || m[1][0] != 5 || m[1][1] != 6) {
        return 2;
    }
    if (an.inner != 6 || an.last != 7 || an.first != 0 || over[0] != 2 || calls != 0) {
        return 3;
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
    return 0;
}

int main(void) {
    int failed = locals();
    if (failed != 0) {
        return failed;
    }
    return statics();
}
