void *malloc(int n);
int **f(int c, int *p) {
    int **b = malloc(8);
    if (!b) return 0;
    if (c) *b = p;
    return b;
}
