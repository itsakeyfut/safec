void *malloc(int n);
void f(int *** _Nonnull out, int *x) {
    int **b = malloc(8);
    if (!b) return;
    *out = b;
    *b = x;
}
