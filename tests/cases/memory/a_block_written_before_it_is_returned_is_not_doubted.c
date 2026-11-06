void *malloc(int n);
int **f(int *p) {
    int **b = malloc(8);
    if (!b) return 0;
    *b = p;
    return b;
}
