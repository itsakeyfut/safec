void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void out(void ***pp) {
    int i = 1;
    int k;
    void *x = 0;
    if (!pp) return;
    memcpy(&k, &i, 4);
    void *m = malloc(8);
    if (!m) return;
    memcpy(m, &x, 8);
    memcpy(m, &k, 4);
    void **q = m;
    *pp = q;
}
