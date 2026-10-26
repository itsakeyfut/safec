int f(void) {
    int x = 0;
    int *q = 0;
    int i = 0;
    while (i < 2) {
        int *m[1];
        if (i > 0) {
            q = m[0];
        }
        m[0] = &x;
        i = i + 1;
    }
    return q == 0;
}
