void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int ****out) {
    if (!out) return;
    int *c = malloc(4); if (!c) return;
    int **b = malloc(8); if (!b) return;
    int ***box = malloc(8); if (!box) return;
    *box = b;
    *out = box;
    void *vb = b; void **vv = vb;
    memcpy(vv, &c, 8);
}
