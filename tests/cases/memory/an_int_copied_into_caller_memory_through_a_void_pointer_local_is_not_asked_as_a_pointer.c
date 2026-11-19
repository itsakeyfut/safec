void *memcpy(void *d, void *s, int n);
void keep(int *p);
void f(int **out) {
    if (!out) return;
    int i = 1;
    keep(&i);
    void *vo = out;
    memcpy(vo, &i, 4);
}
