void *malloc(int n);
int f(int k, int j, int *x) {
    int t = 0;
    int **m = 0;
    while (k) {
        int i = j;
        while (i) { if (m) { int *r = *m; if (r) t += *r; } i--; }
        m = malloc(8); if (!m) return 0;
        *m = x;
        k--;
    }
    return t;
}
