void *malloc(int n);
int **f(int * _Nonnull x) {
    int **b = malloc(8);
    int ok = b != 0;
    int k = ok;
    if (k) *b = x;
    return b;
}
