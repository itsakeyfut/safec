void *malloc(int n);
int **f(void) {
    int **b = malloc(8);
    if (!b) return 0;
    return b;
}
