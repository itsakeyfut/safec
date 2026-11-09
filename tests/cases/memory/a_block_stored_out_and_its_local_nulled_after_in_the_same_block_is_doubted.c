void *malloc(int n);
void g(int ***out, int *x, int c) {
    int **b = malloc(8);
    if (!b || !out) return;
    int **t = b;
    *out = b; b = 0;
    if (c) *t = x;
}
