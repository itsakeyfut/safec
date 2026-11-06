void *memcpy(void *d, void *s, int n); void *memset(void *d, int c, int n);
void f(int *d) {
    int k = 1;
    if (!d) return;
    memcpy(d, &k, 4);
}
