void *malloc(int n);
void j(int *** _Nonnull out, int c) {
    int **b = malloc(8);
    if (!b) return;
    if (c) *out = b;
}
