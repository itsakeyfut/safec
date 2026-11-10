void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void copied(int *** _Nonnull out, int *x) {
    int **b = malloc(8);
    if (b) *b = x;
    int **c = b;
    memcpy(out, &c, 8);
}
