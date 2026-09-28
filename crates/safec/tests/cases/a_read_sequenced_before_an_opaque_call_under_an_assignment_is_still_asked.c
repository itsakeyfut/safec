int g(void);

int f(int *p) {
    int x;
    if (p == 0) {
        return 0;
    }
    x = p[0] ? g() : 0;
    return x;
}
