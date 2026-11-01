void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n) {
    return d;
}

void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (!x) return;
    int **b = malloc(8);
    if (!b) return;
    *b = x;
    memcpy(pp, b, 8);
    free(b);
    free(x);
}
