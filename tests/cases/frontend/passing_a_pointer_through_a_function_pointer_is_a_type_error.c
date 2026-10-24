int g(int a) {
    return a;
}

int f(int *p) {
    int (*fp)(int) = g;
    return fp(p);
}
