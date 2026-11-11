void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
int read(void) {
    int i = 1;
    void *x = 0;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &i, 4);
    void **b = m;
    *b = x;
    void *s = *b;
    free(m);
    return s != 0;
}
