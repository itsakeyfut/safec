void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    int *a[1];
    memcpy(out, a, 8);
}
