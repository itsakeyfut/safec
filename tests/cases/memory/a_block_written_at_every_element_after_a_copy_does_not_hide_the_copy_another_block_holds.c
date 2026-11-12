// `b` was written at every element after a copy carried bytes into it, so
// only `c` may be unwritten where `*p` reads, and the report names the copy
// into `c` (#752).
void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int k, int * _Nonnull x) {
    int **a = malloc(16); if (!a) return 0;
    int **a2 = malloc(8); if (!a2) return 0;
    int **b = malloc(16); if (!b) return 0;
    memcpy(b, a, 16);
    b[0] = x;
    b[1] = x;
    int **c = malloc(8); if (!c) return 0;
    memcpy(c, a2, 8);
    int **p = b;
    if (k) p = c;
    int *r = *p;
    return r == 0;
}
