int g(void);
int f(void) {
    int *m[1];
    int x;
    int i;
    x = 1;
    for (i = 0; i < 2; i = i + 1) {
        if (g()) continue;
        m[0] = &x;
    }
    return *m[0];
}
