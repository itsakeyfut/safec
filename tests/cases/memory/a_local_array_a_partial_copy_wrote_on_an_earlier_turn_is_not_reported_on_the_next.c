void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
void out(void ***pp, int n) {
    int i = 1;
    void *x = 0;
    if (!pp) return;
    void *m2 = malloc(8);
    if (!m2) return;
    memcpy(m2, &x, 8);
    for (int k = 0; k < n; k++) {
        char buf[8];
        memcpy(buf, &x, 8);
        memcpy(m2, buf, 8);
        memcpy(buf, &i, 4);
    }
    void **q = m2;
    *pp = q;
    use();
}
