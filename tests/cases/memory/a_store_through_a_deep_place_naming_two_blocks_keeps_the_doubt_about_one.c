void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int k, int c, int **pp) {
    if (!pp) return 0;
    int **m = malloc(8); if (!m) return 0;
    int **o = malloc(8); if (!o) return 0;
    int ***h = malloc(8); if (!h) return 0;
    if (k) *m = *pp;
    *h = o;
    if (c) *h = m;
    **h = *pp;
    int *r = *m; if (!r) return 0;
    return *r;
}
