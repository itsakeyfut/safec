void *malloc(int n);
int **f(int n, int *p) {
    int **b = malloc(8);
    if (!b) return 0;
    while (n) { *b = p; n = n - 1; }
    return b;
}
