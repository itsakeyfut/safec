int g(void);
int f(void) {
    int *q;
    int *r;
    int x;
    int n;
    x = 1;
    n = 0;
    while (n < 3) {
        n = n + 1;
        if (g()) {
            q = &x;
            continue;
        }
        r = q;
    }
    return 0;
}
