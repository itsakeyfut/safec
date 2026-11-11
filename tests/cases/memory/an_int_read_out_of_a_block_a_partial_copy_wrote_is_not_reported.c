void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
int read(void) {
    int i = 1;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &i, 4);
    int *b = m;
    int r = *b;
    char *c = m;
    r += *c;
    free(m);
    return r;
}
