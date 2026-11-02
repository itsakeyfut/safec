void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int k, int c, int **pp) {
    if (!pp) return 0;
    int **m = malloc(8); if (!m) return 0;
    int *loc = 0;
    if (k) *m = *pp;
    int **p = &loc;
    if (c) p = m;
    memcpy(p, pp, 8);
    int *r = *m; if (!r) return 0;
    return *r + (loc ? 1 : 0);
}
