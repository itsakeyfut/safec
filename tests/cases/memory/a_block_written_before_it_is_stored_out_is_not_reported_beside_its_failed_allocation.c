void *malloc(int n);
int f(int *** _Nonnull out, int *x) {
    int **b = malloc(8);
    if (!b) return -1;
    *b = x;
    *out = b;
    return 0;
}
