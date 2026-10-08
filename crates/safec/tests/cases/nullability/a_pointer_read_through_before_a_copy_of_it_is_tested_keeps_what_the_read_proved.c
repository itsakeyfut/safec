int f(int *p) {
    int *q;
    if ((q = p, *p, q)) {
        return 1;
    }
    return *p;
}
