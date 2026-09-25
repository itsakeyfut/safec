void g(void);
int f(int n) {
    int r;
    r = g() * 1;
    r = g() / 1;
    r = g() % 1;
    r = n + g();
    r = n - g();
    r = g() << 1;
    r = g() >> 1;
    r = g() < 1;
    r = g() > 1;
    r = g() <= 1;
    r = g() >= 1;
    r = g() == 1;
    r = g() != 1;
    r = g() & 1;
    r = g() ^ 1;
    r = g() | 1;
    r = g() && 1;
    r = g() || 1;
    return r;
}
