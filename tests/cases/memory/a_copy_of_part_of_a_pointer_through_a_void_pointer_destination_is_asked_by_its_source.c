void *malloc(int n); void *memcpy(void *d, void *s, int n); void use(void);
void f(int **out) {
    int *v = malloc(4);
    void *d = out;
    memcpy(d, &v, 4);
    use();
}
