void free(void *p);
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int ***ppp) {
    if (!ppp) return 0;
    int **t = *ppp;
    if (!t) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *t = m;
    free(m);
    int **d = malloc(8);
    if (!d) return 0;
    memcpy(d, t, 8);
    int *q = *d;
    if (!q) return 0;
    return *q;
}
