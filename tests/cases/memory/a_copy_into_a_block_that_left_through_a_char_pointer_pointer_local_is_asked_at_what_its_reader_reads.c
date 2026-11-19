void *malloc(int n); void *memcpy(void *d, void *s, int n); void *realloc(void *p, int n);
void f(int ***out) {
    if (!out) return;
    char *c = malloc(3); if (!c) return;
    int **b = malloc(8); if (!b) return;
    *out = b;
    void *vb = b; char **vv = vb;
    memcpy(vv, &c, 8);
}
