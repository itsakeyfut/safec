void free(void *p);
void *malloc(int n);
void stash(int *p);
int g(void) {
    int *u = malloc(4);
    if (!u) return 0;
    stash(u);
    int *m = malloc(4);
    if (!m) return 0;
    int *k = malloc(4);
    if (!k) return 0;
    int **b = malloc(8);
    if (!b) return 0;
    int ***c = malloc(8);
    if (!c) return 0;
    *c = b;
    *b = k;
    free(m);
    int *q = **c;
    if (!q) return 0;
    return *q;
}
