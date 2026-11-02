void *malloc(int n);
int f(int *x) {
    int *loc;
    int ***t = malloc(8); if (!t) return 0;
    *t = &loc;
    int **u = *t;
    if (!u) return 0;
    int *r = *u;
    return r == x;
}
