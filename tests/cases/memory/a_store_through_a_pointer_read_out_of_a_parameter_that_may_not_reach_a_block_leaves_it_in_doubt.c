void *malloc(int n);
int f(int ***t, int ***s, int *y) {
    if (s == 0 || t == 0) {
        return 0;
    }
    int **m = malloc(8);
    if (m == 0) {
        return 0;
    }
    *s = m;
    int **v = *t;
    if (v == 0) {
        return 0;
    }
    *v = y;
    int *q = *m;
    return q != 0;
}
