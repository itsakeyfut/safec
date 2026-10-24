void *malloc(int n);

int * _Nonnull f(void) {
    return malloc(4);
}
