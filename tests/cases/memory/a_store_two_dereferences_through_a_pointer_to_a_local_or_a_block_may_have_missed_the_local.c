void *malloc(int n);
int f(int c, int x) {
    int *p;
    int **g = &p;
    if (c) { g = malloc(8); if (!g) return 0; }
    int ***h = &g;
    **h = &x;
    int *r = p;
    return r == 0;
}
