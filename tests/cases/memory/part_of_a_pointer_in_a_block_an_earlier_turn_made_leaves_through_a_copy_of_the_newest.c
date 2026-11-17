void *malloc(int n); void *memcpy(void *d, void *s, int n); void *memset(void *d, int c, int n);
void f(void ***out, int n, int * _Nonnull y) {
    if (!out) return;
    void **q = 0;
    void **prev = 0;
    for (int i = 0; i < n; i++) {
        q = malloc(16);
        if (!q) { while (1) {} }
        memset(q, 1, 4);
        q[1] = prev;
        prev = q;
    }
    if (!q) { while (1) {} }
    q[0] = y;
    void **d = malloc(16);
    if (!d) { while (1) {} }
    memcpy(d, q, 16);
    *out = d;
}
