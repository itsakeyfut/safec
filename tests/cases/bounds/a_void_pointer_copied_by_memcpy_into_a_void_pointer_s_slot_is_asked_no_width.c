void *malloc(int n); void *memcpy(void *d, void *s, int n); void use(void);
void f(void **out) {
    void *v = malloc(1);
    memcpy(out, &v, 8);
    use();
}
