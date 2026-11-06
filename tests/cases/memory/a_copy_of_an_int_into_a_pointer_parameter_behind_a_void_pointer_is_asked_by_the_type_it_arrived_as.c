void *memcpy(void *d, void *s, int n); void use(void);
void f(int **out) {
    int i = 1;
    void *d = out; void *s = &i;
    memcpy(d, s, 4);
    use();
}
