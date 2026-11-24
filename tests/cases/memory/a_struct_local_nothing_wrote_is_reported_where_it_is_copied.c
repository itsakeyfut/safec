struct S {
    int *p;
};

union U {
    int *p;
    long n;
};

struct O {
    struct S s;
    int *q;
};

struct I {
    int a;
};

void g(struct S s);

void copied(void) {
    struct S t;
    struct S u = t;
}

void assigned_whole(void) {
    struct S t, u;
    u = t;
}

void a_union(void) {
    union U t;
    union U u = t;
}

void into_a_block(struct S *_Nonnull h) {
    struct S t;
    *h = t;
}

void passed(void) {
    struct S t;
    g(t);
}

void one_member_written(void) {
    struct S t;
    struct S u;
    t.p = 0;
    u = t;
}

void address_taken(void) {
    struct S t;
    struct S *a = &t;
    struct S u = t;
    (void)a;
}

void member_read_first(void) {
    struct O o;
    struct S v = o.s;
    struct O w = o;
}

void member_address_taken(void) {
    struct S t, u;
    u = t;
    int **q = &t.p;
    (void)q;
}

void written_on_both_arms(int c) {
    struct S t, u;
    if (c)
        t.p = 0;
    else
        t.p = 0;
    u = t;
}

void assigned_on_one_arm(int c, struct S v) {
    struct S t;
    if (c)
        t = v;
    struct S u = t;
}

void integers(void) {
    struct I t;
    struct I u = t;
}

void address_unused(void) {
    struct S t;
    (void)&t;
    struct S u = t;
}

void copied_on(struct S *_Nonnull src) {
    struct S a, b, c;
    a = *src;
    b = a;
    c = b;
}
