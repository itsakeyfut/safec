// The copy writes `b[0]` alone, so `b[1]` is read before anything wrote it,
// as the store spelling `b[0] = x;` says (#925).
void *memcpy(void *d, void *s, int n); void *malloc(int n); void free(void *p);
int f(int *x) {
    int **b = malloc(16);
    if (!b) return 0;
    memcpy(b, &x, 8);
    int *r = b[1];
    free(b);
    return r == x;
}
