void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void copied(int *** _Nonnull out) {
    int **b = malloc(8);
    void *v = &b;
    int **w = v;
    memcpy(out, w, 8);
}
