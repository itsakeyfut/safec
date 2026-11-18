void other(void);
int f(int ***out) {
    int *x;
    if (out == 0) return 0;
    *out = 0;
    *out = &x;
    other();
    return x != 0;
}
