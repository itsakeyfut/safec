void *malloc(int n); void *realloc(void *p, int n);
int f(int k, int *x) {
    int **m = malloc(8); if (!m) return 0;
    if (k) *m = x;
    int **n = realloc(m, 16); if (!n) return 0;
    int *r = *n; if (!r) return 0;
    return *r;
}
