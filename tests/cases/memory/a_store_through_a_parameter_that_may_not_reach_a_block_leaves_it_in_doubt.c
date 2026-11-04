void *malloc(int n);

int f(int ***t, int ***s, int *y) {
    int **m = malloc(8);
    if (m == 0) {
        return 0;
    }
    *s = m;
    **t = y;
    int *q = *m;
    return q != 0;
}
