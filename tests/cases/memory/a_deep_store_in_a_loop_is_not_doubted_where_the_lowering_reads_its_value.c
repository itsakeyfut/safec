void *malloc(int n);
int f(int k, int c, int **pp) {
    if (!pp) return 0;
    int **m = malloc(8); if (!m) return 0;
    int **o = malloc(8); if (!o) return 0;
    int ***h = malloc(8); if (!h) return 0;
    *h = o;
    if (c) *h = m;
    for (int i = 0; i < k; i++) { **h = *pp; }
    return 0;
}
