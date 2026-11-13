// The free-then-null idiom: `*b = 0;` replaces the freed `x` in a block of
// one slot made once, so nothing freed is returned (#656).
void *malloc(int n);
void free(void *p);
int **make(void) {
    int **b = malloc(8);
    if (b == 0) { return 0; }
    int *x = malloc(4);
    *b = x;
    free(x);
    *b = 0;
    return b;
}
