void *malloc(int n);
int **f(int * _Nonnull x, int c) {
    int **p = 0;
    if (c) p = malloc(8);
    if (p) *p = x;
    return p;
}
