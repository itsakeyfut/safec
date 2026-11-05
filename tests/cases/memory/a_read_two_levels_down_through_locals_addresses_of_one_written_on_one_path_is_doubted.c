int f(int c) {
    int y = 0;
    int *x;
    if (c) x = &y;
    int **q = &x;
    int ***m = &q;
    int *r = **m;
    if (!r) return 0;
    *r = 1;
    return 0;
}
