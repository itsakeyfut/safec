void *malloc(int n);
int **f(int * _Nonnull x, int c) {
    int **b = malloc(8);
    if (c && b) *b = x;
    return b;
}
