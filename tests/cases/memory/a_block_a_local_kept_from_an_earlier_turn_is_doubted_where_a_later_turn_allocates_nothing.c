void *malloc(int n);
int **f(int c, int n) {
    int **d = 0;
    for (int i = 0; i < n; i++) {
        int **p = 0;
        if (c) p = malloc(8);
        if (!p) return d;
        d = p;
    }
    return 0;
}
