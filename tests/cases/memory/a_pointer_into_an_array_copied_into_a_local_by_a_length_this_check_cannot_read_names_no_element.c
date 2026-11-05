void *memcpy(void *d, void *s, int n);
int f(int n) {
    int x = 0;
    int *m[2];
    int **e = m + 1;
    int **g = 0;
    memcpy(&g, &e, n);
    *g = &x;
    int *r = m[1];
    return r == 0;
}
