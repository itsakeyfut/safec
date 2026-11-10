void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void ints(int ** _Nonnull pp) {
    int i = 1;
    void *m = malloc(16);
    if (!m) return;
    memcpy(m, &i, 4);
    int *q = m;
    *pp = q;
}
