void *malloc(int n); void *memcpy(void *d, void *s, int n); void use(void);
void f(int **out) {
    void *v = malloc(4);
    memcpy(out, &v, 4);
    use();
}
