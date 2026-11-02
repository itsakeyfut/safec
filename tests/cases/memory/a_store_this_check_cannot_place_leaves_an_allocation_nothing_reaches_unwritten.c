void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int *** _Nonnull pw, int *x) {
    int **m = malloc(8);
    if (!m) return 0;
    int **w = *pw;
    if (!w) return 0;
    *w = x;
    int *r = *m;
    return r != 0;
}
