void *malloc(int n);
void g(int ***out, int *x, int c, int d) {
    int **b = malloc(8);
    if (!b || !out) return;
    for (int i = 0; i < 2; i++) { *out = b; if (d) *b = x; if (c) return; b = 0; }
}
