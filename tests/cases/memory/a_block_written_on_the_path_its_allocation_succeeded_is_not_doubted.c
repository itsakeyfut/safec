void *malloc(int n);
int **f(int *p) {
    int **m = malloc(8);
    if (m) *m = p;
    return m;
}
