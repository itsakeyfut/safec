void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
void use(void);
void out(void ***pp) {
    int i = 1;
    void *x = 0;
    if (!pp) return;
    void *m = malloc(8);
    if (!m) return;
    memcpy(m, &x, 8);
    memcpy(m, &i, 4);
    void *n = realloc(m, 8);
    if (!n) { free(m); return; }
    void **q = n;
    *pp = q;
    use();
}
