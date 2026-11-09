void *malloc(int n);
void g(int ***out, int ***out2, int *x, int c) {
    int **b = malloc(8);
    if (!b) return;
    if (!out) return;
    if (!out2) return;
    *out = *out2 = b;
    if (c) *b = x;
}
