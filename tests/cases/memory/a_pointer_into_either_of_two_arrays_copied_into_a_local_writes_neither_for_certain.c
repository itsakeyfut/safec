void *memcpy(void *d, void *s, int n);
int f(int c) {
    int x = 0;
    int *m[2];
    int *n[2];
    int **e = m + 1 - 1;
    if (c) e = n + 1 - 1;
    int **g = 0;
    memcpy(&g, &e, 8);
    *g = &x;
    int *r = m[0];
    return r == 0;
}
