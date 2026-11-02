void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(void) {
    int x = 1, y = 2;
    int *a[2]; a[0] = &x; a[1] = &y;
    int **m = malloc(16); if (!m) return 0;
    memcpy(m, a, 12);
    int *r = *m; if (!r) return 0;
    return *r;
}
