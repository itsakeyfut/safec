int f(int (*fp)(int)) {
    int *q = fp(1);
    return *q;
}
