struct S {
    int c;
};

struct S take(struct S s) {
    return s;
}

void chained(struct S a, struct S b, struct S c) {
    a = b = c;
}

void conditional(struct S a, struct S b, struct S c, int k) {
    a = k ? b : c;
}

void comma(struct S a, struct S b, int k) {
    a = (k, b);
}

void argument(struct S a, struct S b, int k) {
    take(k ? a : b);
}

int member(struct S a, struct S b) {
    return (a = b).c;
}

union U {
    int c;
    long long l;
};

void discarded(union U u) {
    u;
}

void read_through(union U *p) {
    *p;
}
