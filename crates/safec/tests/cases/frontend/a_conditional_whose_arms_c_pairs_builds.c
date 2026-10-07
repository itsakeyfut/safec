void h(void);

int f(int c, int n, char ch, int *p, void *v) {
    int a[3];
    int x = c ? n : ch;
    c ? h() : h();
    int *r = c ? p : p;
    r = c ? p : 0;
    r = c ? 0 : p;
    void *w = c ? p : v;
    r = c ? a : p;
    return x;
}
