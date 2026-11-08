void *malloc(int n);
int **f(int *x, int c) {
    int **b = malloc(8);
    if (!b) return 0;
    for (int i = 0; i < 2; i++) { if (c) return b; *b = x; }
    return b;
}
