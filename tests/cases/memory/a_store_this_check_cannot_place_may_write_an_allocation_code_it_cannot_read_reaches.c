void *malloc(int n); void keep(int **p);
int f(int *** _Nonnull pw, int *x) {
    int **m = malloc(8);
    if (!m) return 0;
    keep(m);
    int **w = *pw;
    if (!w) return 0;
    *w = x;
    int *r = *m;
    return r != 0;
}
