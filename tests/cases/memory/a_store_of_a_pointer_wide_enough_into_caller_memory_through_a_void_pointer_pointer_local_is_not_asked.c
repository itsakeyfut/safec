void *malloc(int n);
void f(int **out) {
    if (!out) return;
    int *c = malloc(4);
    if (!c) return;
    void *t = out;
    void **vo = t;
    *vo = c;
}
