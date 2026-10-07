void h(void);

int f(int c, int *p, char *q) {
    int x;
    c ? h() : 1;
    x = c ? h() : 1;
    int *r = c ? p : q;
    c ? p : 1;
    return x;
}
