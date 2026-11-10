void *malloc(int n);
void f(int *** _Nonnull out, int * _Nonnull x) {
    int **b = malloc(8);
    *out = b;
    if (!b) return;
    *b = x;
}
