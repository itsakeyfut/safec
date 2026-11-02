void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int k, int *x) {
    int **m = malloc(8); if (!m) return 0;
    int **n = malloc(8); if (!n) return 0;
    *n = x;
    if (k) *m = x;
    memcpy(n, m, 8);
    int *r = *n;
    return r == x;
}
