void *malloc(int n);
int f(int k) {
    int **m = 0;
    if (!k) { } else { m = malloc(8); if (!m) return 0; }
    if (m) { int *r = *m; if (!r) return 0; return *r; }
    return 0;
}
