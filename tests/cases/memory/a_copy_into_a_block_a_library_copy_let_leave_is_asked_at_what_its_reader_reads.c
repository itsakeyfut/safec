void *malloc(int n); void *calloc(int n, int m); void free(void *p); void *memcpy(void *d, void *s, int n);
void f(int ***out) {
    if (!out) return;
    char *c = calloc(1, 3); if (!c) return;
    int **b = malloc(8); if (!b) return;
    int ***hb = malloc(8); if (!hb) return;
    *b = 0;
    *hb = b;
    memcpy(out, hb, 8);
    free(hb);
    void *vb = b; void **vv = vb;
    memcpy(vv, &c, 8);
}
