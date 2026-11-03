void *malloc(int n);
int f(int ***o, int *x, int c) {
    int **m1 = malloc(8); if (!m1) return 0;
    int **m2 = malloc(8); if (!m2) return 0;
    int ***t = malloc(8); if (!t) return 0;
    if (!o) return 0;
    *o = m1;
    if (c) *t = m1; else *t = m2;
    int **v = *t;
    if (!v) return 0;
    *v = x;
    int *r = *m1;
    return r != 0;
}
