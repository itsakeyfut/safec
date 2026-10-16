void free(void *p);
void *malloc(int n);
int f(int **pp, int **w) {
    if (!pp) return 0;
    if (!w) return 0;
    int **b = malloc(8);
    if (!b) return 0;
    *b = *pp;
    int *m = malloc(4);
    if (!m) return 0;
    *b = m;
    free(m);
    int *q = *w;
    if (!q) return 0;
    free(b);
    return *q;
}
