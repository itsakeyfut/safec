void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int ***out) {
    if (!out) return;
    char *c = malloc(3); if (!c) return;
    int **b = malloc(8); if (!b) return;
    *out = b;
    void *vb = b; void **vv = vb;
    void ***pv = malloc(8); if (!pv) return;
    *pv = vv;
    memcpy(*pv, &c, 8);
}
