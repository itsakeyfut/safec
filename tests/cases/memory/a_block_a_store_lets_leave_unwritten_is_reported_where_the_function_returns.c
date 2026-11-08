void *malloc(int n);
void stored(int *** _Nonnull out) {
    int **b = malloc(8);
    *out = b;
}
