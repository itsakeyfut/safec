void g(void);
int f(int *p, char *c, void *v, int n) {
    int r;
    r = p == 1;
    r = p < n;
    r = p < 0;
    r = p == c;
    r = p < c;
    r = g < g;
    r = v == g;
    r = p == 1 - 1;
    return r;
}
