void *memcpy(void *d, void *s, int n); void use(void);
void f(int *p) {
    int i = 1;
    void *d = p; void *s = &i;
    memcpy(d, s, 4);
    use();
}
