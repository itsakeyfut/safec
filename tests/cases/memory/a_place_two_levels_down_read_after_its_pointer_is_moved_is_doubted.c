void *malloc(int n);
void g(int *p);
int f(int c, int **pp) {
    if (!pp) return 0;
    int **m = malloc(8); if (!m) return 0;
    int **o = malloc(8); if (!o) return 0;
    int **q = malloc(8); if (!q) return 0;
    int ***h = malloc(8); if (!h) return 0;
    *h = o;
    if (c) *h = m;
    **h = *pp; *h = q; g(**h);
    return 0;
}
