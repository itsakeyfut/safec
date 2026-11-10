void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void **returned(void) {
    int i = 1;
    void *m = malloc(8);
    if (!m) return 0;
    void **q = m;
    *q = 0;
    memcpy(m, &i, 4);
    return q;
}
