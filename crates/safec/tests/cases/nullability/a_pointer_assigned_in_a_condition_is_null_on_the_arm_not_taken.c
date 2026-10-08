int f(int *p) {
    int *q;
    if ((q = p)) {
        return 1;
    }
    return *p;
}
