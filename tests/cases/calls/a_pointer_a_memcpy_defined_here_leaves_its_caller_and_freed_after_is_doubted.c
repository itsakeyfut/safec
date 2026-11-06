void *malloc(int n);
void free(void *p);
int **memcpy(int ** _Nonnull d, int ** _Nonnull s, int n) {
    *d = *s;
    return d;
}

void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    int **b = malloc(8);
    if (!x || !b) return;
    *b = x;
    memcpy(pp, b, 8);
    free(b);
    free(x);
}
