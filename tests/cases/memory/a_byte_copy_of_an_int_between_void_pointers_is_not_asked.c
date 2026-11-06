void *malloc(int n); void *memcpy(void *d, void *s, int n); void use(void);
void f(void) {
    int *m = malloc(4);
    if (!m) return;
    int i = 1;
    void *d = m; void *s = &i;
    memcpy(d, s, 4);
    use();
}
