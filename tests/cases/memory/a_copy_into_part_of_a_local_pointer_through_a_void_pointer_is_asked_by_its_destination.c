void *memcpy(void *d, void *s, int n); void use(void);
void f(void *in) {
    int x = 1; int *v = &x;
    void *d = &v;
    memcpy(d, in, 4);
    use();
}
