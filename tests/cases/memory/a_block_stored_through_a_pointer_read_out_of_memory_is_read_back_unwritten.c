void *malloc(int n);
int f(int **** _Nonnull out) {
    int **b = malloc(8);
    if (!b) return 0;
    int ***t = *out;
    if (!t) return 0;
    *t = b;
    int *r = *b;
    return r != 0;
}
