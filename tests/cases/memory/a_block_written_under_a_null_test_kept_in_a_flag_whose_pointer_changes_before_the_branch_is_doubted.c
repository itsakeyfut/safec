void *malloc(int n);
int **f(int * _Nonnull x) {
    int **b = malloc(8);
    int ok = b != 0;
    b = malloc(8);
    if (ok) *b = x;
    return b;
}
