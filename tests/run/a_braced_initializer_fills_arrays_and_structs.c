struct P {
    int x, y;
};

struct Line {
    struct P a, b;
    int n;
};

struct Named {
    char name[4];
    int k;
};

struct Bits {
    int low : 3;
    int high : 5;
};

union U {
    int i;
    char c;
};

union Value {
    int i;
    long long l;
};

struct Gaps {
    char c;
    int : 5;
    int i;
    int rest[];
};

struct Padded {
    char c;
    int i;
};

int memcmp(const void *, const void *, unsigned long long);

int g[4] = {1, 2};
struct P gp = {7, 8};
struct P gs[3] = {{1, 2}, 3, 4};
int scalar = {5};
union U gu = {65};
struct Bits zero_bits = {0};
struct Named gnamed = {"st", 4};
char grows[2][3] = {{"ab"}, {"c"}};

static int dirty(void) {
    int junk[16];
    for (int i = 0; i < 16; i++) {
        junk[i] = 9;
    }
    return junk[3];
}

static struct Padded padded_after(int dirt) {
    int junk[16];
    for (int i = 0; i < 16; i++) {
        junk[i] = dirt;
    }
    struct Padded p = {1, 2};
    if (junk[0] != dirt) {
        p.c = 0;
    }
    return p;
}

static int zeroed(void) {
    union Value v = {0};
    struct P elided[] = {1, 2, 3, 4, 5};
    struct P s = {7, 8};
    struct P whole[] = {s, s, s};
    char h[2][3] = {{"ab"}, {"c"}};
    struct Gaps g = {1, 2};
    struct Padded one = padded_after(0x41);
    struct Padded two = padded_after(0x42);
    if (v.l != 0 || sizeof elided != 24 || elided[2].x != 5 || elided[2].y != 0) {
        return 10;
    }
    if (sizeof whole != 24 || whole[2].y != 8 || h[0][1] != 'b' || h[1][0] != 'c' || h[1][1] != 0) {
        return 11;
    }
    if (g.c != 1 || g.i != 2 || memcmp(&one, &two, sizeof one) != 0) {
        return 12;
    }
    return 0;
}

static int counted(void) {
    static int t[3] = {1, 2, 3};
    return t[2]++;
}

static int locals(void) {
    int a[3] = {0, 1, 2};
    int u[] = {5, 6};
    struct P s = {3, 4};
    struct Line l = {{1, 2}, {3, 4}, 5};
    struct P elided[2] = {1, 2, 3, 4};
    int m[2][2] = {1, 2, 3};
    int x = {7};
    int zeros[16] = {1};
    char rows[][4] = {"ab", "cde"};
    struct Named named = {"xy", 3};
    struct Named single = {"q"};
    const char *one[] = {"z"};
    struct P copied = {s.y, s.x};
    struct Line whole = {s, copied, 1};
    union U lu = {66};
    struct Bits bits = {3, 9};
    const int fixed[2] = {4, 5};
    int sum = 0;
    for (int i = 1; i < 16; i++) {
        sum += zeros[i];
    }
    if (a[0] + a[1] * 10 + a[2] * 100 != 210 || sizeof u != 8 || u[1] != 6) {
        return 1;
    }
    if (s.x + s.y * 10 != 43) {
        return 2;
    }
    if (l.a.x + l.a.y + l.b.x + l.b.y + l.n != 15 || elided[1].y != 4 || m[1][0] != 3 || m[1][1] != 0) {
        return 3;
    }
    if (x != 7 || sum != 0 || sizeof rows != 8 || rows[1][2] != 'e' || named.name[1] != 'y' || named.k != 3) {
        return 4;
    }
    if (copied.x != 4 || whole.a.x != 3 || whole.b.y != 3 || whole.n != 1 || lu.c != 66 || single.name[0] != 'q' || single.k != 0 || one[0][0] != 'z') {
        return 5;
    }
    if (bits.low != 3 || bits.high != 9 || fixed[1] != 5) {
        return 6;
    }
    return 0;
}

static int statics(void) {
    if (g[1] != 2 || g[3] != 0 || gp.y != 8 || gs[0].y != 2 || gs[1].x != 3 || gs[1].y != 4 || gs[2].x != 0) {
        return 7;
    }
    if (scalar != 5 || gu.c != 65 || counted() != 3 || counted() != 4) {
        return 8;
    }
    if (zero_bits.low != 0 || zero_bits.high != 0 || gnamed.name[1] != 't' || gnamed.k != 4 || grows[1][0] != 'c') {
        return 9;
    }
    return 0;
}

int main(void) {
    dirty();
    int failed = locals();
    if (failed != 0) {
        return failed;
    }
    dirty();
    failed = zeroed();
    if (failed != 0) {
        return failed;
    }
    return statics();
}
