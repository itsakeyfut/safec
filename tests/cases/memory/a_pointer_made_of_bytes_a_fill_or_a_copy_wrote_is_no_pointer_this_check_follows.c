// Bytes a `memset` of a byte that may not be zero writes over a slot of
// pointers, or a library copy writes there out of memory that holds no
// pointer, are no address of anything: read back and dereferenced after a
// null test, each is `SC0402`, since nothing this check names is what they
// point at (C17 6.5.3.2 p4, 6.2.6.1 p5). Read as named nothing, a null test
// took each for null and the dereference fit vacuously (#948). Through a
// `void *` the same, since the bytes are marked where they land and a copy
// carries the mark. A fill of zero is read as a null here, and a copy into
// a slot of `int`s is read back as `int`s.
void *malloc(int n);
void *memset(void *d, int c, int n);
void *memcpy(void *d, void *s, int n);

int filled_local_array(void) {
    int *a[1];
    memset(a, 1, 8);
    int *r = a[0];
    if (!r) return 0;
    return *r;
}

int filled_local(void) {
    int *q;
    memset(&q, 1, 8);
    int *r = q;
    if (!r) return 0;
    return *r;
}

int filled_over_a_stored_pointer(void) {
    int x = 1;
    int **b = malloc(8);
    if (!b) return 0;
    *b = &x;
    memset(b, 1, 8);
    int *r = *b;
    if (!r) return 0;
    return *r;
}

int filled_by_a_byte_not_known(int n) {
    int *a[1];
    memset(a, n, 8);
    int *r = a[0];
    if (!r) return 0;
    return *r;
}

int copied_out_of_chars(void) {
    int **b = malloc(8);
    if (!b) return 0;
    char s[8];
    memset(s, 1, 8);
    memcpy(b, s, 8);
    int *r = b[0];
    if (!r) return 0;
    return *r;
}

int filled_with_zero(void) {
    int *a[1];
    memset(a, 0, 8);
    int *r = a[0];
    if (!r) return 0;
    return *r;
}

int copied_into_ints(void) {
    int *m = malloc(8);
    if (!m) return 0;
    char s[8];
    memset(s, 1, 8);
    memcpy(m, s, 8);
    return m[0];
}

int filled_through_a_void_pointer(void) {
    int x = 1;
    int **b = malloc(8);
    if (!b) return 0;
    *b = &x;
    void *v = b;
    memset(v, 1, 8);
    int *r = *b;
    if (!r) return 0;
    return *r;
}

int copied_through_a_void_pointer(void) {
    int **b = malloc(8);
    if (!b) return 0;
    char s[8];
    memset(s, 1, 8);
    void *v = b;
    memcpy(v, s, 8);
    int *r = *b;
    if (!r) return 0;
    return *r;
}

int copied_out_of_a_filled_buffer_through_a_void_pointer(void) {
    int **b = malloc(8);
    if (!b) return 0;
    char s[8];
    memset(s, 1, 8);
    void *v = s;
    memcpy(b, v, 8);
    int *r = *b;
    if (!r) return 0;
    return *r;
}

int copied_out_of_ints_a_store_wrote(void) {
    int **b = malloc(8);
    if (!b) return 0;
    int s[2];
    s[0] = 1;
    s[1] = 1;
    memcpy(b, s, 8);
    int *r = *b;
    if (!r) return 0;
    return *r;
}
