void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void copied(void * _Nonnull out) {
    void **p = malloc(8);
    if (!p) return;
    *p = p;
    void *v = &p;
    memcpy(out, v, 8);
}
