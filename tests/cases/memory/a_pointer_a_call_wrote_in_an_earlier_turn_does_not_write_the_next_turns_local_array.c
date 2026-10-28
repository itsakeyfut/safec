void get2(int ***out, int **m);
int f(int n) {
    int **t = 0;
    int r = 0;
    for (int i = 0; i < n; i++) {
        int *m[1];
        if (i == 1) {
            if (t == 0) { return 1; }
            *t = 0;
            int *q = m[0];
            if (q != 0) { r = *q; }
        }
        m[0] = 0;
        t = 0;
        get2(&t, m);
    }
    return r;
}
