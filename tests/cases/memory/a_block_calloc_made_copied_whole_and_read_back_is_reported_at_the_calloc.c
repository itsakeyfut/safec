void *memcpy(void *d, void *s, int n);
void *calloc(int n, int size);
void *malloc(int n);
void free(void *p);
int b(void) {
    void **q = calloc(1, 8);
    if (!q) return 0;
    void **n = malloc(8);
    if (!n) { free(q); return 0; }
    memcpy(n, q, 8);
    void *r = *n;
    free(q);
    free(n);
    return r == 0;
}
