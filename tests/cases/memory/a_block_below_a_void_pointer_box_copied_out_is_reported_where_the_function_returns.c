void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void copied(void *** _Nonnull out) {
    int **b = malloc(8);
    void **box = malloc(8);
    if (!box) return;
    *box = b;
    void **c = box;
    memcpy(out, &c, 8);
}
