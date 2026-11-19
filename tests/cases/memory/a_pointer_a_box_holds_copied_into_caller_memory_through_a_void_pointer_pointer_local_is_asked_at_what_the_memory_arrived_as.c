void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    char **box = malloc(8);
    if (!box) return;
    char *c = malloc(3);
    if (!c) return;
    *box = c;
    void *t = out;
    void **vo = t;
    memcpy(vo, box, 8);
}
