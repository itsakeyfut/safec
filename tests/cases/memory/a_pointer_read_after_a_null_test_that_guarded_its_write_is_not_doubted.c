void *malloc(int n);
int f(int *x) {
    if (!x) return 0;
    int **m = malloc(8);
    if (m) *m = x;
    if (!m) return 0;
    int *r = *m;
    if (!r) return 0;
    return *r;
}
