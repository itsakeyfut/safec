int f(int *r) {
    int *q = r;
    if (q) {
        return *q;
    }
    return 0;
}
