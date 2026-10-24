void abort(void);

int * _Nonnull f(int *p) {
    if (p) {
        return p;
    }
    abort();
}
