// The same block, copied from its second slot, which nothing wrote: what
// lands in `r` is certainly indeterminate, whatever the first slot holds
// (#928).
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int * _Nonnull x) {
    int **b = malloc(16);
    if (!b) return 0;
    b[0] = x;
    int **s = b + 1;
    int *r;
    memcpy(&r, s, 8);
    return r == x;
}
