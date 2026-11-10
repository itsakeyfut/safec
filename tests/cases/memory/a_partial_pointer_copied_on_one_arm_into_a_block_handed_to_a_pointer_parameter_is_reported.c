void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void g(void **q);
void arm(void **p) {
    int i = 1;
    void *m = malloc(16);
    if (!m) return;
    if (p) memcpy(m, &i, 4);
    g(m);
}
