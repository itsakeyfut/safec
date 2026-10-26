void *malloc(int n);

int f(int c) {
    int *m[1];
    int **p = malloc(8);
    if (p == 0) {
        return 1;
    }
    if (c) {
        p = m;
    }
    int *q = *p;
    return q == 0;
}
