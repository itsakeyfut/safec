void *malloc(int n);
int f(int k, int ***param, int *y) {
    int **m = malloc(8);
    if (!m) return 0;
    int **p = m;
    if (k) p = *param;
    int ***t = &p;
    **t = y;
    int *r = *m;
    return r != 0;
}
