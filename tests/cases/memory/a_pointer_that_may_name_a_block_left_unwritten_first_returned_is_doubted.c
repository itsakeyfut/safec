void *malloc(int n);
int **f(int c, int d, int *p) {
    int **a = malloc(8);
    int **b = malloc(8);
    if (!a || !b) return 0;
    if (d) *a = p;
    *b = p;
    int **r = b;
    if (c) r = a;
    return r;
}
