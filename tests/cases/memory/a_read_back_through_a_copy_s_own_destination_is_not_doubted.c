void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int c, int **pp) {
    if (!pp) return 0;
    int **m = malloc(8); if (!m) return 0;
    int **o = malloc(8); if (!o) return 0;
    int **p = o;
    if (c) p = m;
    memcpy(p, pp, 8);
    int *r = *p;
    if (!r) return 0;
    return *r;
}
