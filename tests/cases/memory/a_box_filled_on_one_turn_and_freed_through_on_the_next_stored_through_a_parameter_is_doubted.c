void *malloc(int n);
void free(void *p);
void put(int *** _Nonnull out) {
    int **b = malloc(8);
    if (b == 0) { return; }
    int *x = 0;
    for (int i = 0; i < 2; i++) {
        free(x);
        x = malloc(4);
        if (x == 0) { free(b); return; }
        if (i == 0) { *b = x; }
    }
    *out = b;
}
