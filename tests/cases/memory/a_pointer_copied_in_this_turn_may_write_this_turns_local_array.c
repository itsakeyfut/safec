void *memcpy(void *d, void *s, int n);
int f(int ***b, int ***src, int n) {
    if (b == 0) { return 1; }
    int **t = 0;
    for (int i = 0; i < n; i++) {
        int *m[1];
        *b = m;
        memcpy(&t, src, 8);
        if (t != 0) {
            *t = 0;
            int *q = m[0];
            if (q == 0) { return 1; }
            *q = 1;
        }
    }
    return 0;
}
