int f(int c) {
    int x = 0;
    int *m[1];
    if (c) {
        m[0] = &x;
    }
    int *q = m[0];
    return q == 0;
}
