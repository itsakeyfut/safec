void free(void *p);
void *malloc(int n);
int f(int ***ppp) {
    if (!ppp) return 0;
    int **t = *ppp;
    if (!t) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *t = m;
    free(m);
    int **u = *ppp;
    int *q = *u;
    if (!q) return 0;
    return *q;
}
