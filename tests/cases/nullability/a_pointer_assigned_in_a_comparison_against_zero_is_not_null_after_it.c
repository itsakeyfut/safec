int f(int *p) {
    int *q;
    if ((q = p) != 0) {
        return *q + *p;
    }
    return 0;
}
