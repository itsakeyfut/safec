void *malloc(int n);
void l(int *** _Nonnull out, int n, int *x) {
    for (int i = 0; i < n; i++) {
        int **b = malloc(8);
        if (!b) return;
        if (i != 3) {
            *out = b;
            *b = x;
        }
    }
}
