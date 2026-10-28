int f(int n) {
    int *p = 0;
    int r = 0;
    for (int i = 0; i < n; i++) {
        int x = i;
        if (p != 0) { r = *p; }
        p = &x;
    }
    return r;
}
