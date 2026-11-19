void *malloc(int n);
void f(int **out, char **in) {
    if (!out || !in) return;
    void *t = out;
    void **vo = t;
    *vo = *in;
}
