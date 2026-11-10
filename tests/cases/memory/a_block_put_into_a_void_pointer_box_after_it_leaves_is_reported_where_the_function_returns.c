void *malloc(int n);
void stored(void *** _Nonnull out) {
    int **b = malloc(8);
    void **box = malloc(8);
    if (!box) return;
    *out = box;
    *box = b;
}
