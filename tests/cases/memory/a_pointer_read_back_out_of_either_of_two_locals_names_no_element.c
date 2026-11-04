int f(int c) {
    int x = 0;
    int *m[2];
    int **e = m + 1 - 1;
    int **e2 = m + 1;
    int ***pe = &e;
    if (c) pe = &e2;
    int **g = *pe;
    *g = &x;
    int *r = m[0];
    return r == 0;
}
