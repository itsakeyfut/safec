struct S {
    int *p;
};

union U {
    int *p;
    long n;
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
