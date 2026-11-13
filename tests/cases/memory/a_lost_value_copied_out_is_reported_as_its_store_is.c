// A value this check stopped following, let leave by a copy, is asked as
// its store `*out = v;` is: `SC0414` at the copy. The copy left out every
// lost answer, and said nothing where the store reported; found by the
// architecture review of 2026-11-13, on `main` too.
void *malloc(int n); void free(void *p); void *memcpy(void *d, void *s, int n);
void f(int **out, int k) {
    if (!out) return;
    int *o = 0;
    int t = 0;
    while (t < 2) {
        int *p = malloc(4);
        if (!p) return;
        if (t == 1) {
            int *v = o;
            memcpy(out, &v, 8);
        }
        o = p;
        free(p);
        t = t + 1;
    }
}
