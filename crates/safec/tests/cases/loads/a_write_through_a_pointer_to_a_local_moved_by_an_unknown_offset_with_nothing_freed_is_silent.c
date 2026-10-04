void *malloc(int n);
int f(int k) {
    int *slot = 0;
    int **po = &slot;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    po[k] = a;
    int *q = slot;
    if (q == 0) {
        return 0;
    }
    return *q;
}
