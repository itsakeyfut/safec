void *memcpy(void *d, void *s, int n); void use(void);
void f(void *out, int c) {
    int x = 1; int *v = &x; void *w = &x;
    void *s = &v;
    if (c) s = &w;
    void *d = out;
    memcpy(d, s, 4);
    use();
}
