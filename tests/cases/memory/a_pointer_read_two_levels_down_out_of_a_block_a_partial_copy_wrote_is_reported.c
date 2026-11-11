void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
int read(void) {
    int i = 1;
    void *x = 0;
    void *m2 = malloc(8);
    if (!m2) return 0;
    void *m1 = malloc(8);
    if (!m1) { free(m2); return 0; }
    memcpy(m2, &x, 8);
    memcpy(m2, &i, 4);
    void ***b = m1;
    *b = m2;
    void *s = **b;
    free(m1);
    free(m2);
    return s != 0;
}
