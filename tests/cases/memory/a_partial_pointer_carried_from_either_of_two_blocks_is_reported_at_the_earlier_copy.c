void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
void out(void ***pp, int c) {
    int i = 1;
    void *x = 0;
    if (!pp) return;
    void *m1 = malloc(8);
    if (!m1) return;
    void *m2 = malloc(8);
    if (!m2) { free(m1); return; }
    void *m3 = malloc(8);
    if (!m3) { free(m1); free(m2); return; }
    memcpy(m1, &x, 8);
    memcpy(m2, &x, 8);
    memcpy(m1, &i, 4);
    memcpy(m2, &i, 4);
    void *from = m2;
    if (c) from = m1;
    memcpy(m3, from, 8);
    free(m1);
    free(m2);
    void **q = m3;
    *pp = q;
    use();
}
