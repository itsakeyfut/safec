void *malloc(int n);
int f(int c) {
    int x;
    int **m = malloc(8);
    int **n = malloc(8);
    if (!m || !n) return 0;
    *n = &x;
    int **p = c ? m : n;
    *p = &x;
    int *r = *m;
    return r != 0;
}
