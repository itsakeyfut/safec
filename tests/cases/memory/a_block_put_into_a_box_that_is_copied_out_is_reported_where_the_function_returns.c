void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void copied(int **** _Nonnull out) {
    int **b = malloc(8);
    int ***box = malloc(8);
    if (!box) return;
    *box = b;
    int ***c = box;
    memcpy(out, &c, 8);
}
