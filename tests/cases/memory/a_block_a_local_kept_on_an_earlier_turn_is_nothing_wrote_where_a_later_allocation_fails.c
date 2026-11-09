void *malloc(int n);
int **f(int c, int n) {
    int **q = 0;
    for (int i = 0; i < n; i++) {
        int **p = malloc(8);
        if (c) q = p;
        if (!p) return q;
    }
    return 0;
}
