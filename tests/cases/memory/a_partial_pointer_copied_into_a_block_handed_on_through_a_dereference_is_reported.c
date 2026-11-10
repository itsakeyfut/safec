void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void g(void **q);
void loaded(void) {
    int i = 1;
    void *m = malloc(16);
    if (!m) return;
    memcpy(m, &i, 4);
    void ***box = malloc(8);
    if (!box) return;
    *box = m;
    g(*box);
}
