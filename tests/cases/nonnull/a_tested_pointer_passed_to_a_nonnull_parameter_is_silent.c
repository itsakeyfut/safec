void g(int * _Nonnull p);
int f(int *q) {
    if (q) {
        g(q);
    }
    return 0;
}
