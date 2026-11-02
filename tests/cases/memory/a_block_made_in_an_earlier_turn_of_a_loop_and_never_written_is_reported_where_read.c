void *malloc(int n);
int f(int k) {
    int **m = 0;
    while (k) {
        if (m) { int *r = *m; if (!r) return 0; return *r; }
        m = malloc(8); if (!m) return 0;
        k--;
    }
    return 0;
}
