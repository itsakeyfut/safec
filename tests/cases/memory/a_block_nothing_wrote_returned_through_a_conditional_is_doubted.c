void *malloc(int n);
int **f(int c) {
    int **a = malloc(8);
    int **b = malloc(8);
    if (!a || !b) return 0;
    return c ? a : b;
}
