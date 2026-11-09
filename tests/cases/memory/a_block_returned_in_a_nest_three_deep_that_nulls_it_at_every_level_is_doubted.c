void *malloc(int n);
int **f(int *x, int c, int d) {
    int **b = malloc(8);
    if (!b) return 0;
    if (d) *b = x;
    for (int k = 0; k < 2; k++) {
        for (int j = 0; j < 2; j++) {
            for (int i = 0; i < 2; i++) { if (c) return b; b = 0; }
            b = 0;
        }
        b = 0;
    }
    return 0;
}
