void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int ***pp) {
    if (!pp) return 0;
    int **m = malloc(8); if (!m) return 0;
    int ***o = malloc(8); if (!o) return 0;
    *o = m;
    memcpy(o, pp, 8);
    int *r = *m; if (!r) return 0;
    return *r;
}
