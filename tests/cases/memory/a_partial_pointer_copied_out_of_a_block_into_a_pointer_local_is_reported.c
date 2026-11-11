void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
void out(void **pp) {
    int i = 1;
    void *x = 0;
    if (!pp) return;
    void *m = malloc(8);
    if (!m) return;
    void *s;
    memcpy(m, &x, 8);
    memcpy(m, &i, 4);
    memcpy(&s, m, 8);
    free(m);
    *pp = s;
}
