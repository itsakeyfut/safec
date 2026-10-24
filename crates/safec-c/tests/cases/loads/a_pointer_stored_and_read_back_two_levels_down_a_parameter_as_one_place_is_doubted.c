void free(void *p);
void *malloc(int n);
int f(int ***ppp) {
    if (!ppp) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    **ppp = m;
    free(m);
    int *q = **ppp;
    if (!q) return 0;
    return *q;
}
