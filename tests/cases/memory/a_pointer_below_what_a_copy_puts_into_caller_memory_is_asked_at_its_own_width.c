void *memcpy(void *d, void *s, int n);
void f(int ***out, int *y) {
    if (!out) return;
    int *x = y;
    int **c = &x;
    void *t = out;
    void **vo = t;
    memcpy(vo, &c, 8);
}
