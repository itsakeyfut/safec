void *malloc(int n);
int **f(int c, int *p) {
    int **a = malloc(8);
    if (!a) return 0;
    *a = p;
    int **b = malloc(8);
    if (!b) return 0;
    int **r = a;
    if (c) r = b;
    return r;
}
