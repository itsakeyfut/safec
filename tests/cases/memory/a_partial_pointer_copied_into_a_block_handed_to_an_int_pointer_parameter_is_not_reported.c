void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void g(int *q);
void ints(void) {
    int i = 1;
    void *m = malloc(16);
    if (!m) return;
    void **q = m;
    memcpy(m, &i, 4);
    g(m);
}
