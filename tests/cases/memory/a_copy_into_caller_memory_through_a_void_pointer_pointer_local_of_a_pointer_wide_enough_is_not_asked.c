void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    int *c = malloc(4);
    if (!c) return;
    void *t = out;
    void **vo = t;
    memcpy(vo, &c, 8);
}
