void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void use(void);
void slot(void ***pp) {
    int i = 1;
    if (!pp) return;
    void *m = malloc(16);
    if (!m) return;
    *pp = m;
    memcpy(m, &i, 4);
    use();
}
