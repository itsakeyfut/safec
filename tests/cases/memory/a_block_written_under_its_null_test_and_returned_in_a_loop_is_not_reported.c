void *malloc(int n);
int **f(int *x, int c) {
    int **b = malloc(8);
    if (b) *b = x;
    for (int i = 0; i < 2; i++) { if (c) return b; }
    return 0;
}
