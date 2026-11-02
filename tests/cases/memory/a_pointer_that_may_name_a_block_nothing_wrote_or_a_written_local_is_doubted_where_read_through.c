void *malloc(int n); void free(void *p);
int f(int c, int *x) {
    int *w = x;
    int **a = malloc(8);
    if (a == 0) return 1;
    int **p = a;
    if (c) { p = &w; }
    int *t = *p;
    free(a);
    if (t == 0) return 1;
    return 0;
}
