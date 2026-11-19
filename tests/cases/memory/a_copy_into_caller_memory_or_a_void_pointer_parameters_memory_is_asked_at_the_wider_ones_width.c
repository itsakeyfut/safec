void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out, void **other, int k) {
    if (!out || !other) return;
    char *c = malloc(3);
    if (!c) return;
    void *t;
    if (k) t = out; else t = other;
    void **vo = t;
    memcpy(vo, &c, 8);
}
