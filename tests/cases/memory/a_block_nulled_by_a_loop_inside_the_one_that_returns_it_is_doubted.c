void *malloc(int n);
int **f(int *x, int c, int d) {
    int **b = malloc(8);
    if (!b) return 0;
    if (d) *b = x;
    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < c; i++) { b = 0; }
        if (d) return b;
    }
    return 0;
}
