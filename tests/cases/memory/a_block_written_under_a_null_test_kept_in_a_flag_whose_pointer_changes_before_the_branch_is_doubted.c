void *malloc(int n);
int **f(int * _Nonnull x) {
    int **m = malloc(8);
    int **b = malloc(8);
    int ok = b != 0;
    b = m;
    if (ok) *b = x;
    return b;
}
