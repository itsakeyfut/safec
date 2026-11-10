void *malloc(int n);
void f(int *** _Nonnull out, int * _Nonnull x, int c) {
    int **b = malloc(8);
    *out = b;
    if (!b) return;
    if (c) *b = x;
}
