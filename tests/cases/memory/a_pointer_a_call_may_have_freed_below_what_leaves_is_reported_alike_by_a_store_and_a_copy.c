// A pointer a call this check cannot read may have freed, one level below
// what leaves the function: the store and the library copy of the same `b`
// are one write, and each is `SC0414` at the write. The copy asked only
// what was freed below, not what this check stopped following, and was
// the argument door's `SC0407`; found by the architecture review of
// 2026-11-13.
void *malloc(int n); void *memcpy(void *d, void *s, int n);
void keep(int **pp); void release(void);
void stored(int ***out) {
    int *x = malloc(4);
    if (!x) return;
    int **b = malloc(8);
    if (!b) return;
    keep(&x);
    *b = x;
    release();
    if (!out) return;
    *out = b;
}
void copied(int ***out) {
    int *x = malloc(4);
    if (!x) return;
    int **b = malloc(8);
    if (!b) return;
    keep(&x);
    *b = x;
    release();
    if (!out) return;
    memcpy(out, &b, 8);
}
