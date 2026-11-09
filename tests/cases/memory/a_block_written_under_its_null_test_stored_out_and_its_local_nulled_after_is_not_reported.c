void *malloc(int n);
void g(int ***out, int *x) {
    int **b = malloc(8);
    if (!out) return;
    if (b) *b = x;
    *out = b; b = 0;
}
