void *malloc(int n); void *memcpy(void *d, void *s, int n);
void keep(int **p);
void f(void) {
    char *c = malloc(3); if (!c) return;
    int **b = malloc(8); if (!b) return;
    keep(b);
    void *vb = b; void **vv = vb;
    memcpy(vv, &c, 8);
}
