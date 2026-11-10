void *malloc(int n);
void stored(int **** _Nonnull out, int *x) {
    int **b = malloc(8);
    if (b) *b = x;
    int ***box = malloc(8);
    if (!box) return;
    *box = b;
    *out = box;
}
