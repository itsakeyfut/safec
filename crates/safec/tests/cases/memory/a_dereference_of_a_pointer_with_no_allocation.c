int f(void) {
    int x = 1;
    int *p = &x;
    *p = 2;
    return x;
}
