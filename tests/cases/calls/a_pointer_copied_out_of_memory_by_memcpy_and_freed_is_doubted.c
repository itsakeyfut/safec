void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);

void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    int **b = malloc(8);
    if (!x || !b) return;
    *b = x;
    memcpy(pp, b, 8);
    free(b);
    free(x);
}
