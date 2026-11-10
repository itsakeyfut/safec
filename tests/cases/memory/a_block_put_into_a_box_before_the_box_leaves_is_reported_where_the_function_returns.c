void *malloc(int n);
void stored(int **** _Nonnull out) {
    int **b = malloc(8);
    int ***box = malloc(8);
    if (!box) return;
    *box = b;
    *out = box;
}
