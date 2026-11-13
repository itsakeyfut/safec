// The bytes `*n` holds came from `m` through the copy, so the report names
// the copy and `m`, not `calloc`, which allocated `n` (#752).
void *malloc(int n); void *calloc(int n, int m); void free(void *p); void *memcpy(void *d, void *s, int n);
int f(void) {
    int **m = malloc(8); if (!m) return 0;
    int **n = calloc(1, 8); if (!n) return 0;
    memcpy(n, m, 8);
    free(m);
    int *r = *n;
    return r == 0;
}
