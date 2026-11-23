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

struct Pair {
    long long a;
    long long b;
};

struct Big {
    long long a;
    long long b;
    long long c;
};

struct Odd odd(struct Odd o) {
    return o;
}

struct Wide wide(struct Wide w) {
    return w;
}

long long last(long long a, long long b, long long c, long long d, long long e, struct Pair p) {
    return p.a + a + b + c + d + e;
}

struct Big big(struct Big b) {
    return b;
}

int main(void) {
    struct Odd o;
    struct Wide w;
    struct Pair p;
    struct Big b;
    o.a = 1;
    o.b = 2;
    o.c = 3;
    w.a = 4;
    w.b = 5;
    w.c = 6;
    p.a = 7;
    p.b = 8;
    b.a = 9;
    b.b = 10;
    b.c = 11;
    return odd(o).c + wide(w).c + (int)last(0, 0, 0, 0, 0, p) + (int)big(b).c - 27;
}
