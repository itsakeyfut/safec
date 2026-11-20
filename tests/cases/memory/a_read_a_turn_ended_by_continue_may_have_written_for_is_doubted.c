int g(void);
int f(void) {
    int *m[1];
    int x;
    int *r;
    int n;
    x = 1;
    n = 0;
    while (n < 3) {
        n = n + 1;
        if (g()) {
            m[0] = &x;
            continue;
        }
        r = m[0];
    }
    return 0;
}
