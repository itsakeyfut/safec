int f(int c) {
    int x = 0;
    int *m;
    int *n;
    int **g = &m;
    if (c) g = &n;
    *g = &x;
    int *r = m;
    return r == 0;
}
