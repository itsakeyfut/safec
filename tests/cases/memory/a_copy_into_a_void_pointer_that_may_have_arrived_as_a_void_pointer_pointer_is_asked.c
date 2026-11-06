void *memcpy(void *d, void *s, int n); void use(void);
void f(int c, void **pp, int *ip) {
    int i = 1;
    void *d = ip;
    if (c) d = pp;
    void *s = &i;
    memcpy(d, s, 4);
    use();
}
