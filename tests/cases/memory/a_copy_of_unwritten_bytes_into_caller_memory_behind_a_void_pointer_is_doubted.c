void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    int *a[1];
    void *d = out; void *s = a;
    memcpy(d, s, 8);
}
