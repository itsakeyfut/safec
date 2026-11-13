// `*p` may read bytes either of two copies carried, so the report names
// neither, rather than one of them (#752).
void *malloc(int n); void *calloc(int n, int m); void *memcpy(void *d, void *s, int n);
int f(int k) {
    int **a = malloc(8); if (!a) return 0;
    int **b = malloc(8); if (!b) return 0;
    int **n = calloc(1, 8); if (!n) return 0;
    int **o = calloc(1, 8); if (!o) return 0;
    memcpy(n, a, 8);
    memcpy(o, b, 8);
    int **p = n;
    if (k) p = o;
    int *r = *p;
    return r == 0;
}
