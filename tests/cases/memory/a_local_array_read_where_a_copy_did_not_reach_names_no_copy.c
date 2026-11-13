// The copy fills `loc[0]` alone; `loc[1]` is unwritten because nothing
// wrote the array since it began, so the report names no copy (#752).
void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int k, int * _Nonnull x) {
    int **a = malloc(8); if (!a) return 0;
    if (k) *a = x;
    int *loc[2];
    memcpy(loc, a, 8);
    int *r = loc[1];
    return r == 0;
}
