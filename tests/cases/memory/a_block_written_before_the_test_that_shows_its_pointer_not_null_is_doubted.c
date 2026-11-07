void *malloc(int n);
int **f(int *p) {
    int **b = malloc(8);
    if (b) *b = p;
    if (!b) return 0;
    return b;
}
