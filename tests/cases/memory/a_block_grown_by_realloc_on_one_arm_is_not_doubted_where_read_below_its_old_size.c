// `*m` reads the slot the old block held on either arm, which was written:
// the grown block's slot is found by `m`'s counted position, since the join
// keeps no element (#918's review).
void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(int k, int * _Nonnull x) {
    int **m = malloc(8);
    if (!m) return 0;
    *m = x;
    if (k) {
        int **g = realloc(m, 16);
        if (!g) return 0;
        m = g;
    }
    int *r = *m;
    return r == 0;
}
