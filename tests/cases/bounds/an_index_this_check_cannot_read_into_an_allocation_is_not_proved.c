void *malloc(int n);
int f(int k) {
    int *m = malloc(8);
    if (!m) return 0;
    m[k] = 3;
    return 0;
}
