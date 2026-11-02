void *memcpy(void *d, void *s, int n); void use(void);
void f(int **out, int *v) {
    memcpy(out, &v, 8);
    use();
}
