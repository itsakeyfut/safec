void *malloc(int n);
int f(int k, int *x) {
    int **m;
    if (k) { m = malloc(8); if (!m) return 0; }
    else   { m = malloc(8); if (!m) return 0; *m = x; }
    int *r = *m; if (!r) return 0;
    return *r;
}
