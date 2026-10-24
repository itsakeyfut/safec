void free(void *p);
void *malloc(int n);
int ***get(void);
int f(void) {
    int ***r = get();
    if (!r) return 0;
    int **t = *r;
    if (!t) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *t = m;
    free(m);
    int **u = *r;
    int *q = *u;
    if (!q) return 0;
    return *q;
}
