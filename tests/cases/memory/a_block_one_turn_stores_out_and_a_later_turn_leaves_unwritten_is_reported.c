void *malloc(int n);
void f(int *** _Nonnull out, int *x) {
    for (int i = 0; i < 2; i++) {
        int **b = malloc(8);
        if (!b) return;
        if (i == 0) *out = b; else *b = x;
    }
}
