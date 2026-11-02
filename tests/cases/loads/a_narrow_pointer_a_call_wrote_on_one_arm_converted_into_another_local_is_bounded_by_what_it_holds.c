void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void geti(int **p);
void getc(char **p);
int f(int k) {
    int *c;
    geti(&c);
    char *t = 0;
    if (k) getc(&t);
    void *v = t;
    c = v;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &c, 8);
    int **ib = m;
    int *r = *ib;
    if (!r) return 0;
    return *r;
}
