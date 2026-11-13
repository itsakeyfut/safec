void *malloc(int n); void *realloc(void *p, int n);
int f(int k, int *x) {
    int **q = malloc(8); if (!q) return 0;
    *q = x;
    int **h = q;
    int **n = realloc(h, 16); if (!n) return 0;
    int *r = *n;
    return r == x;
}
