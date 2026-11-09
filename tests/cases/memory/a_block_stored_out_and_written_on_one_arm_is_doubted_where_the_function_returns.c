void *malloc(int n);
void g(int *** _Nonnull out, int c, int *x) {
    int **b = malloc(8);
    if (!b) return;
    *out = b;
    if (c) *b = x;
}
