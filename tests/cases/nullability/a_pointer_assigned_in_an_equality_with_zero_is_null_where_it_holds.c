int f(int *p) {
    int *q;
    if ((q = p) == 0) {
        return *p;
    }
    return *q;
}
