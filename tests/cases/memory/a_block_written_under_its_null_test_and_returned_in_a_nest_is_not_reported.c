void *malloc(int n);
int **f(int *x, int c) {
    int **b = malloc(8);
    if (b) *b = x;
    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < 2; i++) { if (c) return b; b = 0; }
        b = 0;
    }
    return 0;
}
