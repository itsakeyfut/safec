void use(int *p);
int f(int c, int d, int * _Nonnull q) {
    int *p;
    if (c) p = q;
    use(p);
    if (d) return p == q;
    return 0;
}
