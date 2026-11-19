void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out, char **in) {
    if (!out || !in) return;
    void *t = out;
    void **vo = t;
    memcpy(vo, in, 8);
}
