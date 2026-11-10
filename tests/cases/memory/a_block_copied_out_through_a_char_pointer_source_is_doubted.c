void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void copied(int *** _Nonnull out) {
    int **b = malloc(8);
    void *v = &b;
    char *c = v;
    memcpy(out, c, 8);
}
