// A subscript past the end of its row of an array of arrays, inside the
// whole object, is undefined (C17 6.5.6 p8; Annex J.2 names `a[1][7]` of
// `int a[4][5]`). Counted against the object, each of these was proved to
// fit after #924; each is doubted at the access, however the pointer into
// the row travels: copied, stored through a `void **`, handed back by
// `memcpy`, or moved back from the next row. An access inside a row is
// proved (#947).
void *malloc(int n);
void free(void *p);

void past_a_row(void) {
    int a[2][2];
    a[0][2] = 3;
}

int read_past_a_row(void) {
    int a[2][2];
    a[1][0] = 3;
    return a[0][2];
}

void past_a_row_of_a_block(void) {
    int (*m)[2] = malloc(16);
    if (!m) return;
    m[0][2] = 1;
    free(m);
}

int before_a_row(void) {
    int a[2][2];
    a[0][0] = 1;
    return a[1][-2];
}

void through_a_pointer_to_a_row(void) {
    int a[2][2];
    int (*pa)[2] = &a[0];
    (*pa)[2] = 1;
}

int through_a_pointer_to_an_element(void) {
    int a[2][2];
    a[1][0] = 1;
    int *p = &a[0][1];
    return p[2];
}

void the_example_the_standard_gives(void) {
    int a[4][5];
    a[1][7] = 0;
}

void through_a_void_pointer_slot(void) {
    int a[2][2];
    void *v;
    void **vp = &v;
    *vp = &a[0];
    int *q = v;
    if (!q) return;
    q[2] = 1;
}

void *memcpy(void *d, void *s, int n);

void through_what_memcpy_hands_back(void) {
    int a[2][2];
    int b[1];
    b[0] = 0;
    int *q = memcpy(&a[0], b, 4);
    if (!q) return;
    q[2] = 1;
}

void back_from_the_next_row(void) {
    int b[2];
    int (*r)[2] = &b;
    int *p = r[1];
    p[-1] = 1;
}

int inside_a_row(void) {
    int a[2][3];
    a[1][2] = 5;
    return a[1][2];
}
