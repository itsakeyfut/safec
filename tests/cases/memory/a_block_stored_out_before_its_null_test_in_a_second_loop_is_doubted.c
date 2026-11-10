void *malloc(int n);
void f(int *** _Nonnull out, int * _Nonnull x, int n) {
    for (int j = 0; j < n; j++) { }
    for (int i = 0; i < n; i++) {
        int **b = malloc(8);
        out[i] = b;
        if (!b) return;
        *b = x;
    }
}
