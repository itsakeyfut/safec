void *malloc(int n); void *memcpy(void *d, void *s, int n);
void g(int **p);
int f(int **d, int *x) {
    int **w = malloc(8); if (!w) return 0;
    *w = x;
    g(w);
    int **m = malloc(8); if (!m) return 0;
    memcpy(d, m, 8);
    int *r = *w;
    return r == x;
}
