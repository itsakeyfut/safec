void free(void *p);
void *malloc(int n);
int f(int ****pa, int ****pb) {
    if (!pa) return 0;
    if (!pb) return 0;
    int ***a = *pa;
    if (!a) return 0;
    int ***b = *pb;
    if (!b) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *m = 1;
    int *x = 0;
    *a = &x;
    int **t = *b;
    if (!t) return 0;
    *t = m;
    free(m);
    int *q = x;
    if (!q) return 0;
    return *q;
}
