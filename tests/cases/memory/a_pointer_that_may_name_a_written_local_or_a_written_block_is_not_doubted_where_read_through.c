void *malloc(int n);
int f(int k, int *x) {
    int *w = x;
    int **p = &w;
    int **q = malloc(8); if (!q) return 0;
    *q = x;
    if (k) p = q;
    int *r = *p;
    return r == x;
}
