void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out, char **other, int k) {
    if (!out || !other) return;
    int *c = malloc(4);
    if (!c) return;
    void *t;
    if (k) t = out; else t = other;
    void **vo = t;
    memcpy(vo, &c, 8);
}
