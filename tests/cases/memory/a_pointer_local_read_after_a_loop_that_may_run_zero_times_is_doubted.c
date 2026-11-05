int f(int n) {
    int x = 1;
    int *q;
    int i = 0;
    while (i < n) {
        q = &x;
        i = i + 1;
    }
    int *r = q;
    return r != 0;
}
