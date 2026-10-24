int f(int *p) {
    int *q;
    if ((q = p)) {
        return *q + *p;
    }
    return 0;
}
