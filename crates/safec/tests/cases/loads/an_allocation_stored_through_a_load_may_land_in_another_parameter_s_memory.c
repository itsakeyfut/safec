void free(void *p);
void *malloc(int n);
int f(int **pp, int ***ppp) {
    if (!pp) return 0;
    if (!ppp) return 0;
    int **t = *ppp;
    if (!t) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *m = 1;
    *t = m;
    free(m);
    int *q = *pp;
    if (!q) return 0;
    return *q;
}
