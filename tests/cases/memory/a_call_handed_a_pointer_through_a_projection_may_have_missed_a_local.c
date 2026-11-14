// A call this check cannot read, handed a pointer spelled through a
// dereference or an index, is asked what that pointer may name, as one
// handed as a local is: where it may be `&a` or `&b`, or `&a` or null, the
// call may have written the other, and each read of `a` below is `SC0411`
// (#949). A dereference certainly of one local hands that local's value,
// so `*pp` with `p` certainly `&a` is believed to write `a`.
void *malloc(int n);
void fill(int **p);

int through_a_dereference(int k) {
    int *a;
    int *b;
    int **p = &a;
    if (k) p = &b;
    int ***pp = &p;
    fill(*pp);
    return a == 0;
}

int through_an_index(int k) {
    int *a;
    int *b;
    int **ps[1];
    ps[0] = &a;
    if (k) ps[0] = &b;
    fill(ps[0]);
    return a == 0;
}

int through_a_block(int k) {
    int *a;
    int *b;
    int ***box = malloc(8);
    if (!box) return 0;
    *box = &a;
    if (k) *box = &b;
    fill(*box);
    return a == 0;
}

int null_through_a_dereference(int k) {
    int *a;
    int **p = 0;
    if (k) p = &a;
    int ***pp = &p;
    fill(*pp);
    return a == 0;
}

int null_through_an_index(int k) {
    int *a;
    int **q[1];
    q[0] = 0;
    if (k) q[0] = &a;
    fill(q[0]);
    return a == 0;
}

int one_local_through_a_dereference(void) {
    int *a;
    int **p = &a;
    int ***pp = &p;
    fill(*pp);
    return a == 0;
}
