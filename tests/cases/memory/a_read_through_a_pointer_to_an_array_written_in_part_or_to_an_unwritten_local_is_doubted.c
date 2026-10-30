int f(int c) {
    int x = 1;
    int *m[4];
    int *slot;
    int **e = m + 1;
    if (c) {
        e = e + 1;
    }
    *e = &x;
    int **w = &slot;
    if (c) {
        w = m;
    }
    int *r = *w;
    return r == 0;
}
