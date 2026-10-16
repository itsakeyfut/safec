void free(void *p);
void *malloc(int n);
int f(int ***ppp, int ****rrrr) {
    if (!ppp) return 0;
    if (!rrrr) return 0;
    int *x = 0;
    *ppp = &x;
    int *m = malloc(4);
    if (!m) return 0;
    int **t = **rrrr;
    if (!t) return 0;
    *t = m;
    free(m);
    int *q = x;
    if (!q) return 0;
    return *q;
}
