void h(void);
int g(int a);

int f(int c, int *p, char *q, char ch, void *v) {
    int x;
    c ? h() : 1;
    x = c ? h() : 1;
    int *r = c ? p : q;
    c ? p : 1;
    c ? 1 : p;
    c ? p : h();
    c ? h() : p;
    c ? 1 : h();
    c ? p : ch;
    v = c ? g : v;
    v = c ? v : g;
    return x;
}
