void *malloc(int n); void *memcpy(void *d, void *s, int n); void use(void);
void f(void *out, int *p) {
    int **b = malloc(8);
    if (!b) return;
    *b = p;
    void *d = out; void *s = b;
    memcpy(d, s, 4);
    use();
}
