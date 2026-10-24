void free(void *p);
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int ***ppp, int ***r) {
    if (!ppp) return 0;
    if (!r) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *m = 1;
    int *x = 0;
    int **px = &x;
    memcpy(ppp, &px, 8);
    int **t = *r;
    if (!t) return 0;
    *t = m;
    free(m);
    int *q = x;
    if (!q) return 0;
    return *q;
}
