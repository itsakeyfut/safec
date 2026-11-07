void *memcpy(void *d, void *s, int n);
int f(int **src, int c) {
    if (!src) return 0;
    int *m[2];
    int *n[2];
    int **d = m;
    if (c) d = n;
    memcpy(d, src, 8);
    int *r = m[0];
    return r == 0;
}
