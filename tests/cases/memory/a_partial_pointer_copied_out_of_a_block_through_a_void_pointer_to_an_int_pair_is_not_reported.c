void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
int read(void) {
    int i = 1;
    void *x = 0;
    int k[2];
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &x, 8);
    memcpy(m, &i, 4);
    void *d = k;
    memcpy(d, m, 8);
    free(m);
    return k[0];
}
