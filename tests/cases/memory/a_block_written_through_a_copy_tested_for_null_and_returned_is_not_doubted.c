void *malloc(int n);
int **f(int * _Nonnull x) {
    int **b = malloc(8);
    int **c = b;
    if (c) *c = x;
    return b;
}
