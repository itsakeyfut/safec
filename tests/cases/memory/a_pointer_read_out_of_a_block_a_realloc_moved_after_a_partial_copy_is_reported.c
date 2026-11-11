void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int read(void) {
    int i = 1;
    void *x = 0;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &x, 8);
    memcpy(m, &i, 4);
    void *n = realloc(m, 8);
    if (!n) { free(m); return 0; }
    void **b = n;
    void *s = *b;
    free(n);
    return s != 0;
}
