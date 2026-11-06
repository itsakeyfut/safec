void *malloc(int n); void *memcpy(void *d, void *s, int n); void use(void);
void f(int **out) {
    int x = 1;
    int **b = malloc(8);
    if (!b) return;
    *b = &x;
    void *d = out; void *s = b;
    memcpy(d, s, 4);
    use();
}
