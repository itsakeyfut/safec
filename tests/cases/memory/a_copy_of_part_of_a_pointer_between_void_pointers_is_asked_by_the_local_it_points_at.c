void *memcpy(void *d, void *s, int n); void use(void);
void f(void *out) {
    int x = 1; int *v = &x;
    void *d = out; void *s = &v;
    memcpy(d, s, 4);
    use();
}
