struct Small {
    int a;
    int b;
};

struct Odd {
    char a;
    char b;
    char c;
};

struct Wide {
    int a;
    int b;
    int c;
};

struct Big {
    long long a;
    long long b;
    long long c;
};

static struct Small small(int a) {
    struct Small s;
    s.a = a;
    s.b = a + 1;
    return s;
}

static int small_sum(struct Small s) {
    return s.a + s.b;
}

static struct Odd odd(void) {
    struct Odd o;
    o.a = 1;
    o.b = 2;
    o.c = 3;
    return o;
}

static int odd_sum(struct Odd o) {
    return o.a + o.b + o.c;
}

static struct Wide wide(void) {
    struct Wide w;
    w.a = 4;
    w.b = 5;
    w.c = 6;
    return w;
}

static int wide_sum(struct Wide w) {
    return w.a + w.b + w.c;
}

static struct Big big(void) {
    struct Big b;
    b.a = 7;
    b.b = 8;
    b.c = 9;
    return b;
}

static long long big_sum(struct Big b) {
    b.a = b.a + 100;
    return b.a + b.b + b.c;
}

int main(void) {
    int r = 0;
    r = r + (small_sum(small(3)) == 7 ? 0 : 1);
    r = r + (odd_sum(odd()) == 6 ? 0 : 1);
    struct Wide w = wide();
    r = r + (wide_sum(w) == 15 && wide().c == 6 ? 0 : 1);
    struct Big b = big();
    r = r + (big_sum(b) == 124 && b.a == 7 ? 0 : 1);

    struct Small x = small(10);
    struct Small y;
    y = x;
    x.a = 0;
    r = r + (y.a == 10 && y.b == 11 && x.a == 0 ? 0 : 1);

    struct Small *p = &x;
    struct Small *q = &y;
    *p = *q;
    r = r + (x.a == 10 ? 0 : 1);
    return r;
}
