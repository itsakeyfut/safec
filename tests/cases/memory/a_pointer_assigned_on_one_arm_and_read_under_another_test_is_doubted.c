void use(int *p);
int f(int c, int d, int * _Nonnull q) {
    int *p;
    if (c) p = q;
    if (d) use(p);
    return 0;
}
