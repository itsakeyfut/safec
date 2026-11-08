void *malloc(int n);
void f(int **** _Nonnull out) {
    int **b = malloc(8);
    if (!b) return;
    int ***t = *out;
    if (!t) return;
    *t = b;
}
