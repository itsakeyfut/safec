void *malloc(int n);
void g(int ***out, int ***o2, int *x, int c) {
    int **b = malloc(8);
    int **t = b;
    if (!b) return;
    if (!out) return;
    if (!o2) return;
    *o2 = t;
    *out = b;
    if (c) *b = x;
}
