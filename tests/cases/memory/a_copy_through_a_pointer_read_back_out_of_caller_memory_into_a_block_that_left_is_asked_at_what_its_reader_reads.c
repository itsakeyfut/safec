void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int ***out, void ***qq) {
    if (!out || !qq) return;
    char *c = malloc(3); if (!c) return;
    int **b = malloc(8); if (!b) return;
    *b = 0;
    *out = b;
    void *vb = b; void **vv = vb;
    *qq = vv;
    void **w = *qq;
    if (!w) return;
    memcpy(w, &c, 8);
}
