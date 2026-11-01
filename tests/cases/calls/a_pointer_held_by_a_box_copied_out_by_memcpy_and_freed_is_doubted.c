void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);

void put(int *** _Nonnull pp) {
    int *x = malloc(4);
    if (!x) return;
    int **box = malloc(8);
    if (!box) return;
    int ***b = malloc(8);
    if (!b) return;
    *box = x;
    *b = box;
    memcpy(pp, b, 8);
    free(b);
    free(x);
}
