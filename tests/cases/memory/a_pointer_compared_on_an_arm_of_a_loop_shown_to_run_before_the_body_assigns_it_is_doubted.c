int f(int * _Nonnull q) {
    int *p;
    int r = 0;
    for (int i = 0; i < 3; i++) {
        if (i >= 0)
            r = r + (p == q);
        p = q;
    }
    return r;
}
