void *malloc(int n);
int **f(int c, int *p) {
    int **b = malloc(8);
    if (b) {
        if (c) *b = p;
    }
    return b;
}
