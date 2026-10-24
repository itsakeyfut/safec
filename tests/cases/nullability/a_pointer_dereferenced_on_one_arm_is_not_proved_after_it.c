int f(int *p, int c) {
    if (c) {
        *p = 1;
    }
    *p = 2;
    return 0;
}
