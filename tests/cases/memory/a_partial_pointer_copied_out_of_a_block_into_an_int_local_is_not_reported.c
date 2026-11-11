void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
int read(void) {
    int i = 1;
    int k = 0;
    void *x = 0;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &x, 8);
    memcpy(m, &i, 4);
    memcpy(&k, m, 4);
    free(m);
    return k;
}
