void *memcpy(void *d, void *s, int n);
void *malloc(int n);
void free(void *p);
void out(void **o) {
    int i = 1;
    if (!o) return;
    void *m = malloc(8);
    if (!m) return;
    memcpy(m, &i, 4);
    memcpy(o, m, 8);
    free(m);
}
