void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int read(int n) {
    int r = 0;
    int i = 1;
    void *x = 0;
    for (int k = 0; k < n; k++) {
        void *m = malloc(8);
        if (!m) return r;
        memcpy(m, &x, 8);
        void **b = m;
        void *s = *b;
        r += s != 0;
        memcpy(m, &i, 4);
        free(m);
    }
    return r;
}
