int f(int *p) {
    if (!p) {
        return 0;
    }
    *p = 1;
    return 0;
}
