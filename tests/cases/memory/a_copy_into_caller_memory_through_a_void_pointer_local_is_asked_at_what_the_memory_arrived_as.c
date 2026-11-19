void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    char *c = malloc(3);
    if (!c) return;
    c[0] = 0; c[1] = 0; c[2] = 0;
    void *v = c;
    void *vo = out;
    memcpy(vo, &v, 8);
}
