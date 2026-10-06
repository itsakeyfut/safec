int f(int x, int *p, int (**pp)(int)) {
    return x(1) + p(1) + pp(1);
}
