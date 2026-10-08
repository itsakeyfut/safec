int f(void) {
    int x;
    int *p = &x;
    if (p == 0) {
        *p = 1;
    }
    return 0;
}
