void free(void *p);
void *malloc(int n);
int f(int ***a, int ***b, int c) {
    if (!a) return 0;
    if (!b) return 0;
    int **t = malloc(8);
    if (!t) return 0;
    *t = 0;
    *a = t;
    int **w = malloc(8);
    if (!w) return 0;
    *w = 0;
    if (c) w = *b;
    if (!w) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *m = 1;
    *w = m;
    free(m);
    int *q = *t;
    if (!q) return 0;
    return *q;
}
