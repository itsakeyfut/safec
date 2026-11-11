void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
int read(void) {
    int i = 1;
    void *x = 0;
    void *m = malloc(8);
    if (!m) return 0;
    void **b = m;
    memcpy(m, &x, 8);
    memcpy(m, &i, 4);
    void *s = *b;
    int r = s != 0;
    free(m);
    return r;
}
