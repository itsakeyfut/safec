void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    int x = 1;
    int *a[2];
    a[0] = &x;
    memcpy(out, a, 8);
}
