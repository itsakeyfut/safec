void *malloc(int n);
int **f(int c, int *p) {
    if (c) {
        int **b = malloc(8);
        if (!b) return 0;
        if (c == 2) *b = p;
        return b;
    }
    return 0;
}
