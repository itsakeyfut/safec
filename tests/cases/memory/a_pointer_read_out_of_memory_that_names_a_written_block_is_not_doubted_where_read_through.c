void *malloc(int n);
int f(int *x) {
    int **m = malloc(8); if (!m) return 0;
    *m = x;
    int ***t = malloc(8); if (!t) return 0;
    *t = m;
    int **u = *t;
    if (!u) return 0;
    int *r = *u;
    return r != 0;
}
