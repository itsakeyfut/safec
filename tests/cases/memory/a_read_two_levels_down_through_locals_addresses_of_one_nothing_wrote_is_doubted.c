int f(int k) {
    int *x;
    int **q = &x;
    int ***m = &q;
    int *r = **m;
    if (!r) return 0;
    *r = 1;
    return 0;
}
