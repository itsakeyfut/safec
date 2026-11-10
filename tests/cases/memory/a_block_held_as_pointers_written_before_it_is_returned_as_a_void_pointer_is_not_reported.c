void *malloc(int n);
void *returned(int *x) {
    int **b = malloc(8);
    if (!b) return 0;
    *b = x;
    return b;
}
