void *malloc(int n);
int f(int k, int c, int **pp) {
    if (!pp) return 0;
    int **m = malloc(8);
    if (!m) return 0;
    int **o = malloc(8);
    if (!o) return 0;
    if (k) *m = *pp;
    int **p = o;
    if (c) p = m;
    *p = *pp;
    int *r = *m;
    if (!r) return 0;
    return *r;
}
