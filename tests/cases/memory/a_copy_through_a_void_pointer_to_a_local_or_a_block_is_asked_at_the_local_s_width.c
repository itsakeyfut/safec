void *malloc(int n); void *memcpy(void *d, void *s, int n); void use(void);
void f(void *out, int c) {
    int x = 1; int *v = &x;
    int **b = malloc(8);
    if (!b) return;
    *b = &x;
    void *s = &v;
    if (c) s = b;
    void *d = out;
    memcpy(d, s, 4);
    use();
}
