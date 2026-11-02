void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int c) {
    int **m = malloc(8); if (!m) return 0;
    int **a = malloc(8); if (!a) return 0;
    int **b = malloc(8); if (!b) return 0;
    int **p = c ? a : b;
    memcpy(p, m, 8);
    int *r = *a; if (!r) return 0;
    return *r;
}
