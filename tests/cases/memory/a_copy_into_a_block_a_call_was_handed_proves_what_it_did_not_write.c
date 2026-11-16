void *malloc(int n); void *memcpy(void *d, void *s, int n); void keep(int ***p);
int f(void) {
    int x = 0;
    int ***b = malloc(8);
    if (!b) return 0;
    keep(b);
    int **m2 = malloc(8);
    if (!m2) return 0;
    *b = m2;
    int *y = &x;
    int **yy = &y;
    memcpy(b, &yy, 8);
    int *r = *m2;
    return r == 0;
}
