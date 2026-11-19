void *calloc(int n, int m); void *memcpy(void *d, void *s, int n);
int **g(void);
void f(void) {
    int **q = g();
    if (!q) return;
    char *c = calloc(3, 1);
    if (!c) return;
    void *t = q;
    memcpy(t, &c, 8);
}
