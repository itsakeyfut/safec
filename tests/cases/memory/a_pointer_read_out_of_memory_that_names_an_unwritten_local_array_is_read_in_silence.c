int f(int *x) {
    int *a[1];
    int **p = a;
    int ***pp = &p;
    int **u = *pp;
    if (!u) return 0;
    int *r = *u;
    return r == x;
}
