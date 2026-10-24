void two(int * _Nonnull a, int * _Nonnull b);
int f(int *q) {
    two(q, q);
    return 0;
}
