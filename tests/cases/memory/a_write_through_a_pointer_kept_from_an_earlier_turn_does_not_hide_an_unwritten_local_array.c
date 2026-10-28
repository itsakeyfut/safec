int f(int n, int ***b, int c) {
    if (b == 0) { return 1; }
    int *y[1];
    int **p = y;
    for (int i = 0; i < n; i++) {
        int *x[1];
        int *m[1];
        *b = m;
        *p = 0;
        int *q = m[0];
        if (q != 0) { *q = 1; }
        if (c) { p = x; }
    }
    return 0;
}
