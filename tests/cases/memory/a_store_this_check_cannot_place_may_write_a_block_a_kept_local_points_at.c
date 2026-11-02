void *malloc(int n); void keep(int ***p);
int f(int **** _Nonnull ppw, int *x) {
    int **p = 0;
    keep(&p);
    int **m = malloc(8);
    if (!m) return 0;
    p = m;
    int ***t = *ppw;
    if (!t) return 0;
    **t = x;
    int *r = *m;
    return r != 0;
}
