// The free-then-null idiom written with a copy: `memcpy(b, &z, 8)` replaces
// the freed `x` in a block of one slot made once, as `*b = 0;` does (#927).
void *malloc(int n); void free(void *p); void *memcpy(void *d, void *s, int n);
int **make(void) {
    int **b = malloc(8);
    if (b == 0) { return 0; }
    int *x = malloc(4);
    *b = x;
    free(x);
    int *z = 0;
    memcpy(b, &z, 8);
    return b;
}
