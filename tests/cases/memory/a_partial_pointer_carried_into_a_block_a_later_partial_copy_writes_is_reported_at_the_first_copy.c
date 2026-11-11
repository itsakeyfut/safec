void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
void out(void ***pp) {
    int i = 1;
    void *x = 0;
    if (!pp) return;
    void *m = malloc(8);
    if (!m) return;
    void *m2 = malloc(8);
    if (!m2) { free(m); return; }
    memcpy(m, &x, 8);
    memcpy(m, &i, 4);
    memcpy(m2, m, 8);
    memcpy(m2, &i, 4);
    free(m);
    void **q = m2;
    *pp = q;
    use();
}
