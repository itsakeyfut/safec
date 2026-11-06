void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(void) {
    int *a[1];
    int *b[1];
    memcpy(b, a, 8);
}
