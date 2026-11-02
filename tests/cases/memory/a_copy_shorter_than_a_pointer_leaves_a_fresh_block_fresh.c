void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int **pp) {
    if (!pp) return 0;
    int **m = malloc(8); if (!m) return 0;
    memcpy(m, pp, 1);
    int *r = *m; if (!r) return 0;
    return *r;
}
