// The same through `memmove`, which copies as `memcpy` does (C17 7.24.2.2):
// `SC0402` at `*q = 1` (#969, #979).
void *malloc(int n);
void free(void *p);
void *memmove(void *d, void *s, int n);
void f(void) {
    int *p = malloc(4);
    if (!p) return;
    int *q;
    memmove(&q, &p, 8);
    free(p);
    *q = 1;
}
