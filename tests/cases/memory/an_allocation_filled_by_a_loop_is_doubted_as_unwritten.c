void *malloc(int n);
int f(int n, int *x) {
    int **m = malloc(8 * 4);
    if (!m) return 0;
    for (int i = 0; i < 4; i = i + 1) m[i] = x;
    int *r = m[0];
    if (!r) return 0;
    return *r;
}
