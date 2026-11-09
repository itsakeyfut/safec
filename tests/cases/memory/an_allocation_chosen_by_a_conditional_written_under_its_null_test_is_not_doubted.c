void *malloc(int n);
int **f(int * _Nonnull x, int c) {
    int **b = c ? malloc(8) : 0;
    if (b) *b = x;
    return b;
}
