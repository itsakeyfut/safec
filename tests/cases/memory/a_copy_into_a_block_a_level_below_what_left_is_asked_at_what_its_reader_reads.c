void *malloc(int n); void *calloc(int n, int m); void *memcpy(void *d, void *s, int n);
void f(int ****out) {
    if (!out) return;
    char *c = calloc(1, 3); if (!c) return;
    int **b = malloc(8); if (!b) return;
    int ***box = malloc(8); if (!box) return;
    *b = 0;
    *box = b;
    *out = box;
    void *vb = b; void **vv = vb;
    memcpy(vv, &c, 8);
}
