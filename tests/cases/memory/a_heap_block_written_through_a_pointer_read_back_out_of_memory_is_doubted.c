// `c` is read back out of `box` and names no element, so `*c = x;` may
// have written either one, and `b[1]` is doubted where it is read (#893).
void *malloc(int n);
int g(int * _Nonnull x) {
    int **b = malloc(16);
    if (!b) return 0;
    int ***box = malloc(8);
    if (!box) return 0;
    *box = b;
    int **c = *box;
    if (!c) return 0;
    *c = x;
    int *r = b[1];
    return r != 0;
}
