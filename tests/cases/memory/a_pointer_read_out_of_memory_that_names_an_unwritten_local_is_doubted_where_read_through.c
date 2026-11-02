int f(int k, int *x) {
    int *loc;
    int **p = &loc;
    int ***pp = &p;
    int **u = *pp;
    if (!u) return 0;
    int *r = *u;
    return r == x;
}
