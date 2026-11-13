// A pointer freed on one arm only, let leave by a store and by a library
// copy: `SC0414` for both, as a pointer freed on one arm one level below
// what leaves already is. The copy asked the value itself of what is
// certainly freed alone, and fell through to the argument door's
// `SC0407`; found by the architecture review of 2026-11-13.
void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void stored(int **out, int c) {
    int *x = malloc(4);
    if (!x) return;
    if (c) free(x);
    if (!out) return;
    *out = x;
}
void copied(int **out, int c) {
    int *x = malloc(4);
    if (!x) return;
    if (c) free(x);
    if (!out) return;
    memcpy(out, &x, 8);
}
