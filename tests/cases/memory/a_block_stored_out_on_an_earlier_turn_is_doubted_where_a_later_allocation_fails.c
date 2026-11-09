void *malloc(int n);
void f(int *** _Nonnull out) {
    *out = 0;
    for (;;) {
        int **b = malloc(8);
        if (!b) return;
        *out = b;
    }
}
