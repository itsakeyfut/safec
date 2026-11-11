void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
int read(int c) {
    int i = 1;
    void *x = 0;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &x, 8);
    if (c) {
        memcpy(m, &i, 4);
    } else {
        c = 2;
    }
    void **b = m;
    void *s = *b;
    free(m);
    return s != 0;
}
