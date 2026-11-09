void *malloc(int n);
void g(int ***out, int *x, int c, int n) {
    int **b = malloc(8);
    if (!b || !out) return;
    int **t = b;
    for (int i = 0; i < n; i++) { *out = b; b = 0; if (c) *t = x; }
}
