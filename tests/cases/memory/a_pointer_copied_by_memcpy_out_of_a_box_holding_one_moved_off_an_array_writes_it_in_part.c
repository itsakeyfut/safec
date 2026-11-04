void *memcpy(void *d, void *s, int n);
int f(void) {
    int x = 0;
    int *m[2];
    int **e = m + 1;
    int **box[1];
    box[0] = e;
    int **g = 0;
    memcpy(&g, box, 8);
    if (!g) return 0;
    g[0] = &x;
    int *r = m[0];
    return r != 0;
}
