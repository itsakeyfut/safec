void *malloc(int n);
void g(int ***out, int *x, int c) {
    int **b = malloc(8);
    if (!out) return;
    if (c) { if (b) *b = x; }
    if (!b) { *out = b; }
}
