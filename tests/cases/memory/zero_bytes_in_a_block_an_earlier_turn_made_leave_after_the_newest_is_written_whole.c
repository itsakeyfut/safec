void *malloc(int n);
void *memset(void *s, int c, int n);
void f(void ***out, int n, int * _Nonnull y) {
    if (!out) return;
    void **q = 0;
    void **prev = 0;
    for (int i = 0; i < n; i++) {
        q = malloc(16);
        if (!q) { while (1) {} }
        memset(q, 0, 8);
        q[1] = prev;
        prev = q;
    }
    if (!q) { while (1) {} }
    q[0] = y;
    *out = q;
}
