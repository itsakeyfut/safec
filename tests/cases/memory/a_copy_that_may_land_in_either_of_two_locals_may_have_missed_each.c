void *memcpy(void *d, void *s, int n);
int f(int c) {
    int x = 0; int *m; int *n;
    int **g = &m;
    if (c) g = &n;
    int *px = &x; memcpy(g, &px, 8);
    int *r = m;
    return r == 0;
}
