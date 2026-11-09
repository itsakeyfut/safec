void *malloc(int n);
int **f(int * _Nonnull x, int c) {
    int **p = malloc(8);
    if (c) p = 0;
    if (p) *p = x;
    return p;
}
