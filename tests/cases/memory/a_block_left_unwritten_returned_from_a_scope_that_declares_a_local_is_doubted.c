void *malloc(int n);
int **f(int c, int *p) {
    int **b = malloc(8);
    if (b) {
        int k = c;
        if (k) *b = p;
        return b;
    }
    return 0;
}
