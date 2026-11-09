void *malloc(int n);
int **f(int *x, int c, int d) {
    int **b = malloc(8);
    if (!b) return 0;
    if (d) *b = x;
    for (int j = 0; j < 2; j++) { b = 0; }
    for (int i = 0; i < 2; i++) { if (c) return b; }
    return 0;
}
