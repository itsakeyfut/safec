void *malloc(int n);
int f(void) {
    int *a;
    int ***m = malloc(8);
    if (!m) return 0;
    int **b = &a;
    *m = b;
    int *r = **m;
    return r != 0;
}
