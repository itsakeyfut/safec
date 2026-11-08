void *malloc(int n);
void free(void *p);
void f(int *** _Nonnull out) {
    int **b = malloc(8);
    if (!b) return;
    *out = b;
    free(b);
}
