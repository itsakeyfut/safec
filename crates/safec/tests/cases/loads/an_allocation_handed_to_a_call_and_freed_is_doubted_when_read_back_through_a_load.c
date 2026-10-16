void free(void *p);
void *malloc(int n);
void stash(int *p);
int f(int ***ppp) {
    if (!ppp) return 0;
    int **t = *ppp;
    if (!t) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    stash(m);
    free(m);
    int *q = *t;
    if (!q) return 0;
    return *q;
}
