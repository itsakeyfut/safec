// `memcpy(&r, b, 8)` reads the first slot of a block kept per slot, which
// `b[0] = x;` wrote, as `r = b[0];` reads it. Asked of the whole block, of
// whose two slots one is unwritten, `r` was doubted (#928).
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int * _Nonnull x) {
    int **b = malloc(16);
    if (!b) return 0;
    b[0] = x;
    int *r;
    memcpy(&r, b, 8);
    return r == x;
}
