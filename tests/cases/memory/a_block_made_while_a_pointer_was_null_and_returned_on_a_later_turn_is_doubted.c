void *malloc(int n);
int **f(int *x, int c, int d) {
    int **b = 0;
    for (int j = 0; j < 2; j++) {
        if (b) {
            for (int i = 0; i < 2; i++) { if (c) return b; b = 0; }
        } else {
            b = malloc(8);
            if (!b) return 0;
            if (d) *b = x;
        }
    }
    return 0;
}
