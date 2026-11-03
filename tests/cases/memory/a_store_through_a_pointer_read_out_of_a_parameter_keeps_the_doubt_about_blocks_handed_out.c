void *malloc(int n);
int f(int ***o1, int ***o2, int ***q, int *x) {
    int **m1 = malloc(8); if (!m1) return 0;
    int **m2 = malloc(8); if (!m2) return 0;
    if (!o1) return 0;
    if (!o2) return 0;
    if (!q) return 0;
    *o1 = m1;
    *o2 = m2;
    int **v = *q;
    if (!v) return 0;
    *v = x;
    int *r = *m1;
    return r != 0;
}
