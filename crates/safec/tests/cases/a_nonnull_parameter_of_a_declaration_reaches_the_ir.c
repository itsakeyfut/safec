void g(int * _Nonnull p);
int f(void) {
    int x = 0;
    g(&x);
    return x;
}
