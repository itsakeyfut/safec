int f(int n) {
    int a = 5;
    int r = 0;
    int *p = &a;
    for (int i = 0; i < n; i++) {
        int x = i;
        r += *p;
        int used = 0;
        if (n > 3) { p = &x; used = 1; }
        r += *p;
        if (used) { p = &a; }
    }
    return r;
}
