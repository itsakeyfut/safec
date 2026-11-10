void *malloc(int n);
void stored(int *** _Nonnull out) {
    int *a = malloc(4);
    int **box = malloc(8);
    if (!box) return;
    *box = a;
    *out = box;
}
