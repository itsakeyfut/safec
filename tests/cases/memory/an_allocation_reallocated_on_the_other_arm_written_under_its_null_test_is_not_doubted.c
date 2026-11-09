void *malloc(int n);
void *realloc(void *p, int n);
int **f(int c, int * _Nonnull x) {
    int **q = malloc(8);
    if (!q) return 0;
    int **p = 0;
    if (!c) {} else p = realloc(q, 8);
    if (p) *p = x;
    return p;
}
