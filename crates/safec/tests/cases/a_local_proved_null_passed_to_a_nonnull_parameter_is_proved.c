void g(int * _Nonnull p);
int f(void) {
    int *q = 0;
    g(q);
    return 0;
}
