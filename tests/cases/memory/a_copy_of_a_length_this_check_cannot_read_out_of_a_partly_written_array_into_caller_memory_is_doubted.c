void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out, int *x, int k) {
    if (!out) return;
    int *a[2];
    a[0] = x;
    memcpy(out, a, k);
}
