void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void aliased(void ***pp) {
    int i = 1;
    if (!pp) return;
    void *m = malloc(16);
    if (!m) return;
    void **q = m;
    *pp = q;
    memcpy(m, &i, 4);
    use();
}
