void *memcpy(void *d, void *s, int n); void use(void);
void f(int **out) {
    int x = 1; int *v = &x;
    void *vv = &v; char *s = vv;
    void *d = out;
    memcpy(d, s, 4);
    use();
}
