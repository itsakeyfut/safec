void g(void);
int f(int *p, int *q, char *c, void *v, int n) {
    int a[2];
    int *s;
    int r;
    r = n * 2 / 3 % 4 + 5 - 6 << 1 >> 1;
    r = (n & 3) ^ (n | c[0]);
    r = n < 1 && n > 1 || n <= 1 && n >= 1 && n == 1 && n != 1;
    r = p == q && p != q && p < q && p > q && p <= q && p >= q;
    r = p == 0 && 0 == p && p != 0 && p == v && v != p && v < v;
    r = c < c && g == g && a == p && p && q && (p || n);
    s = p + 1;
    s = 1 + p;
    s = p - 1;
    s = a + 1;
    return r;
}
