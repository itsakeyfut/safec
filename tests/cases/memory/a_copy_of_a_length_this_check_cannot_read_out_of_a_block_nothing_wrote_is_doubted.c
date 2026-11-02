void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int *x, int k) {
    int **m = malloc(8); if (!m) return 0;
    int **n = malloc(8); if (!n) return 0;
    *n = x;
    memcpy(n, m, k);
    int *r = *n; if (!r) return 0;
    return *r;
}
