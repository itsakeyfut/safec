void *malloc(int n); void *memcpy(void *d, void *s, int n);
int **f(int *p) {
    int **m = malloc(8);
    if (m) *m = p;
    return m;
}
