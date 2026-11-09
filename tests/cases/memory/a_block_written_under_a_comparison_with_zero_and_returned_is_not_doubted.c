void *malloc(int n);
int **f(int * _Nonnull x) {
    int **m = malloc(8);
    if (m != 0) *m = x;
    return m;
}
