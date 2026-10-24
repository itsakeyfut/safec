int f(int *p) {
    int *q;
    if ((q = p, q)) {
        return *p;
    }
    return 0;
}
