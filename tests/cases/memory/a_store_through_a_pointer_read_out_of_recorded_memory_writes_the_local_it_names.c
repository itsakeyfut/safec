void *malloc(int n);
int f(int x) {
    int *m[1];
    int ***h = malloc(8);
    if (!h) return 0;
    *h = m;
    int **t = *h;
    *t = &x;
    int *q = m[0];
    return q == 0;
}
