void *malloc(int n);
void f(int ***out) {
    if (!out) return;
    char *c = malloc(3);
    if (!c) return;
    int **b = malloc(8);
    if (!b) return;
    *out = b;
    void *vb = b;
    void **vv = vb;
    *vv = c;
}
