void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void use(void);
int read(void) {
    int i = 1;
    void *x = 0;
    char buf[8];
    memcpy(buf, &x, 8);
    memcpy(buf, &i, 4);
    void *v = buf;
    void **p = v;
    void *s = *p;
    return s != 0;
}
