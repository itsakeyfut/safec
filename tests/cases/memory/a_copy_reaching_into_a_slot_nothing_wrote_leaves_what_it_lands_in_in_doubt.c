// Twelve bytes of a block of two slots, of which only the first is
// written, take four of the second: what lands in `c` may be
// indeterminate, and `r`, read across both, is doubted. The slots a copy
// reads run to the one its last byte is in (#928).
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int * _Nonnull x) {
    int **b = malloc(16);
    if (!b) return 0;
    b[0] = x;
    char c[12];
    memcpy(c, b, 12);
    int *r; memcpy(&r, c + 4, 8); return r == x;
}
