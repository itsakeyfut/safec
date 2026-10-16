void free(void *p);
void *malloc(int n);
int f(int **pp, int **w) {
    if (!pp) return 0;
    if (!w) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *m = 1;
    *w = m;
    free(m);
    int *q = *pp;
    if (!q) return 0;
    return *q;
}
