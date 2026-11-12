// A loop over a block kept per element whose pointer moves through it may
// stop before the end: `b[1]` reaches the caller unwritten here, and a
// store at an element not known writes none for certain (#893).
void *malloc(int n);
int **f(int * _Nonnull x) {
    int **b = malloc(16);
    if (!b) return 0;
    int **e = b;
    for (int k = 0; k < 1; k++) {
        *e = x;
        e = e + 1;
    }
    return b;
}
