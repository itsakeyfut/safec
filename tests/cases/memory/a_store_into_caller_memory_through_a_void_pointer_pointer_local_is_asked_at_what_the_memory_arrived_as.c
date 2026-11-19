void *malloc(int n);
void f(int **out) {
    if (!out) return;
    char *c = malloc(3);
    if (!c) return;
    void *t = out;
    void **vo = t;
    *vo = c;
}
