int f(int n) {
    int x = 1;
    int *m[1];
    int i = 0;
    while (i < n) {
        m[0] = &x;
        i = i + 1;
    }
    int *q = m[0];
    return q != 0;
}
