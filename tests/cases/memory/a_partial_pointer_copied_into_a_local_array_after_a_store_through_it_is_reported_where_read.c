void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
int read(void) {
    int i = 1;
    void *x = &i;
    char buf[8];
    void *v = buf;
    void **p = v;
    *p = x;
    memcpy(buf, &i, 4);
    void *s = *p;
    return s != 0;
}
