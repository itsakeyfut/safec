void *malloc(int n);
int f(int k, int *x) {
    int *loc;
    int **p = &loc;
    int **q = malloc(8); if (!q) return 0;
    *q = x;
    if (k) p = q;
    int *r = *p;
    return r == x;
}
