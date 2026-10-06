int f(int (*fp)(int *)) {
    return fp(0);
}
