void use2(int *a, int *b);
int f(int c, int d, int * _Nonnull q) {
    int *r;
    int *p;
    if (c) p = q;
    if (d) use2(r, p);
    return 0;
}
