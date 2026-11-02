void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(void) {
    int **m = malloc(8); if (!m) return 0;
    int *d[1];
    memcpy(d, m, 8);
    int *r = *m; if (!r) return 0;
    return *r;
}
